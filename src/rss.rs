// Download feed, parse it into XML and return a list of articles

use anyhow::Result;

use crate::models::FeedItem;

pub async fn fetch_feed(url: &str) -> Result<Vec<FeedItem>> {
    let response = reqwest::get(url).await?;
    let body = response.bytes().await?;

    let channel = rss::Channel::read_from(&body[..])?;

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
