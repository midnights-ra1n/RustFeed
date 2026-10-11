// Configuration, stored in data/config.toml and managed from the web interface:
// nothing has to be edited by hand. The file is created with defaults on first
// start, so mounting the data/ volume is enough. An old ./config.toml (from
// versions that needed it) is migrated once if it is a real file.

use std::fs;
use std::path::Path;
use anyhow::Context;
use serde::{Deserialize, Deserializer, Serialize};
use toml_edit::DocumentMut;

const CONFIG_DIR: &str = "data";
const CONFIG_PATH: &str = "data/config.toml";
const LEGACY_CONFIG_PATH: &str = "config.toml";

const DEFAULT_CONFIG: &str = r#"# RustFeed configuration, managed from the web interface.

interval = 1800
webhook = ""
web_port = 3060
public_url = "http://localhost:3060"
behind_proxy = false
feeds = []
"#;

pub const DEFAULT_MAIL_SUBJECT: &str = "RustFeed · {{count}} nouvel(s) article(s)";

pub const DEFAULT_MAIL_TEMPLATE: &str = r#"<div style="background:#16110f;padding:24px;font-family:system-ui,sans-serif;color:#f3ece6">
  <div style="max-width:600px;margin:auto">
    <h1 style="margin:0 0 4px;font-size:22px;background:linear-gradient(90deg,#ff8a3d,#ff5f8f);-webkit-background-clip:text;color:#ff7a59">RustFeed</h1>
    <p style="margin:0 0 20px;color:#a3968c">{{count}} nouvel(s) article(s) · {{date}}</p>
    {{items}}
  </div>
</div>"#;

pub const DEFAULT_MAIL_ITEM: &str = r#"<div style="background:#221b18;border-radius:12px;padding:16px;margin-bottom:12px">
  <div style="font-size:12px;color:#ff8a65;text-transform:uppercase">{{feed}}</div>
  <a href="{{link}}" style="color:#f3ece6;font-size:17px;font-weight:600;text-decoration:none">{{title}}</a>
  <p style="margin:8px 0 0;color:#c9bcb2;font-size:14px">{{description}}</p>
</div>"#;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Default delay between two checks of a feed, in seconds.
    pub interval: u64,
    #[serde(default)]
    pub webhook: String,
    #[serde(default = "yes")]
    pub webhook_enabled: bool,
    #[serde(default = "default_web_port")]
    pub web_port: u16,
    #[serde(default = "default_public_url")]
    pub public_url: String,
    #[serde(default)]
    pub behind_proxy: bool,
    #[serde(default, deserialize_with = "feeds_compat")]
    pub feeds: Vec<Feed>,
    #[serde(default)]
    pub mail: MailConfig,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Feed {
    pub url: String,
    /// Overrides the global interval for this feed, in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interval: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MailConfig {
    pub enabled: bool,
    pub smtp_host: String,
    pub smtp_port: u16,
    /// "starttls", "tls" or "none"
    pub security: String,
    pub username: String,
    pub password: String,
    pub from: String,
    pub to: Vec<String>,
    /// Minimum delay between two digest emails, in seconds (0 = after every check).
    pub frequency: u64,
    pub subject: String,
    pub template: String,
    pub item_template: String,
}

impl Default for MailConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            smtp_host: String::new(),
            smtp_port: 587,
            security: "starttls".into(),
            username: String::new(),
            password: String::new(),
            from: String::new(),
            to: Vec::new(),
            frequency: 3600,
            subject: DEFAULT_MAIL_SUBJECT.into(),
            template: DEFAULT_MAIL_TEMPLATE.into(),
            item_template: DEFAULT_MAIL_ITEM.into(),
        }
    }
}

impl Config {
    pub fn webhook_active(&self) -> bool {
        self.webhook_enabled && self.webhook.starts_with("http")
    }
}

fn yes() -> bool {
    true
}

fn default_web_port() -> u16 {
    3060
}

fn default_public_url() -> String {
    "http://localhost:3060".into()
}

/// Accepts both the old `feeds = ["url", ...]` form and `[[feeds]]` tables.
fn feeds_compat<'de, D: Deserializer<'de>>(de: D) -> Result<Vec<Feed>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Entry {
        Url(String),
        Full(Feed),
    }
    Ok(Vec::<Entry>::deserialize(de)?
        .into_iter()
        .map(|e| match e {
            Entry::Url(url) => Feed { url, interval: None },
            Entry::Full(feed) => feed,
        })
        .collect())
}

pub fn load() -> anyhow::Result<Config> {
    fs::create_dir_all(CONFIG_DIR).context("Failed to create the data directory")?;
    if !Path::new(CONFIG_PATH).exists() {
        // Docker creates a *directory* when a missing file is bind-mounted: ignore that.
        if Path::new(LEGACY_CONFIG_PATH).is_file() {
            fs::copy(LEGACY_CONFIG_PATH, CONFIG_PATH).context("Failed to migrate ./config.toml")?;
            println!("Migrated ./config.toml to ./{CONFIG_PATH}. The old file is no longer used.");
        } else {
            fs::write(CONFIG_PATH, DEFAULT_CONFIG).context("Failed to create the default configuration")?;
            println!("Created a default configuration in ./{CONFIG_PATH}.");
        }
    }

    let content = fs::read_to_string(CONFIG_PATH)
        .context("Failed to read data/config.toml")?;

    let mut config: Config = toml::from_str(&content)
        .context("Failed to parse data/config.toml")?;

    // The web interface enforces this; only a hand-edited file can get here.
    config.interval = config.interval.max(60);

    Ok(config)
}

/// Rewrites data/config.toml from `config`, keeping comments on existing keys.
pub fn save(config: &Config) -> anyhow::Result<()> {
    let content = fs::read_to_string(CONFIG_PATH).unwrap_or_default();
    let mut doc: DocumentMut = content.parse()
        .context("Failed to parse data/config.toml")?;
    let fresh: DocumentMut = toml::to_string(config)
        .context("Failed to serialize config")?
        .parse()?;

    for (key, item) in fresh.iter() {
        // A value turning into a table (feeds = [] -> [[feeds]]) can't keep the old key's formatting.
        if doc.get(key).is_some_and(|old| old.is_value() != item.is_value()) {
            doc.remove(key);
        }
        doc[key] = item.clone();
    }

    fs::write(CONFIG_PATH, doc.to_string())
        .context("Failed to write data/config.toml")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_and_new_feed_formats_parse() {
        let old: Config = toml::from_str(r#"interval = 60
webhook = "x"
feeds = ["https://a"]"#).unwrap();
        assert_eq!(old.feeds, vec![Feed { url: "https://a".into(), interval: None }]);
        assert_eq!(old.mail.smtp_port, 587);

        let text = toml::to_string(&old).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(back.feeds, old.feeds);
    }
}
