// Recept FeedItem and built a embed

use anyhow::Result;
use serde_json::json;

use crate::models::FeedItem;

pub async fn send_embed(client: &reqwest::Client, webhook: &str, item: &FeedItem) -> Result<()> {
    let embed = json!({
        "embeds": [{
            "title": item.title,
            "url": item.link,
            "description": item.description,
            "footer": item.published.as_ref().map(|p| json!({ "text": p })),
        }]
    });

    client
        .post(webhook)
        .json(&embed)
        .send()
        .await?
        .error_for_status()?;

    Ok(())
}
