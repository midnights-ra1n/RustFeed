// Web interface: feeds, reader, delivery settings and account (password + passkeys).
// The static files in web/ are embedded into the binary at compile time,
// so the executable (and the Docker image) stays self-contained.

use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::Ordering;
use std::sync::Arc;

use axum::extract::{DefaultBodyLimit, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Extension, Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use webauthn_rs::prelude::{PublicKeyCredential, RegisterPublicKeyCredential};

use crate::app::AppState;
use crate::config::{self, Feed, MailConfig, parse_hhmm};
use crate::state::{now, StoredItem};
use crate::{auth, discord, mail, mcp, rss, security, update};

const INDEX_HTML: &str = include_str!("../web/index.html");
const STYLE_CSS: &str = include_str!("../web/style.css");
const APP_JS: &str = include_str!("../web/app.js");
const THEME_JS: &str = include_str!("../web/theme.js");
const FAVICON_SVG: &str = include_str!("../web/favicon.svg");
const MANIFEST: &str = include_str!("../web/manifest.webmanifest");
const SW_JS: &str = include_str!("../web/sw.js");
const ICON_192: &[u8] = include_bytes!("../web/icon-192.png");
const ICON_512: &[u8] = include_bytes!("../web/icon-512.png");

const COOKIE: &str = "rf_session";

type SharedState = Arc<AppState>;

pub async fn serve(listener: tokio::net::TcpListener, app: SharedState) -> anyhow::Result<()> {
    let protected = Router::new()
        .route("/api/feeds", get(list_feeds).post(add_feed).put(edit_feed).delete(delete_feed))
        .route("/api/check", post(check_feed))
        .route("/api/refresh", post(refresh))
        .route("/api/items", get(list_items))
        .route("/api/items/content", post(item_content))
        .route("/api/items/read", post(mark_read))
        .route("/api/settings", get(get_settings).put(put_settings))
        .route("/api/settings/test-webhook", post(test_webhook))
        .route("/api/settings/test-mail", post(test_mail))
        .route("/api/settings/preview", post(preview_mail))
        .route("/api/account", get(account))
        .route("/api/account/password", post(change_password))
        .route("/api/account/access", post(set_access))
        .route("/api/account/mcp", post(set_mcp))
        .route("/api/account/passkey/start", post(passkey_register_start))
        .route("/api/account/passkey/finish", post(passkey_register_finish))
        .route("/api/account/passkey", delete(passkey_delete))
        .route("/api/update", get(check_update).post(install_update))
        .route("/api/update/auto", post(set_auto_update))
        .route_layer(middleware::from_fn_with_state(app.clone(), require_session));

    let router = Router::new()
        .route("/", get(|| async { asset("text/html; charset=utf-8", INDEX_HTML) }))
        .route("/style.css", get(|| async { asset("text/css; charset=utf-8", STYLE_CSS) }))
        .route("/app.js", get(|| async { asset("text/javascript; charset=utf-8", APP_JS) }))
        .route("/theme.js", get(|| async { asset("text/javascript; charset=utf-8", THEME_JS) }))
        .route("/sw.js", get(|| async { asset("text/javascript; charset=utf-8", SW_JS) }))
        .route("/favicon.svg", get(|| async { asset("image/svg+xml", FAVICON_SVG) }))
        .route("/manifest.webmanifest", get(|| async { asset("application/manifest+json", MANIFEST) }))
        .route("/icon-192.png", get(|| async { ([(header::CONTENT_TYPE, "image/png")], ICON_192) }))
        .route("/icon-512.png", get(|| async { ([(header::CONTENT_TYPE, "image/png")], ICON_512) }))
        .route("/api/auth/status", get(auth_status))
        .route("/api/auth/setup", post(setup))
        .route("/api/auth/login", post(login))
        .route("/api/auth/logout", post(logout))
        .route("/api/auth/passkey/start", post(passkey_login_start))
        .route("/api/auth/passkey/finish", post(passkey_login_finish))
        // Authenticated by its own token, not the session cookie.
        .route("/mcp/{token}", post(mcp::handle).get(mcp::not_allowed).delete(mcp::not_allowed))
        .merge(protected)
        .layer(DefaultBodyLimit::max(256 * 1024))
        .layer(middleware::from_fn_with_state(app.clone(), security::firewall))
        .with_state(app);

    axum::serve(listener, router.into_make_service_with_connect_info::<SocketAddr>()).await?;
    Ok(())
}

fn asset(content_type: &'static str, body: &'static str) -> Response {
    ([(header::CONTENT_TYPE, content_type), (header::CACHE_CONTROL, "no-cache")], body).into_response()
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

fn internal(err: anyhow::Error) -> ApiError {
    eprintln!("Internal error: {err:#}");
    ApiError(StatusCode::INTERNAL_SERVER_ERROR, format!("{err:#}"))
}

type ApiResult<T> = Result<T, ApiError>;

// ---------------------------------------------------------------- sessions

fn session_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .find_map(|c| c.trim().strip_prefix(&format!("{COOKIE}=")).map(str::to_string))
}

fn session_cookie(app: &AppState, token: &str, max_age: u64) -> HeaderValue {
    let secure = if app.config.read().unwrap().public_url.starts_with("https://") { "; Secure" } else { "" };
    HeaderValue::from_str(&format!("{COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={max_age}{secure}")).unwrap()
}

fn logged_in(app: &AppState) -> Response {
    let token = app.auth.create_session();
    let mut res = Json(json!({ "ok": true })).into_response();
    res.headers_mut().insert(header::SET_COOKIE, session_cookie(app, &token, auth::SESSION_TTL));
    res
}

async fn require_session(State(app): State<SharedState>, req: Request, next: Next) -> Response {
    match session_token(req.headers()) {
        Some(token) if app.auth.valid_session(&token) => {
            let renewed = app.auth.renew_session(&token);
            let mut res = next.run(req).await;
            if renewed && !res.headers().contains_key(header::SET_COOKIE) {
                res.headers_mut().insert(header::SET_COOKIE, session_cookie(&app, &token, auth::SESSION_TTL));
            }
            res
        }
        _ => ApiError(StatusCode::UNAUTHORIZED, "Session expirée, reconnectez-vous.".into()).into_response(),
    }
}

// ---------------------------------------------------------------- auth

async fn auth_status(State(app): State<SharedState>, headers: HeaderMap) -> Json<Value> {
    let account = app.auth.account();
    let authenticated = session_token(&headers).is_some_and(|t| app.auth.valid_session(&t));
    Json(json!({
        "setup_required": account.is_none(),
        "authenticated": authenticated,
        "passkeys": app.auth.webauthn().is_some() && account.is_some_and(|a| !a.passkeys.is_empty()),
    }))
}

#[derive(Deserialize)]
struct SetupBody {
    code: String,
    username: String,
    password: String,
}

fn validate_password(password: &str) -> ApiResult<()> {
    if password.chars().count() < 10 {
        return Err(bad_request("Le mot de passe doit faire au moins 10 caractères."));
    }
    if password.len() > 256 {
        return Err(bad_request("Mot de passe trop long."));
    }
    Ok(())
}

async fn setup(
    State(app): State<SharedState>,
    Extension(ip): Extension<IpAddr>,
    headers: HeaderMap,
    Json(body): Json<SetupBody>,
) -> ApiResult<Response> {
    let username = body.username.trim().to_string();
    if username.is_empty() || username.len() > 64 {
        return Err(bad_request("Nom d'utilisateur invalide."));
    }
    validate_password(&body.password)?;
    let app2 = app.clone();
    let created = tokio::task::spawn_blocking(move || app2.auth.setup(&body.code, &username, &body.password))
        .await
        .map_err(|e| internal(e.into()))?
        .map_err(internal)?;
    if !created {
        app.guard.strike(ip);
        return Err(ApiError(StatusCode::FORBIDDEN, "Code d'installation invalide.".into()));
    }
    println!("Account created from {ip}.");
    // The address used to create the account becomes the passkey origin.
    if let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()).and_then(|o| validate_url(o).ok()) {
        let origin = origin.trim_end_matches('/').to_string();
        if update_config(&app, |c| { c.public_url = origin.clone(); Ok(()) }).is_ok() {
            app.auth.set_public_url(&origin);
        }
    }
    Ok(logged_in(&app))
}

