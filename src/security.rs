// Small embedded WAF, per-IP rate limiting and security headers.
// The WAF only inspects the URL and headers: request bodies legitimately contain
// HTML (mail templates) and are validated by each handler instead.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};

use axum::extract::{ConnectInfo, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::app::AppState;
use crate::state::now;

const WINDOW: u64 = 60;
const MAX_REQUESTS: u32 = 240;
const MAX_AUTH_ATTEMPTS: u32 = 10;
const STRIKES_BEFORE_BAN: u32 = 5;
const BAN_SECS: u64 = 15 * 60;

#[derive(Default)]
struct Client {
    window_start: u64,
    requests: u32,
    auth_window_start: u64,
    auth_attempts: u32,
    strikes: u32,
    banned_until: u64,
}

#[derive(Default)]
pub struct Guard {
    clients: Mutex<HashMap<IpAddr, Client>>,
}

pub enum Verdict {
    Allow,
    Limited,
    Banned,
}

impl Guard {
    fn with<T>(&self, ip: IpAddr, f: impl FnOnce(&mut Client, u64) -> T) -> T {
        let mut clients = self.clients.lock().unwrap();
        let t = now();
        if clients.len() > 10_000 {
            clients.retain(|_, c| c.banned_until > t || t - c.window_start < WINDOW);
        }
        f(clients.entry(ip).or_default(), t)
    }

    pub fn check(&self, ip: IpAddr, auth: bool) -> Verdict {
        self.with(ip, |c, t| {
            if c.banned_until > t {
                return Verdict::Banned;
            }
            if t - c.window_start >= WINDOW {
                c.window_start = t;
                c.requests = 0;
            }
            c.requests += 1;
            if auth {
                if t - c.auth_window_start >= WINDOW {
                    c.auth_window_start = t;
                    c.auth_attempts = 0;
                }
                c.auth_attempts += 1;
                if c.auth_attempts > MAX_AUTH_ATTEMPTS {
                    return Verdict::Limited;
                }
            }
            if c.requests > MAX_REQUESTS { Verdict::Limited } else { Verdict::Allow }
        })
    }

    /// Records a blocked attack or failed login; enough of them bans the IP.
    pub fn strike(&self, ip: IpAddr) {
        self.with(ip, |c, t| {
            c.strikes += 1;
            if c.strikes >= STRIKES_BEFORE_BAN {
                c.strikes = 0;
                c.banned_until = t + BAN_SECS;
                eprintln!("WAF: banning {ip} for {} min", BAN_SECS / 60);
            }
        })
    }
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
            if let Ok(b) = u8::from_str_radix(hex, 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(if bytes[i] == b'+' { b' ' } else { bytes[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

const BAD_PATTERNS: &[&str] = &[
    // traversal / file access
    "../", "..\\", "\0", "/etc/passwd", "/proc/self", "c:\\windows", "cmd.exe", "/bin/sh",
    // xss
    "<script", "javascript:", "onerror=", "onload=", "<iframe", "<svg",
    // sql injection
    "union select", "union all select", "' or '1'='1", "\" or \"1\"=\"1", " or 1=1", "information_schema",
    "sleep(", "benchmark(", "waitfor delay", "xp_cmdshell",
    // template / command injection
    "${jndi:", "{{7*7}}", "$(", "`", ";wget ", ";curl ", "| sh",
];

const BAD_PATHS: &[&str] = &[
    "/.env", "/.git", "/.aws", "/.ssh", "/wp-", "/xmlrpc.php", "/phpmyadmin", "/cgi-bin", "/actuator",
    "/server-status", "/vendor/phpunit", "/admin.php", "/boaform", "/hnap1",
];

const BAD_AGENTS: &[&str] = &[
    "sqlmap", "nikto", "nmap", "masscan", "zgrab", "nuclei", "acunetix", "wpscan", "dirbuster", "gobuster",
    "fuzz faster", "ffuf", "havij", "netsparker", "jaeles",
];

/// Returns the reason a request looks malicious, if it does.
pub fn inspect(method: &Method, path_and_query: &str, headers: &HeaderMap) -> Option<&'static str> {
    if path_and_query.len() > 4096 {
        return Some("URL trop longue");
    }
    if !matches!(*method, Method::GET | Method::HEAD | Method::POST | Method::PUT | Method::DELETE) {
        return Some("méthode non autorisée");
    }
    // Decode twice to catch double-encoded payloads (%252e%252e).
    let url = percent_decode(&percent_decode(path_and_query)).to_lowercase();
    let path = url.split('?').next().unwrap_or("");
    if BAD_PATHS.iter().any(|p| path.starts_with(p)) || path.ends_with(".php") || path.ends_with(".asp") {
        return Some("chemin de scanner");
    }
    if BAD_PATTERNS.iter().any(|p| url.contains(p)) {
        return Some("motif d'attaque dans l'URL");
    }
    let agent = headers.get(header::USER_AGENT).and_then(|v| v.to_str().ok()).unwrap_or("").to_lowercase();
    if BAD_AGENTS.iter().any(|a| agent.contains(a)) {
        return Some("outil d'attaque");
    }
    for value in headers.values() {
        let v = value.as_bytes();
        if v.len() > 8192 || v.windows(7).any(|w| w.eq_ignore_ascii_case(b"${jndi:")) {
            return Some("en-tête suspect");
        }
    }
    None
}

pub fn client_ip(app: &AppState, peer: SocketAddr, headers: &HeaderMap) -> IpAddr {
    if app.config.read().unwrap().behind_proxy {
        // The rightmost entry is the one appended by our own proxy, so it can't be spoofed.
        if let Some(ip) = headers
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.rsplit(',').next())
            .and_then(|v| v.trim().parse().ok())
        {
            return ip;
        }
    }
    peer.ip()
}

/// Same-origin check for state-changing requests (CSRF), on top of SameSite=Strict cookies.
fn same_origin(headers: &HeaderMap) -> bool {
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok());
    match headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
        Some(origin) => {
            let origin_host = origin.split("://").nth(1);
            origin_host.is_some() && origin_host == host
        }
        None => headers.get("sec-fetch-site").and_then(|v| v.to_str().ok()).is_none_or(|s| s == "same-origin" || s == "none"),
    }
}

fn reject(status: StatusCode, msg: &str) -> Response {
    (status, axum::Json(serde_json::json!({ "error": msg }))).into_response()
}

pub async fn firewall(
    State(app): State<Arc<AppState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    mut req: Request,
    next: Next,
) -> Response {
    let ip = client_ip(&app, peer, req.headers());
    req.extensions_mut().insert(ip);
    let path_and_query = req.uri().path_and_query().map(|p| p.as_str()).unwrap_or("/");
    let is_auth = req.method() == Method::POST && req.uri().path().starts_with("/api/auth/");

    match app.guard.check(ip, is_auth) {
        Verdict::Banned => return reject(StatusCode::FORBIDDEN, "Adresse temporairement bloquée."),
        Verdict::Limited => {
            let mut res = reject(StatusCode::TOO_MANY_REQUESTS, "Trop de requêtes, réessayez dans une minute.");
            res.headers_mut().insert(header::RETRY_AFTER, HeaderValue::from_static("60"));
            return res;
        }
        Verdict::Allow => {}
    }

    if let Some(reason) = inspect(req.method(), path_and_query, req.headers()) {
        eprintln!("WAF: blocked {ip} {} {path_and_query} ({reason})", req.method());
        app.guard.strike(ip);
        return reject(StatusCode::FORBIDDEN, "Requête bloquée.");
    }

    if req.method() != Method::GET && req.method() != Method::HEAD {
        if !same_origin(req.headers()) {
            return reject(StatusCode::FORBIDDEN, "Origine refusée.");
        }
        let json = req.headers().get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("application/json"));
        if !json {
            return reject(StatusCode::UNSUPPORTED_MEDIA_TYPE, "JSON attendu.");
        }
    }

    let https = app.config.read().unwrap().public_url.starts_with("https://");
    let mut res = next.run(req).await;
    let h = res.headers_mut();
    h.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static(
        "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src * data:; \
         media-src *; connect-src 'self'; frame-src 'self'; object-src 'none'; base-uri 'none'; \
         form-action 'self'; frame-ancestors 'none'; manifest-src 'self'; worker-src 'self'",
    ));
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    h.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    h.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    h.insert("permissions-policy", HeaderValue::from_static("camera=(), microphone=(), geolocation=(), interest-cohort=()"));
    h.insert("cross-origin-opener-policy", HeaderValue::from_static("same-origin"));
    if https {
        h.insert(header::STRICT_TRANSPORT_SECURITY, HeaderValue::from_static("max-age=31536000"));
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(url: &str) -> Option<&'static str> {
        inspect(&Method::GET, url, &HeaderMap::new())
    }

    #[test]
    fn waf_blocks_attacks_and_lets_normal_requests_through() {
        assert!(check("/").is_none());
        assert!(check("/api/items?feed=https%3A%2F%2Fexample.com%2Ffeed.xml").is_none());
        assert!(check("/../etc/passwd").is_some());
        assert!(check("/%252e%252e/secret").is_some());
        assert!(check("/?q=1%27%20UNION%20SELECT%20*").is_some());
        assert!(check("/?x=<script>alert(1)</script>").is_some());
        assert!(check("/wp-admin/").is_some());
        assert!(check("/index.php").is_some());
        assert!(inspect(&Method::PATCH, "/", &HeaderMap::new()).is_some());

        let mut h = HeaderMap::new();
        h.insert(header::USER_AGENT, HeaderValue::from_static("sqlmap/1.7"));
        assert!(inspect(&Method::GET, "/", &h).is_some());
    }

    #[test]
    fn rate_limit_and_ban() {
        let g = Guard::default();
        let ip: IpAddr = "10.0.0.1".parse().unwrap();
        for _ in 0..MAX_AUTH_ATTEMPTS {
            assert!(matches!(g.check(ip, true), Verdict::Allow));
        }
        assert!(matches!(g.check(ip, true), Verdict::Limited));
        for _ in 0..STRIKES_BEFORE_BAN {
            g.strike(ip);
        }
        assert!(matches!(g.check(ip, false), Verdict::Banned));
    }

    #[test]
    fn csrf_origin_check() {
        let mut h = HeaderMap::new();
        h.insert(header::HOST, HeaderValue::from_static("feed.local:3060"));
        h.insert(header::ORIGIN, HeaderValue::from_static("http://feed.local:3060"));
        assert!(same_origin(&h));
        h.insert(header::ORIGIN, HeaderValue::from_static("https://evil.com"));
        assert!(!same_origin(&h));
    }
}
