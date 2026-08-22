mod config;
mod discord;
mod rss;
mod models;

use std::collections::HashSet;
use std::time::Duration;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = config::load()?;
    let client = reqwest::Client::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut interval = tokio::time::interval(Duration::from_secs(config.interval));

    loop {
        interval.tick().await;

        for feed in &config.feeds {
            let items = match rss::fetch_feed(feed).await {
                Ok(items) => items,
                Err(err) => {
                    eprintln!("Failed to fetch {feed}: {err:#}");
                    continue;
                }
            };

            for item in items {
                if !seen.insert(item.link.clone()) {
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
    }
}