#[derive(Deserialize)]
struct LoginBody {
    username: String,
    password: String,
}

async fn login(
    State(app): State<SharedState>,
    Extension(ip): Extension<IpAddr>,
    Json(body): Json<LoginBody>,
) -> ApiResult<Response> {
    if body.password.len() > 256 {
        return Err(bad_request("Identifiants invalides."));
    }
    let app2 = app.clone();
    let ok = tokio::task::spawn_blocking(move || app2.auth.check_password(body.username.trim(), &body.password))
        .await
        .map_err(|e| internal(e.into()))?;
    if !ok {
        eprintln!("Failed login from {ip}");
        app.guard.strike(ip);
        return Err(ApiError(StatusCode::UNAUTHORIZED, "Identifiants invalides.".into()));
    }
    Ok(logged_in(&app))
}

async fn logout(State(app): State<SharedState>, headers: HeaderMap) -> Response {
    if let Some(token) = session_token(&headers) {
        app.auth.end_session(&token);
    }
    let mut res = Json(json!({ "ok": true })).into_response();
    res.headers_mut().insert(header::SET_COOKIE, session_cookie(&app, "", 0));
    res
}

async fn passkey_login_start(State(app): State<SharedState>) -> ApiResult<Json<Value>> {
    let (id, options) = app.auth.start_login().map_err(|e| bad_request(format!("{e:#}")))?;
    Ok(Json(json!({ "id": id, "options": options })))
}

