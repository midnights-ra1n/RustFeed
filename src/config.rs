// Reads config.toml, deserializes to a config struct.
// If the file is missing, a default one is created automatically so a container
// started without an interactive terminal (docker run / docker compose) still
// ends up with an editable config.toml on its mounted volume.

use std::fs;
use std::path::Path;
use anyhow::{bail, Context};
use serde::Deserialize;
use toml_edit::{Array, DocumentMut, Value};

const CONFIG_PATH: &str = "config.toml";
const DEFAULT_WEB_PORT: u16 = 3060;

const DEFAULT_CONFIG: &str = r#"# RustFeed configuration file
# Interval is in seconds

interval = 1800

webhook = "PUT YOUR WEBHOOK URL HERE"

# Port of the web interface used to manage feeds
web_port = 3060

feeds = [
  "https://www.clubic.com/feed/rss"
]
"#;

#[derive(Debug, Deserialize)]
pub struct Config {
    pub interval: u64,
    pub webhook: String,
    #[serde(default = "default_web_port")]
    pub web_port: u16,
    #[serde(default)]
    pub feeds: Vec<String>,
}

fn default_web_port() -> u16 {
    DEFAULT_WEB_PORT
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
        eprintln!(
            "config.toml: `feeds` is empty. Add RSS feeds from the web interface on port {}.",
            config.web_port
        );
    }

    Ok(config)
}

/// Rewrites only the `feeds` array of config.toml, keeping the user's comments
/// and formatting everywhere else. The file is written in place (not renamed over)
/// because it is usually bind-mounted as a single file in Docker.
pub fn save_feeds(feeds: &[String]) -> anyhow::Result<()> {
    let content = fs::read_to_string(CONFIG_PATH)
        .context("Failed to read config.toml")?;
    let mut doc: DocumentMut = content.parse()
        .context("Failed to parse config.toml")?;

    let mut array = Array::new();
    for feed in feeds {
        let mut value = Value::from(feed.as_str());
        value.decor_mut().set_prefix("\n  ");
        array.push_formatted(value);
    }
    array.set_trailing(if feeds.is_empty() { "" } else { "\n" });
    array.set_trailing_comma(false);

    doc["feeds"] = toml_edit::value(array);

    fs::write(CONFIG_PATH, doc.to_string())
        .context("Failed to write config.toml")?;
    Ok(())
}
