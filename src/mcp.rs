// MCP server (Streamable HTTP, stateless, JSON responses) so an LLM client such as
// Claude or ChatGPT can read the user's articles: daily recap, article summaries.
// Disabled by default and only served when the public address is HTTPS (reverse proxy).
// The secret token is part of the URL because most remote MCP clients can't send
// custom headers without OAuth.

use std::net::IpAddr;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde_json::{json, Value};

use crate::app::AppState;
use crate::models::plain_text;
use crate::state::now;

const PROTOCOL_VERSIONS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];
const MAX_ARTICLE_CHARS: usize = 60_000;

pub async fn not_allowed() -> StatusCode {
    StatusCode::METHOD_NOT_ALLOWED
}

pub async fn handle(
    State(app): State<Arc<AppState>>,
    Extension(ip): Extension<IpAddr>,
    Path(token): Path<String>,
    headers: HeaderMap,
    Json(req): Json<Value>,
) -> Response {
    let (active, expected, public_url) = {
        let c = app.config.read().unwrap();
        (c.mcp_active(), c.mcp.token.clone(), c.public_url.clone())
    };
    if !active {
        return StatusCode::NOT_FOUND.into_response();
    }
    if !crate::auth::same(&token, &expected) {
        eprintln!("MCP: bad token from {ip}");
        app.guard.strike(ip);
        return StatusCode::NOT_FOUND.into_response();
    }
    // DNS rebinding protection required by the MCP spec: browsers send Origin, server-side clients don't.
    if let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
        if origin.trim_end_matches('/') != public_url.trim_end_matches('/') {
            return StatusCode::FORBIDDEN.into_response();
        }
    }

    let Some(method) = req.get("method").and_then(Value::as_str) else {
        return Json(rpc_error(Value::Null, -32600, "Invalid request")).into_response();
    };
    // Notifications (no id) get no response body.
    let Some(id) = req.get("id").cloned() else {
        return StatusCode::ACCEPTED.into_response();
    };
    let params = req.get("params").cloned().unwrap_or(Value::Null);

    let result = match method {
        "initialize" => Ok(initialize(&params)),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools() })),
        "tools/call" => Ok(call_tool(&app, &params).await),
        _ => Err((-32601, "Method not found")),
    };
    Json(match result {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err((code, msg)) => rpc_error(id, code, msg),
    })
    .into_response()
}

fn rpc_error(id: Value, code: i32, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn initialize(params: &Value) -> Value {
    let asked = params.get("protocolVersion").and_then(Value::as_str).unwrap_or("");
    let version = PROTOCOL_VERSIONS.iter().find(|v| **v == asked).unwrap_or(&PROTOCOL_VERSIONS[1]);
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": {} },
        "serverInfo": { "name": "RustFeed", "version": env!("CARGO_PKG_VERSION") },
        "instructions": "Articles from the user's RSS feeds. For a recap of the day, call list_articles (hours=24) \
            then get_article for the stories worth detailing. To summarize one article, call get_article with its link.",
    })
}

fn tools() -> Value {
    json!([
        {
            "name": "list_feeds",
            "description": "List the followed RSS feeds with their folder and tags.",
            "inputSchema": { "type": "object", "properties": {} },
            "annotations": { "readOnlyHint": true },
        },
        {
            "name": "list_articles",
            "description": "List recent articles (newest first) with title, feed, folder, tags, link, date and a short summary.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "hours": { "type": "number", "description": "Only articles fetched in the last N hours (default 24, max 720)." },
                    "folder": { "type": "string", "description": "Only this folder." },
                    "tag": { "type": "string", "description": "Only feeds with this tag." },
                    "feed": { "type": "string", "description": "Only this feed (URL or part of its title)." },
                    "unread_only": { "type": "boolean", "description": "Skip articles already read in RustFeed." },
                    "limit": { "type": "number", "description": "Maximum number of articles (default 100, max 400)." },
                },
            },
            "annotations": { "readOnlyHint": true },
        },
        {
            "name": "get_article",
            "description": "Full text of an article, extracted from the original page when the feed only has an excerpt.",
            "inputSchema": {
                "type": "object",
                "properties": { "link": { "type": "string", "description": "The article link from list_articles." } },
                "required": ["link"],
            },
            "annotations": { "readOnlyHint": true },
        },
    ])
}

