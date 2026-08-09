mod config;
mod discord;
mod rss;
mod models;

use std::time::Duration;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
 let config = config::load()?;
 let body = rss::fetch_feed(&config.feeds[0]).await?;
 println!("{}", body);
 Ok(())
}