#[derive(Deserialize)]
struct PasskeyLoginBody {
    id: String,
    credential: PublicKeyCredential,
}

async fn passkey_login_finish(
    State(app): State<SharedState>,
    Extension(ip): Extension<IpAddr>,
    Json(body): Json<PasskeyLoginBody>,
) -> ApiResult<Response> {
    if let Err(err) = app.auth.finish_login(&body.id, &body.credential) {
        eprintln!("Failed passkey login from {ip}: {err:#}");
        app.guard.strike(ip);
        return Err(ApiError(StatusCode::UNAUTHORIZED, "Passkey refusée.".into()));
    }
    Ok(logged_in(&app))
}

// ---------------------------------------------------------------- account

async fn account(State(app): State<SharedState>) -> ApiResult<Json<Value>> {
    let acc = app.auth.account().ok_or_else(|| bad_request("Aucun compte."))?;
    let passkeys: Vec<Value> = acc.passkeys.iter()
        .map(|p| json!({ "id": p.id, "name": p.name, "created": p.created }))
        .collect();
    Ok(Json(json!({
        "username": acc.username,
        "passkeys": passkeys,
        "passkeys_available": app.auth.webauthn().is_some(),
        "public_url": app.config.read().unwrap().public_url,
        "behind_proxy": app.config.read().unwrap().behind_proxy,
        "mcp": mcp_info(&app),
        "version": update::CURRENT,
        "auto_update": app.config.read().unwrap().auto_update,
    })))
}

fn mcp_info(app: &AppState) -> Value {
    let c = app.config.read().unwrap();
    let url = (!c.mcp.token.is_empty()).then(|| format!("{}/mcp/{}", c.public_url.trim_end_matches('/'), c.mcp.token));
    json!({ "enabled": c.mcp.enabled, "active": c.mcp_active(), "url": url, "https": c.public_url.starts_with("https://") })
}

#[derive(Deserialize)]
struct McpBody {
    enabled: bool,
    #[serde(default)]
    regenerate: bool,
}

