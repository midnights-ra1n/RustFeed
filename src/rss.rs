// Download feed, parse it into XML and return a list of articles

use anyhow::Result;

pub async fn fetch_feed(url: &str) -> Result<String> {
  let response = reqwest::get(url).await?;
  let body = response.text().await?;
  Ok(body)
}

pub struct FeedItem {
  pub title: String,
  pub link: String,
  pub description: Option<String>,
  pub published: Option<String>,
}
