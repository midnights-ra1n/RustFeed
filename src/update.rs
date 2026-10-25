// Updates from GitHub releases. The app only checks the latest release and, when it
// was installed from the .deb, drops a trigger file: rustfeed-update.path (systemd)
// then runs /usr/lib/rustfeed/update.sh as root, which downloads and installs the
// release itself. The app never runs anything privileged nor chooses what is installed.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use serde_json::{json, Value};

use crate::app::AppState;

const LATEST_RELEASE: &str = "https://api.github.com/repos/midnights-ra1n/RustFeed/releases/latest";
const UPDATER: &str = "/usr/lib/rustfeed/update.sh";
const TRIGGER: &str = "data/update-requested";
/// How often the automatic update looks for a new release.
const AUTO_EVERY: Duration = Duration::from_secs(6 * 3600);

pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// Only a .deb install ships the updater; Docker updates by pulling the image.
pub fn can_install() -> bool {
    Path::new(UPDATER).exists()
}

pub async fn check(client: &reqwest::Client) -> anyhow::Result<Value> {
    let release: Value = client.get(LATEST_RELEASE)
        .header("Accept", "application/vnd.github+json")
        .send().await?
        .error_for_status().context("GitHub n'a pas répondu")?
        .json().await?;
    let tag = release["tag_name"].as_str().unwrap_or_default();
    Ok(json!({
        "current": CURRENT,
        "latest": tag.trim_start_matches('v'),
        "url": release["html_url"],
        "available": is_newer(tag, CURRENT),
        "can_install": can_install(),
    }))
}

pub fn request_install() -> anyhow::Result<()> {
    std::fs::write(TRIGGER, b"").context("Impossible de demander la mise à jour")
}

/// Installs new releases on its own when enabled in the settings. A failed install
/// is simply retried at the next check.
pub async fn auto_loop(app: Arc<AppState>) {
    let mut every = tokio::time::interval(AUTO_EVERY);
    loop {
        every.tick().await;
        if !(app.config.read().unwrap().auto_update && can_install()) {
            continue;
        }
        match check(&app.client).await {
            Ok(release) if release["available"] == true => {
                println!("Automatic update: installing {}.", release["latest"]);
                if let Err(err) = request_install() {
                    eprintln!("Automatic update failed: {err:#}");
                }
            }
            Ok(_) => {}
            Err(err) => eprintln!("Automatic update check failed: {err:#}"),
        }
    }
}

/// Compares dotted versions numerically ("v1.10.0" > "1.9.2"); an unparsable tag is never newer.
fn is_newer(tag: &str, current: &str) -> bool {
    let parse = |v: &str| v.trim_start_matches('v').split('.').map(str::parse::<u64>).collect::<Result<Vec<_>, _>>().ok();
    matches!((parse(tag), parse(current)), (Some(a), Some(b)) if a > b)
}

#[cfg(test)]
mod tests {
    use super::is_newer;

    #[test]
    fn compares_versions() {
        assert!(is_newer("v0.2.0", "0.1.0"));
        assert!(is_newer("1.10.0", "1.9.2"));
        assert!(!is_newer("v0.1.0", "0.1.0"));
        assert!(!is_newer("v0.0.9", "0.1.0"));
        assert!(!is_newer("nightly", "0.1.0"));
    }
}