async fn set_mcp(State(app): State<SharedState>, Json(body): Json<McpBody>) -> ApiResult<Json<Value>> {
    update_config(&app, |c| {
        if body.enabled && !c.public_url.starts_with("https://") {
            return Err(bad_request("Le serveur MCP exige une adresse publique en HTTPS (derrière un reverse proxy)."));
        }
        c.mcp.enabled = body.enabled;
        if body.regenerate || (body.enabled && c.mcp.token.is_empty()) {
            c.mcp.token = auth::random_token();
        }
        Ok(())
    })?;
    account(State(app)).await
}

#[derive(Deserialize)]
struct AccessBody {
    behind_proxy: bool,
    /// Sets `public_url` to the address the browser is using right now.
    #[serde(default)]
    use_current_origin: bool,
}

/// The public address is only ever set to the origin the request comes from, so it
/// always matches an address that works (no lock-out with a wrong https:// value).
async fn set_access(
    State(app): State<SharedState>,
    headers: HeaderMap,
    Json(body): Json<AccessBody>,
) -> ApiResult<Response> {
    let origin = if body.use_current_origin {
        let origin = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok())
            .ok_or_else(|| bad_request("En-tête Origin manquant."))?;
        Some(validate_url(origin)?.trim_end_matches('/').to_string())
    } else {
        None
    };
    update_config(&app, |c| {
        c.behind_proxy = body.behind_proxy;
        if let Some(origin) = &origin {
            c.public_url = origin.clone();
        }
        Ok(())
    })?;
    if let Some(origin) = &origin {
        if !app.auth.set_public_url(origin) {
            return Err(bad_request("Adresse enregistrée, mais les passkeys exigent un nom de domaine (pas une IP) en HTTPS ou localhost."));
        }
    }
    // Re-issue the cookie so its Secure flag follows the new address.
    let token = session_token(&headers).unwrap_or_default();
    let mut res = account(State(app.clone())).await?.into_response();
    res.headers_mut().insert(header::SET_COOKIE, session_cookie(&app, &token, auth::SESSION_TTL));
    Ok(res)
}

#[derive(Deserialize)]
struct PasswordBody {
    current: String,
    new: String,
}

async fn change_password(
    State(app): State<SharedState>,
    Extension(ip): Extension<IpAddr>,
    Json(body): Json<PasswordBody>,
) -> ApiResult<Response> {
    validate_password(&body.new)?;
    if body.current.len() > 256 {
        return Err(bad_request("Mot de passe actuel incorrect."));
    }
    let app2 = app.clone();
    let changed = tokio::task::spawn_blocking(move || app2.auth.change_password(&body.current, &body.new))
        .await
        .map_err(|e| internal(e.into()))?
        .map_err(internal)?;
    if !changed {
        app.guard.strike(ip);
        return Err(ApiError(StatusCode::FORBIDDEN, "Mot de passe actuel incorrect.".into()));
    }
    // All sessions were revoked: give this one a fresh session.
    Ok(logged_in(&app))
}

async fn passkey_register_start(State(app): State<SharedState>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    let token = session_token(&headers).unwrap_or_default();
    let options = app.auth.start_registration(&token).map_err(|e| bad_request(format!("{e:#}")))?;
    Ok(Json(json!(options)))
}

#[derive(Deserialize)]
struct PasskeyRegisterBody {
    name: String,
    credential: RegisterPublicKeyCredential,
}

async fn passkey_register_finish(
    State(app): State<SharedState>,
    headers: HeaderMap,
    Json(body): Json<PasskeyRegisterBody>,
) -> ApiResult<Json<Value>> {
    let token = session_token(&headers).unwrap_or_default();
    app.auth
        .finish_registration(&token, &body.name, &body.credential)
        .map_err(|e| bad_request(format!("Enregistrement refusé : {e:#}")))?;
    account(State(app)).await
}

