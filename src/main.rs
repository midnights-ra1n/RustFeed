mod config;
mod discord;
mod rss;
mod models;
mod state;

use std::time::Duration;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = config::load()?;
    let client = reqwest::Client::new();
    let mut state = state::load();
    let mut interval = tokio::time::interval(Duration::from_secs(config.interval));

    loop {
        interval.tick().await;

        let just_flushed = state::maybe_flush(&mut state);
        if just_flushed {
            eprintln!("30-day retention reached: seen history reset, re-baselining without sending notifications.");
        }

        for feed in &config.feeds {
            let items = match rss::fetch_feed(feed).await {
                Ok(items) => items,
                Err(err) => {
                    eprintln!("Failed to fetch {feed}: {err:#}");
                    continue;
                }
            };

            for item in items {
                if !state.seen.insert(item.link.clone()) {
                    continue;
                }

                if just_flushed {
                    continue;
                }

                println!("Titre : {}", item.title);
                println!("Lien  : {}", item.link);
                println!();

                if let Err(err) = discord::send_embed(&client, &config.webhook, &item).await {
                    eprintln!("Failed to send embed for {}: {err:#}", item.link);
                }
            }
        }

        if let Err(err) = state::save(&state) {
            eprintln!("Failed to save seen state: {err:#}");
        }
    }
}
