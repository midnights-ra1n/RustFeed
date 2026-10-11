// Persists the set of already-sent item links to disk so restarts don't resend everything.
// The whole history is wiped every RETENTION_SECS to keep the file from growing forever;
// the flush cycle re-baselines silently (marks the current feed items as seen without
// sending them) instead of treating them all as new.
// The same file also keeps the latest articles for the web reader and the mail digest
// queue, so neither is lost on restart.

use std::collections::HashSet;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use serde::{Deserialize, Serialize};

const STATE_DIR: &str = "data";
const STATE_FILE: &str = "data/seen.json";
const RETENTION_SECS: u64 = 30 * 24 * 60 * 60;
/// Articles kept for the web reader.
const MAX_ITEMS: usize = 400;
/// Pending digest entries kept if mail keeps failing.
const MAX_QUEUE: usize = 500;

#[derive(Serialize, Deserialize)]
pub struct State {
    pub last_flush: u64,
    pub seen: HashSet<String>,
    /// Newest first.
    #[serde(default)]
    pub items: Vec<StoredItem>,
    #[serde(default)]
    pub read: HashSet<String>,
    #[serde(default)]
    pub mail_queue: Vec<StoredItem>,
    #[serde(default)]
    pub last_mail: u64,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct StoredItem {
    pub feed: String,
    pub feed_title: String,
    pub title: String,
    pub link: String,
    pub summary: String,
    #[serde(default)]
    pub content: Option<String>,
    pub published: Option<String>,
    pub fetched_at: u64,
}

pub fn now() -> u64 {
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
            items: Vec::new(),
            read: HashSet::new(),
            mail_queue: Vec::new(),
            last_mail: 0,
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

impl State {
    /// Adds an article to the reader (if not there yet) and trims the oldest ones.
    pub fn remember(&mut self, item: StoredItem) {
        if self.items.iter().any(|i| i.link == item.link) {
            return;
        }
        self.items.insert(0, item);
        if self.items.len() > MAX_ITEMS {
            self.items.truncate(MAX_ITEMS);
            let links: HashSet<&String> = self.items.iter().map(|i| &i.link).collect();
            self.read.retain(|l| links.contains(l));
        }
    }

    pub fn queue_mail(&mut self, item: StoredItem) {
        self.mail_queue.push(item);
        if self.mail_queue.len() > MAX_QUEUE {
            let extra = self.mail_queue.len() - MAX_QUEUE;
            self.mail_queue.drain(..extra);
        }
    }

    pub fn forget_feed(&mut self, feed: &str) {
        self.items.retain(|i| i.feed != feed);
        self.mail_queue.retain(|i| i.feed != feed);
    }
}