#[derive(Deserialize)]
struct IdBody {
    id: String,
}

async fn passkey_delete(State(app): State<SharedState>, Json(body): Json<IdBody>) -> ApiResult<Json<Value>> {
    if !app.auth.delete_passkey(&body.id).map_err(internal)? {
        return Err(ApiError(StatusCode::NOT_FOUND, "Passkey introuvable.".into()));
    }
    account(State(app)).await
}

// ---------------------------------------------------------------- feeds

#[derive(Serialize)]
struct FeedList {
    feeds: Vec<Value>,
    interval: u64,
}

#[derive(Deserialize)]
struct FeedBody {
    url: String,
    #[serde(default)]
    interval: Option<u64>,
    #[serde(default)]
    folder: String,
    #[serde(default)]
    tags: Vec<String>,
}

#[derive(Deserialize)]
struct EditBody {
    old_url: String,
    url: String,
    #[serde(default)]
    interval: Option<u64>,
    #[serde(default)]
    folder: String,
    #[serde(default)]
    tags: Vec<String>,
}

/// Validates a folder and tags and returns them trimmed, without duplicates.
fn validate_labels(folder: &str, tags: &[String]) -> ApiResult<(String, Vec<String>)> {
    let folder = folder.trim().to_string();
    let mut clean: Vec<String> = Vec::new();
    for tag in tags.iter().map(|t| t.trim()).filter(|t| !t.is_empty()) {
        if !clean.iter().any(|t| t == tag) {
            clean.push(tag.to_string());
        }
    }
    if folder.chars().count() > 64 || clean.iter().any(|t| t.chars().count() > 32) || clean.len() > 20 {
        return Err(bad_request("Dossier (64 caractères) ou tags (20 tags de 32 caractères) trop longs."));
    }
    Ok((folder, clean))
}

fn validate_url(raw: &str) -> ApiResult<String> {
    let url = raw.trim();
    let parsed = reqwest::Url::parse(url).map_err(|_| bad_request("URL invalide."))?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() || url.len() > 2048 {
        return Err(bad_request("L'URL doit commencer par http:// ou https://."));
    }
    Ok(url.to_string())
}

fn validate_interval(interval: Option<u64>) -> ApiResult<Option<u64>> {
    match interval {
        Some(s) if !(60..=7 * 86400).contains(&s) => Err(bad_request("La fréquence doit être entre 1 minute et 7 jours.")),
        _ => Ok(interval),
    }
}

fn feed_list(app: &AppState) -> FeedList {
    let config = app.config.read().unwrap();
    let status = app.status.lock().unwrap();
    let feeds = config.feeds.iter()
        .map(|f| json!({ "url": f.url, "interval": f.interval, "folder": f.folder, "tags": f.tags, "status": status.get(&f.url) }))
        .collect();
    FeedList { feeds, interval: config.interval }
}

/// Applies `change` to a copy of the config, persists it to config.toml, and only
/// then swaps it in memory, so the running config never diverges from the file.
fn update_config(app: &AppState, change: impl FnOnce(&mut config::Config) -> ApiResult<()>) -> ApiResult<()> {
    let mut current = app.config.write().unwrap();
    let mut next = current.clone();
    change(&mut next)?;
    config::save(&next).map_err(|err| {
        eprintln!("Failed to save config: {err:#}");
        ApiError(StatusCode::INTERNAL_SERVER_ERROR, format!("Impossible d'enregistrer la configuration : {err:#}"))
    })?;
    *current = next;
    Ok(())
}

async fn list_feeds(State(app): State<SharedState>) -> Json<FeedList> {
    Json(feed_list(&app))
}

