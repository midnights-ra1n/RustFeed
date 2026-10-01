// State shared between the polling loop and the web interface.

use std::collections::HashSet;
use std::sync::{Mutex, RwLock};

use tokio::sync::Notify;

use crate::config::Config;

pub struct AppState {
    pub config: RwLock<Config>,
    pub client: reqwest::Client,
    /// Feeds added from the web interface whose current items must be marked as seen
    /// on their first successful fetch instead of being flooded into Discord.
    pending_baseline: Mutex<HashSet<String>>,
    /// Wakes the polling loop early after the feed list changed.
    pub wake: Notify,
}

impl AppState {
    pub fn new(config: Config, client: reqwest::Client) -> Self {
        Self {
            config: RwLock::new(config),
            client,
            pending_baseline: Mutex::new(HashSet::new()),
            wake: Notify::new(),
        }
    }

    pub fn mark_for_baseline(&self, feed: &str) {
        self.pending_baseline.lock().unwrap().insert(feed.to_string());
    }

    pub fn forget_baseline(&self, feed: &str) {
        self.pending_baseline.lock().unwrap().remove(feed);
    }

    /// Returns true (once) if the feed was waiting for its silent baseline.
    pub fn take_baseline(&self, feed: &str) -> bool {
        self.pending_baseline.lock().unwrap().remove(feed)
    }
}
