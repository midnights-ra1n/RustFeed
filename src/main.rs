mod config;
mod discord;
mod rss;
mod models;

use std::time::Duration;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = config::load()?;
    let items = rss::fetch_feed(&config.feeds[0]).await?;
    for item in items {
        println!("Titre : {}", item.title);
        println!("Lien  : {}", item.link);
        println!();
    }
    Ok(())
}