async fn add_feed(State(app): State<SharedState>, Json(body): Json<FeedBody>) -> ApiResult<(StatusCode, Json<FeedList>)> {
    let url = validate_url(&body.url)?;
    let interval = validate_interval(body.interval)?;
    let (folder, tags) = validate_labels(&body.folder, &body.tags)?;
    update_config(&app, |c| {
        if c.feeds.iter().any(|f| f.url == url) {
            return Err(ApiError(StatusCode::CONFLICT, "Ce flux existe déjà.".into()));
        }
        c.feeds.push(Feed { url: url.clone(), interval, folder, tags });
        Ok(())
    })?;

    app.mark_for_baseline(&url);
    app.wake.notify_one();
    println!("Feed added from web interface: {url}");
    Ok((StatusCode::CREATED, Json(feed_list(&app))))
}

async fn edit_feed(State(app): State<SharedState>, Json(body): Json<EditBody>) -> ApiResult<Json<FeedList>> {
    let url = validate_url(&body.url)?;
    let interval = validate_interval(body.interval)?;
    let (folder, tags) = validate_labels(&body.folder, &body.tags)?;
    update_config(&app, |c| {
        let index = c.feeds.iter()
            .position(|f| f.url == body.old_url)
            .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "Flux introuvable.".into()))?;
        if url != body.old_url && c.feeds.iter().any(|f| f.url == url) {
            return Err(ApiError(StatusCode::CONFLICT, "Ce flux existe déjà.".into()));
        }
        c.feeds[index] = Feed { url: url.clone(), interval, folder, tags };
        for excluded in c.mail.exclude_feeds.iter_mut().filter(|f| **f == body.old_url) {
            *excluded = url.clone();
        }
        Ok(())
    })?;

    if url != body.old_url {
        app.forget_baseline(&body.old_url);
        app.mark_for_baseline(&url);
        app.status.lock().unwrap().remove(&body.old_url);
        println!("Feed edited from web interface: {} -> {url}", body.old_url);
    }
    app.wake.notify_one();
    Ok(Json(feed_list(&app)))
}

async fn delete_feed(State(app): State<SharedState>, Json(body): Json<FeedBody>) -> ApiResult<Json<FeedList>> {
    update_config(&app, |c| {
        let index = c.feeds.iter()
            .position(|f| f.url == body.url)
            .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "Flux introuvable.".into()))?;
        c.feeds.remove(index);
        c.mail.exclude_feeds.retain(|f| *f != body.url);
        Ok(())
    })?;

    app.forget_baseline(&body.url);
    app.status.lock().unwrap().remove(&body.url);
    app.state.lock().unwrap().forget_feed(&body.url);
    println!("Feed removed from web interface: {}", body.url);
    Ok(Json(feed_list(&app)))
}

/// Downloads and parses a feed without saving anything, so the UI can show
/// its title and tell whether it is actually a valid RSS feed.
async fn check_feed(State(app): State<SharedState>, Json(body): Json<FeedBody>) -> ApiResult<Json<Value>> {
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

async fn refresh(State(app): State<SharedState>) -> Json<Value> {
    app.refresh_all.store(true, Ordering::Relaxed);
    app.wake.notify_one();
    Json(json!({ "ok": true }))
}

async fn check_update(State(app): State<SharedState>) -> ApiResult<Json<Value>> {
    update::check(&app.client).await.map(Json).map_err(internal)
}

async fn install_update() -> ApiResult<Json<Value>> {
    if !update::can_install() {
        return Err(bad_request("Mise à jour automatique disponible uniquement pour l'installation .deb."));
    }
    update::request_install().map_err(internal)?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct EnabledBody {
    enabled: bool,
}

async fn set_auto_update(State(app): State<SharedState>, Json(body): Json<EnabledBody>) -> ApiResult<Json<Value>> {
    if body.enabled && !update::can_install() {
        return Err(bad_request("Mise à jour automatique disponible uniquement pour l'installation .deb."));
    }
    update_config(&app, |c| { c.auto_update = body.enabled; Ok(()) })?;
    account(State(app)).await
}

// ---------------------------------------------------------------- reader

async fn list_items(State(app): State<SharedState>) -> Json<Value> {
    let state = app.state.lock().unwrap();
    let items: Vec<Value> = state.items.iter()
        .map(|i| json!({
            "feed": i.feed,
            "feed_title": i.feed_title,
            "title": i.title,
            "link": i.link,
            "summary": i.summary,
            "published": i.published,
            "fetched_at": i.fetched_at,
            "read": state.read.contains(&i.link),
        }))
        .collect();
    Json(json!({ "items": items }))
}

#[derive(Deserialize)]
struct LinkBody {
    link: String,
}

async fn item_content(State(app): State<SharedState>, Json(body): Json<LinkBody>) -> ApiResult<Json<Value>> {
    let item = {
        let mut state = app.state.lock().unwrap();
        let item = state.items.iter().find(|i| i.link == body.link).cloned()
            .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "Article introuvable.".into()))?;
        state.read.insert(item.link.clone());
        item
    };
    Ok(Json(json!({ "content": app.article_html(&item).await })))
}

