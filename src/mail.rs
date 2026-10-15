// Builds the digest email from the user's templates and sends it over SMTP.
// Templates use {{placeholders}}; every value is HTML-escaped before insertion.

use anyhow::{bail, Context, Result};
use lettre::message::{Mailbox, MultiPart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

use crate::config::MailConfig;
use crate::state::StoredItem;

pub fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn fill(template: &str, vars: &[(&str, &str)]) -> String {
    let mut out = template.to_string();
    for (key, value) in vars {
        out = out.replace(&format!("{{{{{key}}}}}"), value);
    }
    out
}

/// Returns (subject, html, plain text).
pub fn render(cfg: &MailConfig, items: &[StoredItem]) -> (String, String, String) {
    let count = items.len().to_string();
    let date = jiff::Timestamp::now().to_zoned(cfg.tz()).strftime("%d/%m/%Y").to_string();

    let rows: String = items
        .iter()
        .map(|item| {
            let link = if item.link.starts_with("http") { item.link.as_str() } else { "#" };
            fill(&cfg.item_template, &[
                ("title", &escape(&item.title)),
                ("link", &escape(link)),
                ("description", &escape(&item.summary)),
                ("feed", &escape(&item.feed_title)),
                ("published", &escape(item.published.as_deref().unwrap_or(""))),
            ])
        })
        .collect();

    let html = fill(&cfg.template, &[("items", &rows), ("count", &count), ("date", &date)]);
    let subject = fill(&cfg.subject, &[("count", &count), ("date", &date)]);
    let text = items
        .iter()
        .map(|i| format!("{} — {}\n{}\n", i.feed_title, i.title, i.link))
        .collect::<Vec<_>>()
        .join("\n");

    (subject, html, text)
}

pub async fn send(cfg: &MailConfig, items: &[StoredItem]) -> Result<()> {
    if cfg.smtp_host.is_empty() || cfg.to.is_empty() || cfg.from.is_empty() {
        bail!("Configuration SMTP incomplète (serveur, expéditeur et destinataires requis).");
    }
    let (subject, html, text) = render(cfg, items);

    let mut builder = Message::builder()
        .from(cfg.from.parse::<Mailbox>().context("Adresse d'expéditeur invalide")?)
        .subject(subject);
    for to in &cfg.to {
        builder = builder.to(to.parse::<Mailbox>().with_context(|| format!("Destinataire invalide : {to}"))?);
    }
    let message = builder.multipart(MultiPart::alternative_plain_html(text, html))?;

    let mut transport = match cfg.security.as_str() {
        "tls" => AsyncSmtpTransport::<Tokio1Executor>::relay(&cfg.smtp_host)?,
        "none" => AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&cfg.smtp_host),
        _ => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&cfg.smtp_host)?,
    }
    .port(cfg.smtp_port)
    .timeout(Some(std::time::Duration::from_secs(30)));
    if !cfg.username.is_empty() {
        transport = transport.credentials(Credentials::new(cfg.username.clone(), cfg.password.clone()));
    }

    transport.build().send(message).await.context("Échec de l'envoi SMTP")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(title: &str, link: &str) -> StoredItem {
        StoredItem {
            feed: "f".into(),
            feed_title: "Feed".into(),
            title: title.into(),
            link: link.into(),
            summary: "s".into(),
            content: None,
            published: None,
            fetched_at: 0,
        }
    }

    #[test]
    fn render_escapes_values_and_neutralises_bad_links() {
        let cfg = MailConfig {
            template: "<b>{{count}}</b>{{items}}".into(),
            item_template: "<a href=\"{{link}}\">{{title}}</a>".into(),
            subject: "{{count}} new".into(),
            ..Default::default()
        };
        let (subject, html, _) = render(&cfg, &[item("<script>", "javascript:alert(1)"), item("ok", "https://x")]);
        assert_eq!(subject, "2 new");
        assert_eq!(html, "<b>2</b><a href=\"#\">&lt;script&gt;</a><a href=\"https://x\">ok</a>");
    }
}