async fn call_tool(app: &AppState, params: &Value) -> Value {
    let args = params.get("arguments").cloned().unwrap_or(json!({}));
    let text = match params.get("name").and_then(Value::as_str).unwrap_or("") {
        "list_feeds" => Ok(list_feeds(app)),
        "list_articles" => Ok(list_articles(app, &args)),
        "get_article" => get_article(app, &args).await,
        other => Err(format!("Unknown tool: {other}")),
    };
    match text {
        Ok(text) => json!({ "content": [{ "type": "text", "text": text }] }),
        Err(msg) => json!({ "content": [{ "type": "text", "text": msg }], "isError": true }),
    }
}

fn list_feeds(app: &AppState) -> String {
    let config = app.config.read().unwrap();
    let status = app.status.lock().unwrap();
    let feeds: Vec<Value> = config.feeds.iter()
        .map(|f| json!({
            "url": f.url,
            "title": status.get(&f.url).and_then(|s| s.title.clone()),
            "folder": f.folder,
            "tags": f.tags,
        }))
        .collect();
    serde_json::to_string_pretty(&feeds).unwrap_or_default()
}

fn list_articles(app: &AppState, args: &Value) -> String {
    let hours = args.get("hours").and_then(Value::as_f64).unwrap_or(24.0).clamp(0.0, 720.0);
    let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(100).min(400) as usize;
    let folder = args.get("folder").and_then(Value::as_str).filter(|s| !s.is_empty());
    let tag = args.get("tag").and_then(Value::as_str).filter(|s| !s.is_empty());
    let feed = args.get("feed").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_lowercase);
    let unread_only = args.get("unread_only").and_then(Value::as_bool).unwrap_or(false);
    let since = now().saturating_sub((hours * 3600.0) as u64);

    let config = app.config.read().unwrap();
    let state = app.state.lock().unwrap();
    let articles: Vec<Value> = state.items.iter()
        .filter(|i| i.fetched_at >= since && !(unread_only && state.read.contains(&i.link)))
        .filter_map(|i| {
            let f = config.feeds.iter().find(|f| f.url == i.feed);
            let (item_folder, item_tags) = f.map(|f| (f.folder.as_str(), f.tags.as_slice())).unwrap_or_default();
            let keep = folder.is_none_or(|x| x.eq_ignore_ascii_case(item_folder))
                && tag.is_none_or(|x| item_tags.iter().any(|t| t.eq_ignore_ascii_case(x)))
                && feed.as_ref().is_none_or(|x| i.feed.to_lowercase() == *x || i.feed_title.to_lowercase().contains(x.as_str()));
            keep.then(|| json!({
                "title": i.title,
                "feed": i.feed_title,
                "folder": item_folder,
                "tags": item_tags,
                "link": i.link,
                "published": i.published,
                "summary": i.summary,
                "read": state.read.contains(&i.link),
            }))
        })
        .take(limit)
        .collect();
    if articles.is_empty() {
        return "No article matches.".into();
    }
    serde_json::to_string_pretty(&articles).unwrap_or_default()
}

async fn get_article(app: &AppState, args: &Value) -> Result<String, String> {
    let link = args.get("link").and_then(Value::as_str).unwrap_or("");
    // Only links RustFeed already knows: the endpoint can't be used to fetch arbitrary URLs.
    let item = app.state.lock().unwrap().items.iter().find(|i| i.link == link).cloned()
        .ok_or("Unknown article: use a link returned by list_articles.")?;
    let text = plain_text(&app.article_html(&item).await, MAX_ARTICLE_CHARS);
    Ok(format!("Title: {}\nFeed: {}\nPublished: {}\nLink: {}\n\n{text}",
        item.title, item.feed_title, item.published.as_deref().unwrap_or("?"), item.link))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initialize_negotiates_a_known_version() {
        assert_eq!(initialize(&json!({ "protocolVersion": "2025-03-26" }))["protocolVersion"], "2025-03-26");
        assert_eq!(initialize(&json!({ "protocolVersion": "1999-01-01" }))["protocolVersion"], "2025-06-18");
        assert_eq!(tools().as_array().unwrap().len(), 3);
    }
}