#[derive(Deserialize)]
struct ReadBody {
    links: Vec<String>,
    read: bool,
}

async fn mark_read(State(app): State<SharedState>, Json(body): Json<ReadBody>) -> Json<Value> {
    let mut state = app.state.lock().unwrap();
    for link in body.links {
        if body.read {
            if state.items.iter().any(|i| i.link == link) {
                state.read.insert(link);
            }
        } else {
            state.read.remove(&link);
        }
    }
    Json(json!({ "ok": true }))
}

// ---------------------------------------------------------------- delivery settings

#[derive(Serialize, Deserialize)]
struct Settings {
    interval: u64,
    webhook: String,
    webhook_enabled: bool,
    mail: MailConfig,
    #[serde(default)]
    password_set: bool,
}

async fn get_settings(State(app): State<SharedState>) -> Json<Settings> {
    let c = app.config.read().unwrap();
    let mut mail = c.mail.clone();
    let password_set = !mail.password.is_empty();
    mail.password.clear();
    // The old "PUT YOUR WEBHOOK URL HERE" placeholder would fail validation on save.
    let webhook = if c.webhook.starts_with("http") { c.webhook.clone() } else { String::new() };
    Json(Settings { interval: c.interval, webhook, webhook_enabled: c.webhook_enabled, mail, password_set })
}

fn validate_mail(mail: &MailConfig) -> ApiResult<()> {
    if !matches!(mail.security.as_str(), "starttls" | "tls" | "none") {
        return Err(bad_request("Sécurité SMTP invalide."));
    }
    if mail.frequency > 7 * 86400 {
        return Err(bad_request("La fréquence d'envoi ne peut pas dépasser 7 jours."));
    }
    if mail.send_times.len() > 24 || mail.send_times.iter().any(|t| parse_hhmm(t).is_none()) {
        return Err(bad_request("Heures d'envoi invalides (HH:MM, 24 au maximum)."));
    }
    if mail.retry && !(60..=86400).contains(&mail.retry_delay) {
        return Err(bad_request("Le délai de réessai doit être compris entre 1 min et 24 h."));
    }
    if mail.template.len() + mail.item_template.len() + mail.subject.len() > 100_000 {
        return Err(bad_request("Modèle de mail trop volumineux."));
    }
    if mail.subject.contains(['\r', '\n']) {
        return Err(bad_request("Le sujet ne peut pas contenir de retour à la ligne."));
    }
    if !mail.timezone.is_empty() && jiff::tz::TimeZone::get(&mail.timezone).is_err() {
        return Err(bad_request(format!("Fuseau horaire inconnu : {}", mail.timezone)));
    }
    if mail.quiet_hours.len() > 20 {
        return Err(bad_request("20 plages de silence au maximum."));
    }
    for range in &mail.quiet_hours {
        if parse_hhmm(&range.start).is_none() || parse_hhmm(&range.end).is_none() {
            return Err(bad_request("Horaire de plage de silence invalide (HH:MM)."));
        }
        if range.days.is_empty() || range.days.iter().any(|d| !(1..=7).contains(d)) {
            return Err(bad_request("Choisissez au moins un jour pour chaque plage de silence."));
        }
    }
    if mail.exclude_folders.len() + mail.exclude_tags.len() + mail.exclude_feeds.len() > 1000 {
        return Err(bad_request("Trop d'exclusions."));
    }
    if mail.enabled {
        if mail.smtp_host.trim().is_empty() || mail.smtp_port == 0 {
            return Err(bad_request("Serveur SMTP requis."));
        }
        mail.from.parse::<lettre::message::Mailbox>().map_err(|_| bad_request("Adresse d'expéditeur invalide."))?;
        if mail.to.is_empty() {
            return Err(bad_request("Au moins un destinataire est requis."));
        }
        for to in &mail.to {
            to.parse::<lettre::message::Mailbox>().map_err(|_| bad_request(format!("Destinataire invalide : {to}")))?;
        }
    }
    Ok(())
}

