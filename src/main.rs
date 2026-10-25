mod app;
mod auth;
mod config;
mod discord;
mod mail;
mod mcp;
mod rss;
mod models;
mod security;
mod state;
mod update;
mod web;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;

use crate::app::{AppState, FeedStatus};
use crate::state::StoredItem;

/// How often the loop looks for feeds that are due.
const TICK_SECS: u64 = 30;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = config::load()?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent(concat!("RustFeed/", env!("CARGO_PKG_VERSION")))
        .build()?;

    let addr = SocketAddr::from(([0, 0, 0, 0], config.web_port));
    let app = Arc::new(AppState::new(config, client, state::load()));

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("Failed to bind the web interface on {addr}"))?;
    println!("Web interface listening on http://{addr}");

    let web_app = app.clone();
    tokio::spawn(async move {
        if let Err(err) = web::serve(listener, web_app).await {
            eprintln!("Web interface stopped: {err:#}");
        }
    });

    tokio::spawn(update::auto_loop(app.clone()));
    poll_loop(app).await;
    Ok(())
}

async fn poll_loop(app: Arc<AppState>) {
    // feed url -> unix time of its last check
    let mut last_check: HashMap<String, u64> = HashMap::new();

    loop {
        let just_flushed = state::maybe_flush(&mut app.state.lock().unwrap());
        if just_flushed {
            eprintln!("30-day retention reached: seen history reset, re-baselining without sending notifications.");
        }

        let config = app.config.read().unwrap().clone();
        let force = app.refresh_all.swap(false, Ordering::Relaxed);
        let now = state::now();

        for feed in &config.feeds {
            let every = feed.interval.unwrap_or(config.interval);
            let due = last_check.get(&feed.url).is_none_or(|t| now.saturating_sub(*t) >= every);
            if !(due || force || just_flushed) {
                continue;
            }
            last_check.insert(feed.url.clone(), now);
            check_feed(&app, &config, &feed.url, just_flushed).await;
        }
        last_check.retain(|url, _| config.feeds.iter().any(|f| &f.url == url));

        if config.mail.enabled {
            send_digest(&app, &config).await;
        }

        if let Err(err) = state::save(&app.state.lock().unwrap()) {
            eprintln!("Failed to save seen state: {err:#}");
        }

        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(TICK_SECS)) => {}
            _ = app.wake.notified() => {}
        }
    }
}

async fn check_feed(app: &AppState, config: &config::Config, url: &str, silent: bool) {
    let result = rss::fetch_feed(&app.client, url).await;
    let (title, items) = match result {
        Ok(ok) => ok,
        Err(err) => {
            eprintln!("Failed to fetch {url}: {err:#}");
            let mut status = app.status.lock().unwrap();
            let entry = status.entry(url.to_string()).or_default();
            entry.last_check = state::now();
            entry.error = Some(format!("{err:#}"));
            return;
        }
    };

    app.status.lock().unwrap().insert(url.to_string(), FeedStatus {
        title: Some(title.clone()),
        last_check: state::now(),
        error: None,
        items: items.len(),
    });

    let baseline = app.take_baseline(url);
    if baseline {
        println!("New feed {url}: marking its {} current items as seen.", items.len());
    }

    let mut to_send = Vec::new();
    {
        let mut state = app.state.lock().unwrap();
        // Oldest first so the reader and notifications keep the feed's order.
        for item in items.into_iter().rev() {
            if item.link.is_empty() {
                continue;
            }
            let stored = StoredItem {
                feed: url.to_string(),
                feed_title: if title.is_empty() { url.to_string() } else { title.clone() },
                summary: models::plain_text(item.description.as_deref().or(item.content.as_deref()).unwrap_or(""), 300),
                title: item.title,
                link: item.link,
                content: item.content.or(item.description),
                published: item.published,
                fetched_at: state::now(),
            };
            state.remember(stored.clone());

            if !state.seen.insert(stored.link.clone()) || silent || baseline {
                continue;
            }

            println!("Titre : {}", stored.title);
            println!("Lien  : {}", stored.link);
            println!();

            if config.mail.enabled {
                state.queue_mail(stored.clone());
            }
            to_send.push(stored);
        }
    }

    if config.webhook_active() {
        for item in &to_send {
            if let Err(err) = discord::send_embed(&app.client, &config.webhook, item).await {
                eprintln!("Failed to send webhook for {}: {err:#}", item.link);
            }
        }
    }
}

/// Sends the queued articles as one email once the configured frequency has elapsed,
/// outside quiet hours. Excluded feeds, folders and tags are checked at send time so
/// a blacklist change also applies to what is already queued.
async fn send_digest(app: &AppState, config: &config::Config) {
    let mail_cfg = &config.mail;
    let batch = {
        let mut state = app.state.lock().unwrap();
        state.mail_queue.retain(|i| !config.mail_excluded(&i.feed));
        if state.mail_queue.is_empty()
            || state::now().saturating_sub(state.last_mail) < mail_cfg.frequency
            || mail_cfg.is_quiet(jiff::Timestamp::now())
        {
            return;
        }
        state.mail_queue.clone()
    };

    match mail::send(mail_cfg, &batch).await {
        Ok(()) => {
            println!("Digest email sent with {} article(s).", batch.len());
            let mut state = app.state.lock().unwrap();
            // Only drop what was sent: items may have been queued meanwhile.
            let sent: std::collections::HashSet<&String> = batch.iter().map(|i| &i.link).collect();
            state.mail_queue.retain(|i| !sent.contains(&i.link));
            state.last_mail = state::now();
        }
        Err(err) => {
            eprintln!("Failed to send digest email: {err:#}");
            // Avoid hammering the SMTP server every tick after a failure.
            app.state.lock().unwrap().last_mail = state::now();
        }
    }
}
