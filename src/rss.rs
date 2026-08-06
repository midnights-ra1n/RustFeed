// Download feed, parse it into XML and return a list of articles

pub struct FeedItems {
  pub title: String,
  pub link: String,
  pub description: Option<String>,
  pub published: Option<String>,
}
