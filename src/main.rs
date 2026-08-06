mod config;
mod discord;
mod rss;
mod models;

use std::time::Duration;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
 let config = config::load()?;
 println!("{:#?}", config);
 Ok(())
}
