// Sends an article to the webhook: a Discord embed for Discord URLs,
// a plain JSON object for any other webhook (Slack-like bridges, n8n, ntfy...).

use anyhow::Result;
use serde_json::json;

use crate::state::StoredItem;

pub async fn send_embed(client: &reqwest::Client, webhook: &str, item: &StoredItem) -> Result<()> {
    let is_discord = webhook.starts_with("https://discord.com/api/webhooks/")
        || webhook.starts_with("https://discordapp.com/api/webhooks/");

    let body = if is_discord {
        json!({
            "embeds": [{
                // Discord limits: title 256, description 4096
                "title": item.title.chars().take(256).collect::<String>(),
                "url": item.link,
                "description": item.summary,
                "author": { "name": item.feed_title.chars().take(256).collect::<String>() },
                "color": 0xff7a59,
                "footer": item.published.as_ref().map(|p| json!({ "text": p })),
            }]
        })
    } else {
        json!({
            "title": item.title,
            "link": item.link,
            "description": item.summary,
            "feed": item.feed_title,
            "published": item.published,
        })
    };

    client
        .post(webhook)
        .json(&body)
        .send()
        .await?
        .error_for_status()?;

    Ok(())
}
