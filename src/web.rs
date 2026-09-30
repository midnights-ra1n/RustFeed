// Web interface to manage the RSS feeds (list / add / edit / delete).
// The static files in web/ are embedded into the binary at compile time,
// so the executable (and the Docker image) stays self-contained.

use std::sync::Arc;

use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::app::AppState;
use crate::{config, rss};

const INDEX_HTML: &str = include_str!("../web/index.html");
const STYLE_CSS: &str = include_str!("../web/style.css");
const APP_JS: &str = include_str!("../web/app.js");
const FAVICON_SVG: &str = include_str!("../web/favicon.svg");

type SharedState = Arc<AppState>;

pub async fn serve(listener: tokio::net::TcpListener, app: SharedState) -> anyhow::Result<()> {
    let router = Router::new()
        .route("/", get(|| async { asset("text/html; charset=utf-8", INDEX_HTML) }))
        .route("/style.css", get(|| async { asset("text/css; charset=utf-8", STYLE_CSS) }))
        .route("/app.js", get(|| async { asset("text/javascript; charset=utf-8", APP_JS) }))
        .route("/favicon.svg", get(|| async { asset("image/svg+xml", FAVICON_SVG) }))
        .route("/api/feeds", get(list_feeds).post(add_feed).put(edit_feed).delete(delete_feed))
        .route("/api/check", post(check_feed))
        .with_state(app);

    axum::serve(listener, router).await?;
    Ok(())
}

fn asset(content_type: &'static str, body: &'static str) -> Response {
    ([(header::CONTENT_TYPE, content_type)], body).into_response()
}

struct ApiError(StatusCode, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}

fn bad_request(msg: impl Into<String>) -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, msg.into())
}

#[derive(Serialize)]
struct FeedList {
    feeds: Vec<String>,
    interval: u64,
}

#[derive(Deserialize)]
struct FeedBody {
    url: String,
}

#[derive(Deserialize)]
struct EditBody {
    old_url: String,
    url: String,
}

fn validate_url(raw: &str) -> Result<String, ApiError> {
    let url = raw.trim();
    let parsed = reqwest::Url::parse(url).map_err(|_| bad_request("URL invalide."))?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err(bad_request("L'URL doit commencer par http:// ou https://."));
    }
    Ok(url.to_string())
}

/// Applies `change` to a copy of the feed list, persists it to config.toml, and only
/// then swaps it in memory, so the running config never diverges from the file.
fn update_feeds(
    app: &AppState,
    change: impl FnOnce(&mut Vec<String>) -> Result<(), ApiError>,
) -> Result<FeedList, ApiError> {
    let mut config = app.config.write().unwrap();
    let mut feeds = config.feeds.clone();
    change(&mut feeds)?;

    config::save_feeds(&feeds).map_err(|err| {
        eprintln!("Failed to save feeds: {err:#}");
        ApiError(StatusCode::INTERNAL_SERVER_ERROR, format!("Impossible d'écrire config.toml : {err:#}"))
    })?;
    config.feeds = feeds.clone();

    Ok(FeedList { feeds, interval: config.interval })
}

async fn list_feeds(State(app): State<SharedState>) -> Json<FeedList> {
    let config = app.config.read().unwrap();
    Json(FeedList { feeds: config.feeds.clone(), interval: config.interval })
}

async fn add_feed(
    State(app): State<SharedState>,
    Json(body): Json<FeedBody>,
) -> Result<(StatusCode, Json<FeedList>), ApiError> {
    let url = validate_url(&body.url)?;
    let list = update_feeds(&app, |feeds| {
        if feeds.contains(&url) {
            return Err(ApiError(StatusCode::CONFLICT, "Ce flux existe déjà.".into()));
        }
        feeds.push(url.clone());
        Ok(())
    })?;

    app.mark_for_baseline(&url);
    app.wake.notify_one();
    println!("Feed added from web interface: {url}");
    Ok((StatusCode::CREATED, Json(list)))
}

async fn edit_feed(
    State(app): State<SharedState>,
    Json(body): Json<EditBody>,
) -> Result<Json<FeedList>, ApiError> {
    let url = validate_url(&body.url)?;
    let list = update_feeds(&app, |feeds| {
        let index = feeds
            .iter()
            .position(|f| *f == body.old_url)
            .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "Flux introuvable.".into()))?;
        if url != body.old_url && feeds.contains(&url) {
            return Err(ApiError(StatusCode::CONFLICT, "Ce flux existe déjà.".into()));
        }
        feeds[index] = url.clone();
        Ok(())
    })?;

    if url != body.old_url {
        app.forget_baseline(&body.old_url);
        app.mark_for_baseline(&url);
        app.wake.notify_one();
        println!("Feed edited from web interface: {} -> {url}", body.old_url);
    }
    Ok(Json(list))
}

async fn delete_feed(
    State(app): State<SharedState>,
    Json(body): Json<FeedBody>,
) -> Result<Json<FeedList>, ApiError> {
    let list = update_feeds(&app, |feeds| {
        let index = feeds
            .iter()
            .position(|f| *f == body.url)
            .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "Flux introuvable.".into()))?;
        feeds.remove(index);
        Ok(())
    })?;

    app.forget_baseline(&body.url);
    println!("Feed removed from web interface: {}", body.url);
    Ok(Json(list))
}

/// Downloads and parses a feed without saving anything, so the UI can show
/// its title and tell whether it is actually a valid RSS feed.
async fn check_feed(
    State(app): State<SharedState>,
    Json(body): Json<FeedBody>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let url = validate_url(&body.url)?;
    let channel = rss::fetch_channel(&app.client, &url)
        .await
        .map_err(|err| ApiError(StatusCode::UNPROCESSABLE_ENTITY, format!("{err:#}")))?;

    Ok(Json(json!({
        "title": channel.title(),
        "link": channel.link(),
        "items": channel.items().len(),
    })))
}
