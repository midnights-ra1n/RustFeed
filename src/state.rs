// Persists the set of already-sent item links to disk so restarts don't resend everything.
// The whole history is wiped every RETENTION_SECS to keep the file from growing forever;
// the flush cycle re-baselines silently (marks the current feed items as seen without
// sending them) instead of treating them all as new.

use std::collections::HashSet;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use serde::{Deserialize, Serialize};

const STATE_DIR: &str = "data";
const STATE_FILE: &str = "data/seen.json";
const RETENTION_SECS: u64 = 30 * 24 * 60 * 60;

#[derive(Serialize, Deserialize)]
pub struct State {
    pub last_flush: u64,
    pub seen: HashSet<String>,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

pub fn load() -> State {
    fs::read_to_string(STATE_FILE)
        .ok()
        .and_then(|content| serde_json::from_str(&content).ok())
        .unwrap_or_else(|| State {
            last_flush: now(),
            seen: HashSet::new(),
        })
}

pub fn save(state: &State) -> Result<()> {
    fs::create_dir_all(STATE_DIR)?;
    let content = serde_json::to_string(state)?;
    fs::write(STATE_FILE, content)?;
    Ok(())
}

/// If the retention period has elapsed, wipes the seen history and resets the
/// flush timer. Returns true when a flush just happened, so the caller can
/// re-baseline the current feed items as seen without notifying for them.
pub fn maybe_flush(state: &mut State) -> bool {
    if now().saturating_sub(state.last_flush) < RETENTION_SECS {
        return false;
    }

    state.seen.clear();
    state.last_flush = now();
    true
}