/// Builds the mail config from the form, keeping the stored password when the field was left empty.
fn incoming_mail(app: &AppState, mut mail: MailConfig) -> MailConfig {
    mail.smtp_host = mail.smtp_host.trim().to_string();
    mail.from = mail.from.trim().to_string();
    mail.to = mail.to.iter().map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect();
    mail.timezone = mail.timezone.trim().to_string();
    if mail.password.is_empty() {
        mail.password = app.config.read().unwrap().mail.password.clone();
    }
    mail
}

async fn put_settings(State(app): State<SharedState>, Json(body): Json<Settings>) -> ApiResult<Json<Settings>> {
    validate_interval(Some(body.interval))?;
    let webhook = body.webhook.trim().to_string();
    if !webhook.is_empty() {
        validate_url(&webhook)?;
    }
    let mail = incoming_mail(&app, body.mail);
    validate_mail(&mail)?;

    let enabled = mail.enabled;
    update_config(&app, |c| {
        c.interval = body.interval;
        c.webhook = webhook;
        c.webhook_enabled = body.webhook_enabled;
        c.mail = mail;
        Ok(())
    })?;
    if !enabled {
        app.state.lock().unwrap().mail_queue.clear();
    }
    app.wake.notify_one();
    Ok(get_settings(State(app)).await)
}

fn sample_items(app: &AppState) -> Vec<StoredItem> {
    let items: Vec<StoredItem> = app.state.lock().unwrap().items.iter().take(3).cloned().collect();
    if !items.is_empty() {
        return items;
    }
    vec![StoredItem {
        feed: String::new(),
        feed_title: "Exemple de flux".into(),
        title: "Un article d'exemple".into(),
        link: "https://example.com/article".into(),
        summary: "Le résumé de l'article apparaît ici, sans mise en forme et coupé à 300 caractères.".into(),
        content: None,
        published: None,
        fetched_at: now(),
    }]
}

async fn preview_mail(State(app): State<SharedState>, Json(mail): Json<MailConfig>) -> Json<Value> {
    let (subject, html, _) = mail::render(&mail, &sample_items(&app));
    Json(json!({ "subject": subject, "html": html }))
}

async fn test_mail(State(app): State<SharedState>, Json(mail): Json<MailConfig>) -> ApiResult<Json<Value>> {
    let mail = incoming_mail(&app, MailConfig { enabled: true, ..mail });
    validate_mail(&mail)?;
    mail::send(&mail, &sample_items(&app)).await.map_err(|e| bad_request(format!("{e:#}")))?;
    Ok(Json(json!({ "ok": true })))
}

async fn test_webhook(State(app): State<SharedState>, Json(body): Json<LinkBody>) -> ApiResult<Json<Value>> {
    let url = validate_url(&body.link)?;
    let item = sample_items(&app).remove(0);
    discord::send_embed(&app.client, &url, &item).await.map_err(|e| bad_request(format!("{e:#}")))?;
    Ok(Json(json!({ "ok": true })))
}
