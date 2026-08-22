// Persists the set of already-sent item links to disk so restarts don't resend everything.

use std::collections::HashSet;
use std::fs;

use anyhow::Result;

const STATE_DIR: &str = "data";
const STATE_FILE: &str = "data/seen.json";

pub fn load() -> HashSet<String> {
    fs::read_to_string(STATE_FILE)
        .ok()
        .and_then(|content| serde_json::from_str(&content).ok())
        .unwrap_or_default()
}

pub fn save(seen: &HashSet<String>) -> Result<()> {
    fs::create_dir_all(STATE_DIR)?;
    let content = serde_json::to_string(seen)?;
    fs::write(STATE_FILE, content)?;
    Ok(())
}
