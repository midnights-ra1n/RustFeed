// Download feed, parse it into XML and return a list of articles

use anyhow::Result;

use crate::models::FeedItem;

pub async fn fetch_channel(client: &reqwest::Client, url: &str) -> Result<rss::Channel> {
    let body = client
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;

    Ok(rss::Channel::read_from(&body[..])?)
}

pub async fn fetch_feed(client: &reqwest::Client, url: &str) -> Result<Vec<FeedItem>> {
    let channel = fetch_channel(client, url).await?;

    let items = channel
        .items()
        .iter()
        .map(|item| FeedItem {
            title: item.title().unwrap_or("Sans titre").to_string(),
            link: item.link().unwrap_or("").to_string(),
            description: item.description().map(String::from),
            published: item.pub_date().map(String::from),
        })
        .collect();

    Ok(items)
}
