// Reads config.toml, deserializes to a config struct

use std::fs;
use anyhow::Context;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
 pub struct Config {
   pub interval: u64,
   pub webhook: String,
   pub feeds: Vec<String>,
 }

pub fn load()-> anyhow::Result<Config> {
    let content = fs::read_to_string("config.toml")
        .context("Failed to read config.toml")?;

    let config: Config = toml::from_str(&content)
        .context("Failed to parse config.toml")?;

      Ok (config)
}
