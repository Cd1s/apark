use std::path::PathBuf;

use anyhow::{Context, Result};
use lettre::message::header::ContentType;
use lettre::message::{Attachment, Mailbox, MultiPart, SinglePart};
use lettre::transport::smtp::authentication::{Credentials, Mechanism};
use lettre::transport::smtp::client::{Certificate, Tls, TlsParameters};
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use serde::{Deserialize, Serialize};

use crate::account::Account;
use crate::imap::Secret;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Outgoing {
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    pub subject: String,
    pub body: String,
    /// Optional HTML alternative to `body`.
    pub html: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Option<String>,
    pub attachments: Vec<PathBuf>,
}

fn mailbox(s: &str) -> Result<Mailbox> {
    s.trim().parse().with_context(|| format!("邮箱地址无效: {s}"))
}

pub fn build(acct: &Account, out: &Outgoing) -> Result<Message> {
    let from = Mailbox::new(
        (!acct.name.is_empty()).then(|| acct.name.clone()),
        acct.email.parse().context("发件地址无效")?,
    );
    let mut b = Message::builder().from(from).subject(&out.subject);
    for t in out.to.iter().filter(|s| !s.trim().is_empty()) {
        b = b.to(mailbox(t)?);
    }
    for t in out.cc.iter().filter(|s| !s.trim().is_empty()) {
        b = b.cc(mailbox(t)?);
    }
    for t in out.bcc.iter().filter(|s| !s.trim().is_empty()) {
        b = b.bcc(mailbox(t)?);
    }
    if let Some(id) = out.in_reply_to.as_ref().filter(|s| !s.is_empty()) {
        b = b.in_reply_to(id.clone());
    }
    if let Some(r) = out.references.as_ref().filter(|s| !s.is_empty()) {
        b = b.references(r.clone());
    }

    let text = match &out.html {
        Some(html) => MultiPart::alternative_plain_html(out.body.clone(), html.clone()),
        None => MultiPart::mixed().singlepart(SinglePart::plain(out.body.clone())),
    };
    let msg = if out.attachments.is_empty() && out.html.is_none() {
        b.header(ContentType::TEXT_PLAIN).body(out.body.clone())?
    } else {
        let mut mixed = MultiPart::mixed().multipart(text);
        for path in &out.attachments {
            let data = std::fs::read(path).with_context(|| format!("读取附件失败: {}", path.display()))?;
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "attachment".into());
            let ct = ContentType::parse("application/octet-stream").expect("static content type");
            mixed = mixed.singlepart(Attachment::new(name).body(data, ct));
        }
        b.multipart(mixed)?
    };
    Ok(msg)
}

/// Send and return the raw RFC 5322 bytes (for appending to Sent).
pub async fn send(acct: &Account, secret: &Secret, out: &Outgoing) -> Result<Vec<u8>> {
    let msg = build(acct, out)?;
    let raw = msg.formatted();
    let (creds, mechs) = match secret {
        Secret::Password(p) => (Credentials::new(acct.email.clone(), p.clone()), vec![Mechanism::Plain, Mechanism::Login]),
        Secret::Bearer(t) => (Credentials::new(acct.email.clone(), t.clone()), vec![Mechanism::Xoauth2]),
    };
    let host = acct.smtp.host.as_str();
    let mut params = TlsParameters::builder(host.to_owned());
    if let Some(pem) = crate::imap::extra_ca_pem() {
        params = params.add_root_certificate(Certificate::from_pem(&pem)?);
    }
    let params = params.build()?;
    // 465 is implicit TLS; everything else (587, 25) must upgrade with STARTTLS.
    let tls = if acct.smtp.port == 465 { Tls::Wrapper(params) } else { Tls::Required(params) };
    let transport = AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(host)
        .tls(tls)
        .port(acct.smtp.port)
        .credentials(creds)
        .authentication(mechs)
        .timeout(Some(std::time::Duration::from_secs(60)))
        .build();
    transport.send(msg).await.with_context(|| format!("SMTP 发送失败: {host}"))?;
    Ok(raw)
}
