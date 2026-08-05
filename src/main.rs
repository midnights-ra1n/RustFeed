mod config;
mod discord;
mod rss;
mod models;

use std::time::Duration;

#[tokio::main]
async fn main() {
 let config = config::load()?;
 
}
