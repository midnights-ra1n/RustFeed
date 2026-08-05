// Reads config.toml, deserializes to a config struct

pub struct Config {
  pub interval: u64,
  pub webhook: String,
  pub feeds: Vec<String>,
}
