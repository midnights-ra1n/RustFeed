// State shared between the polling loop and the web interface.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::AtomicBool;
use std::sync::{Mutex, RwLock};

use serde::Serialize;
use tokio::sync::Notify;

use crate::auth::Auth;
use crate::config::Config;
use crate::security::Guard;
use crate::models::plain_text;
use crate::state::{State, StoredItem};

/// Full articles kept in memory for the reader and the MCP endpoint.
const MAX_CACHED_ARTICLES: usize = 100;

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
    /// link -> full article HTML extracted from the original page.
    articles: Mutex<HashMap<String, String>>,
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
            articles: Mutex::new(HashMap::new()),
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

    /// The most complete version of an article: the original page's main content
    /// when it is longer than what the feed gave, otherwise the feed's content.
    pub async fn article_html(&self, item: &StoredItem) -> String {
        let feed_html = item.content.clone().unwrap_or_else(|| item.summary.clone());
        if let Some(html) = self.articles.lock().unwrap().get(&item.link) {
            return html.clone();
        }
        if !item.link.starts_with("http") {
            return feed_html;
        }
        let html = match crate::rss::fetch_article(&self.client, &item.link).await {
            Ok(full) if plain_text(&full, usize::MAX).len() > plain_text(&feed_html, usize::MAX).len() => full,
            Ok(_) => feed_html,
            Err(err) => {
                eprintln!("Failed to extract {}: {err:#}", item.link);
                return feed_html; // not cached: retried next time
            }
        };
        let mut cache = self.articles.lock().unwrap();
        // shortcut: wipes the whole cache when full, use an LRU if refetches become noticeable
        if cache.len() >= MAX_CACHED_ARTICLES {
            cache.clear();
        }
        cache.insert(item.link.clone(), html.clone());
        html
    }
}
