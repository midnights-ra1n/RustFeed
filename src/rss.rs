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

/// Returns the channel title and its items.
pub async fn fetch_feed(client: &reqwest::Client, url: &str) -> Result<(String, Vec<FeedItem>)> {
    let channel = fetch_channel(client, url).await?;

    let items = channel
        .items()
        .iter()
        .map(|item| FeedItem {
            title: item.title().unwrap_or("Sans titre").to_string(),
            link: item.link().unwrap_or("").to_string(),
            description: item.description().map(String::from),
            published: item.pub_date().map(String::from),
            content: item.content().map(String::from),
        })
        .collect();

    Ok((channel.title().to_string(), items))
}

/// Downloads the article page and extracts its main content (Readability), as HTML.
pub async fn fetch_article(client: &reqwest::Client, url: &str) -> Result<String> {
    let res = client
        .get(url)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await?
        .error_for_status()?;
    if res.content_length().is_some_and(|len| len > 5_000_000) {
        anyhow::bail!("page too large");
    }
    let html = res.text().await?;
    let url = url.to_string();
    // Parsing a big page is CPU work: keep it off the async workers.
    tokio::task::spawn_blocking(move || {
        let article = dom_smoothie::Readability::new(html, Some(&url), None)?.parse()?;
        Ok(article.content.to_string())
    })
    .await?
}
