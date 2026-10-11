// State shared between the polling loop and the web interface.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::AtomicBool;
use std::sync::{Mutex, RwLock};

use serde::Serialize;
use tokio::sync::Notify;

use crate::auth::Auth;
use crate::config::Config;
use crate::security::Guard;
use crate::state::State;

pub struct AppState {
    pub config: RwLock<Config>,
    pub client: reqwest::Client,
    pub state: Mutex<State>,
    pub auth: Auth,
    pub guard: Guard,
    /// Result of the last fetch of each feed, shown in the web interface.
    pub status: Mutex<HashMap<String, FeedStatus>>,
    /// Feeds added from the web interface whose current items must be marked as seen
    /// on their first successful fetch instead of being flooded into Discord.
    pending_baseline: Mutex<HashSet<String>>,
    /// Wakes the polling loop early after the feed list changed.
    pub wake: Notify,
    /// Set by "refresh now": check every feed on the next wake, due or not.
    pub refresh_all: AtomicBool,
}

#[derive(Clone, Default, Serialize)]
pub struct FeedStatus {
    pub title: Option<String>,
    pub last_check: u64,
    pub error: Option<String>,
    pub items: usize,
}

impl AppState {
    pub fn new(config: Config, client: reqwest::Client, state: State) -> Self {
        let auth = Auth::new(&config.public_url);
        Self {
            config: RwLock::new(config),
            client,
            state: Mutex::new(state),
            auth,
            guard: Guard::default(),
            status: Mutex::new(HashMap::new()),
            pending_baseline: Mutex::new(HashSet::new()),
            wake: Notify::new(),
            refresh_all: AtomicBool::new(false),
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
