// Reads config.toml, deserializes to a config struct.
// If the file is missing, a default one is created automatically so a container
// started without an interactive terminal (docker run / docker compose) still
// ends up with an editable config.toml on its mounted volume.

use std::fs;
use std::path::Path;
use anyhow::{bail, Context};
use serde::Deserialize;

const CONFIG_PATH: &str = "config.toml";

const DEFAULT_CONFIG: &str = r#"# RustFeed configuration file
# Interval is in seconds

interval = 1800

webhook = "PUT YOUR WEBHOOK URL HERE"

feeds = [
  "https://www.clubic.com/feed/rss"
]
"#;

#[derive(Debug, Deserialize)]
pub struct Config {
    pub interval: u64,
    pub webhook: String,
    pub feeds: Vec<String>,
}

pub fn load() -> anyhow::Result<Config> {
    if !Path::new(CONFIG_PATH).exists() {
        fs::write(CONFIG_PATH, DEFAULT_CONFIG)
            .context("Failed to create default config.toml")?;
        bail!(
            "No config.toml found: a default one was just created at ./{CONFIG_PATH}. \
             Edit it with your Discord webhook URL and RSS feeds, then restart the container."
        );
    }

    let content = fs::read_to_string(CONFIG_PATH)
        .context("Failed to read config.toml")?;

    let config: Config = toml::from_str(&content)
        .context("Failed to parse config.toml")?;

    if !config.webhook.starts_with("https://discord.com/api/webhooks/")
        && !config.webhook.starts_with("https://discordapp.com/api/webhooks/")
    {
        bail!(
            "config.toml: `webhook` is not a valid Discord webhook URL. \
             It should look like https://discord.com/api/webhooks/<id>/<token>. Fix it and restart."
        );
    }

    if config.feeds.is_empty() {
        bail!("config.toml: `feeds` is empty. Add at least one RSS feed URL and restart.");
    }

    Ok(config)
}
