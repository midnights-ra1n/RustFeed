pub struct FeedItem {
  pub title: String,
  pub link: String,
  pub description: Option<String>,
  pub published: Option<String>,
  /// Full HTML content (content:encoded) when the feed provides it.
  pub content: Option<String>,
}

/// Keeps the text of an HTML snippet, collapses whitespace and cuts it to `max` chars.
pub fn plain_text(html: &str, max: usize) -> String {
  let mut out = String::new();
  let mut in_tag = false;
  for c in html.chars() {
    match c {
      '<' => in_tag = true,
      '>' if in_tag => { in_tag = false; out.push(' '); }
      _ if !in_tag => out.push(c),
      _ => {}
    }
  }
  let out = out
    .replace("&nbsp;", " ").replace("&#160;", " ")
    .replace("&quot;", "\"").replace("&#39;", "'").replace("&apos;", "'")
    .replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&");
  let words: Vec<&str> = out.split_whitespace().collect();
  let text = words.join(" ");
  if text.chars().count() <= max {
    return text;
  }
  let cut: String = text.chars().take(max).collect();
  format!("{}…", cut.trim_end())
}

#[cfg(test)]
mod tests {
  #[test]
  fn plain_text_strips_tags_and_truncates() {
    assert_eq!(super::plain_text("<p>Hello&nbsp;<b>world</b></p>", 100), "Hello world");
    assert_eq!(super::plain_text("abcdef", 3), "abc…");
  }
}
