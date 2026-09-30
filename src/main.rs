mod app;
mod config;
mod discord;
mod rss;
mod models;
mod state;
mod web;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;

use crate::app::AppState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = config::load()?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?;

    let addr = SocketAddr::from(([0, 0, 0, 0], config.web_port));
    let app = Arc::new(AppState::new(config, client));

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

    poll_loop(app).await;
    Ok(())
}

async fn poll_loop(app: Arc<AppState>) {
    let mut state = state::load();
    let interval_secs = app.config.read().unwrap().interval;
    let mut interval = tokio::time::interval(Duration::from_secs(interval_secs));

    loop {
        tokio::select! {
            _ = interval.tick() => {}
            _ = app.wake.notified() => {}
        }

        let just_flushed = state::maybe_flush(&mut state);
        if just_flushed {
            eprintln!("30-day retention reached: seen history reset, re-baselining without sending notifications.");
        }

        let (feeds, webhook) = {
            let config = app.config.read().unwrap();
            (config.feeds.clone(), config.webhook.clone())
        };

        for feed in &feeds {
            let items = match rss::fetch_feed(&app.client, feed).await {
                Ok(items) => items,
                Err(err) => {
                    eprintln!("Failed to fetch {feed}: {err:#}");
                    continue;
                }
            };

            let baseline = app.take_baseline(feed);
            if baseline {
                println!("New feed {feed}: marking its {} current items as seen.", items.len());
            }

            for item in items {
                if !state.seen.insert(item.link.clone()) {
                    continue;
                }

                if just_flushed || baseline {
                    continue;
                }

                println!("Titre : {}", item.title);
                println!("Lien  : {}", item.link);
                println!();

                if let Err(err) = discord::send_embed(&app.client, &webhook, &item).await {
                    eprintln!("Failed to send embed for {}: {err:#}", item.link);
                }
            }
        }

        if let Err(err) = state::save(&state) {
            eprintln!("Failed to save seen state: {err:#}");
        }
    }
}
