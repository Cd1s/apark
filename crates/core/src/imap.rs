//! IMAP over implicit TLS, plus MIME parsing into store rows.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use async_imap::types::Fetch;
use futures::TryStreamExt;
use mail_parser::{Address, HeaderValue, MessageParser, MimeHeaders, PartType};
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;
use tokio_rustls::TlsConnector;

use crate::account::Account;
use crate::categorize::categorize;
use crate::store::{Attachment, Body, Folder, Header, NewMsg, Store};

pub type Session = async_imap::Session<TlsStream<TcpStream>>;

pub enum Secret {
    Password(String),
    Bearer(String),
}

struct XOAuth2(String);

impl async_imap::Authenticator for XOAuth2 {
    type Response = String;
    fn process(&mut self, _challenge: &[u8]) -> String {
        self.0.clone()
    }
}

/// Extra trusted root certificates (PEM) for self-hosted servers with a private CA.
pub fn extra_ca_pem() -> Option<Vec<u8>> {
    std::fs::read(std::env::var_os("APARK_EXTRA_CA")?).ok()
}

pub fn tls() -> TlsConnector {
    use rustls::pki_types::{pem::PemObject, CertificateDer};
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    if let Some(pem) = extra_ca_pem() {
        for cert in CertificateDer::pem_slice_iter(&pem).flatten() {
            let _ = roots.add(cert);
        }
    }
    let cfg = rustls::ClientConfig::builder().with_root_certificates(roots).with_no_client_auth();
    TlsConnector::from(Arc::new(cfg))
}

pub async fn connect(acct: &Account, secret: &Secret) -> Result<Session> {
    let host = acct.imap.host.as_str();
    let tcp = tokio::time::timeout(Duration::from_secs(20), TcpStream::connect((host, acct.imap.port)))
        .await
        .map_err(|_| anyhow!("连接 {host} 超时"))??;
    let name = rustls::pki_types::ServerName::try_from(host.to_owned())?;
    let stream = tls().connect(name, tcp).await.with_context(|| format!("TLS 握手失败: {host}"))?;
    let mut client = async_imap::Client::new(stream);
    client.read_response().await?.context("服务器没有发送问候")?;
    let session = match secret {
        Secret::Password(p) => client.login(&acct.email, p).await.map_err(|(e, _)| e),
        Secret::Bearer(t) => client
            .authenticate("XOAUTH2", XOAuth2(format!("user={}\x01auth=Bearer {t}\x01\x01", acct.email)))
            .await
            .map_err(|(e, _)| e),
    }
    .with_context(|| format!("IMAP 登录失败: {}", acct.email))?;
    Ok(session)
}

// ---- MIME -------------------------------------------------------------

fn addr_list(a: Option<&Address>) -> String {
    let fmt = |name: Option<&str>, addr: Option<&str>| match (name, addr) {
        (Some(n), Some(a)) if !n.is_empty() => format!("{n} <{a}>"),
        (_, Some(a)) => a.to_owned(),
        _ => String::new(),
    };
    let v: Vec<String> = match a {
        Some(Address::List(l)) => l.iter().map(|x| fmt(x.name(), x.address())).collect(),
        Some(Address::Group(g)) => g
            .iter()
            .flat_map(|g| g.addresses.iter().map(|x| fmt(x.name(), x.address())))
            .collect(),
        None => vec![],
    };
    v.into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(", ")
}

fn ids(v: &HeaderValue) -> String {
    match v {
        HeaderValue::Text(t) => format!("<{t}>"),
        HeaderValue::TextList(l) => l.iter().map(|t| format!("<{t}>")).collect::<Vec<_>>().join(" "),
        _ => String::new(),
    }
}

pub fn parse_header(raw: &[u8]) -> Header {
    let Some(msg) = MessageParser::default().parse(raw) else {
        return Header { category: "people".into(), ..Default::default() };
    };
    let from = msg.from().and_then(|a| a.first());
    let from_addr = from.and_then(|a| a.address()).unwrap_or("").to_owned();
    Header {
        message_id: msg.message_id().map(|m| format!("<{m}>")).unwrap_or_default(),
        in_reply_to: ids(msg.in_reply_to()),
        references: ids(msg.references()),
        subject: msg.subject().unwrap_or("").trim().to_owned(),
        from_name: from.and_then(|a| a.name()).unwrap_or("").trim().to_owned(),
        to: addr_list(msg.to()),
        cc: addr_list(msg.cc()),
        date: msg.date().map(|d| d.to_timestamp()).unwrap_or(0),
        category: categorize(&msg, &from_addr).to_owned(),
        from_addr,
    }
}

pub fn parse_body(raw: &[u8]) -> Body {
    let Some(msg) = MessageParser::default().parse(raw) else { return Body::default() };
    let has_html = msg
        .html_body
        .iter()
        .any(|i| matches!(msg.parts.get(*i as usize).map(|p| &p.body), Some(PartType::Html(_))));
    Body {
        text: msg.body_text(0).map(|t| t.into_owned()).unwrap_or_default(),
        html: if has_html { msg.body_html(0).map(|h| h.into_owned()) } else { None },
        attachments: msg
            .attachments()
            .map(|p| Attachment { name: p.attachment_name().unwrap_or("attachment").to_owned(), size: p.len() })
            .collect(),
    }
}

/// Save attachment `index` (or all when None) from a raw message into `dir`.
pub fn extract_attachments(raw: &[u8], index: Option<usize>, dir: &std::path::Path) -> Result<Vec<std::path::PathBuf>> {
    let msg = MessageParser::default().parse(raw).context("无法解析邮件")?;
    std::fs::create_dir_all(dir)?;
    let mut out = vec![];
    for (i, p) in msg.attachments().enumerate() {
        if index.is_some_and(|n| n != i) {
            continue;
        }
        let name = p.attachment_name().unwrap_or("attachment");
        let safe: String = name.chars().map(|c| if "/\\:*?\"<>|".contains(c) { '_' } else { c }).collect();
        let mut path = dir.join(&safe);
        let mut n = 1;
        while path.exists() {
            path = dir.join(format!("{n}-{safe}"));
            n += 1;
        }
        std::fs::write(&path, p.contents())?;
        out.push(path);
    }
    Ok(out)
}

// ---- folders ----------------------------------------------------------

/// Decode IMAP modified UTF-7 (RFC 3501 §5.1.3): "&XfJSoGYfaAc-" -> "已加星标".
pub fn decode_mutf7(s: &str) -> String {
    use base64::Engine as _;
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let Some(j) = after.find('-') else {
            out.push_str(&rest[i..]);
            return out;
        };
        let enc = &after[..j];
        if enc.is_empty() {
            out.push('&');
        } else {
            let decoded = base64::engine::general_purpose::STANDARD_NO_PAD
                .decode(enc.replace(',', "/"))
                .ok()
                .filter(|b| b.len() % 2 == 0)
                .map(|b| String::from_utf16_lossy(&b.chunks(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect::<Vec<_>>()));
            match decoded {
                Some(text) => out.push_str(&text),
                None => {
                    out.push('&');
                    out.push_str(enc);
                    out.push('-');
                }
            }
        }
        rest = &after[j + 1..];
    }
    out.push_str(rest);
    out
}

fn role_of(name: &str, attrs: &str) -> &'static str {
    if name.eq_ignore_ascii_case("INBOX") {
        return "inbox";
    }
    for (needle, role) in [
        ("\\sent", "sent"),
        ("\\trash", "trash"),
        ("\\junk", "junk"),
        ("\\drafts", "drafts"),
        ("\\archive", "archive"),
        ("\\all", "all"),
        ("\\flagged", "flagged"),
        ("sent", "sent"),
        ("trash", "trash"),
        ("junk", "junk"),
        ("drafts", "drafts"),
        ("archive", "archive"),
        ("all", "all"),
        ("flagged", "flagged"),
    ] {
        if attrs.contains(needle) {
            return role;
        }
    }
    let n = name.to_lowercase();
    for (needles, role) in [
        (&["sent", "已发送"][..], "sent"),
        (&["trash", "deleted", "已删除"][..], "trash"),
        (&["junk", "spam", "垃圾"][..], "junk"),
        (&["draft", "草稿"][..], "drafts"),
        (&["archive", "归档"][..], "archive"),
    ] {
        if needles.iter().any(|x| n.contains(x)) {
            return role;
        }
    }
    ""
}

pub async fn list_folders(s: &mut Session) -> Result<Vec<Folder>> {
    let names: Vec<_> = s.list(Some(""), Some("*")).await?.try_collect().await?;
    let mut out = vec![];
    for n in &names {
        let attrs = format!("{:?}", n.attributes()).to_lowercase();
        if attrs.contains("noselect") || attrs.contains("nonexistent") {
            continue;
        }
        let role = role_of(&decode_mutf7(n.name()), &attrs);
        out.push(Folder::new(n.name(), role));
    }
    Ok(out)
}

// ---- sync -------------------------------------------------------------

fn flags_of(f: &Fetch) -> (bool, bool) {
    let mut seen = false;
    let mut flagged = false;
    for fl in f.flags() {
        match fl {
            async_imap::types::Flag::Seen => seen = true,
            async_imap::types::Flag::Flagged => flagged = true,
            _ => {}
        }
    }
    (seen, flagged)
}

const HEADER_QUERY: &str = "(UID FLAGS RFC822.SIZE BODY.PEEK[HEADER])";

/// Incremental sync of one folder. Returns the number of new messages.
pub async fn sync_folder(
    s: &mut Session,
    store: &Store,
    account: &str,
    folder: &str,
    initial_limit: u32,
    prefetch_bytes: u32,
) -> Result<usize> {
    let mb = s.select(folder).await.with_context(|| format!("打开文件夹失败: {folder}"))?;
    let uidvalidity = mb.uid_validity.unwrap_or(0);
    let (old_validity, mut last_uid) = store.folder_state(account, folder)?;
    if old_validity != uidvalidity {
        store.reset_folder(account, folder)?;
        last_uid = 0;
    }
    if mb.exists == 0 {
        store.reset_folder(account, folder)?;
        store.set_folder_state(account, folder, uidvalidity, 0)?;
        return Ok(0);
    }

    // 1. New messages: headers + flags only, so the list appears fast.
    let fetched: Vec<Fetch> = if last_uid == 0 {
        let start = mb.exists.saturating_sub(initial_limit.max(1)) + 1;
        s.fetch(format!("{start}:*"), HEADER_QUERY).await?.try_collect().await?
    } else {
        s.uid_fetch(format!("{}:*", last_uid + 1), HEADER_QUERY).await?.try_collect().await?
    };
    let mut new = Vec::new();
    let mut max_uid = last_uid;
    for f in &fetched {
        let Some(uid) = f.uid else { continue };
        if uid <= last_uid {
            continue;
        }
        let (seen, flagged) = flags_of(f);
        new.push(NewMsg { uid, seen, flagged, size: f.size.unwrap_or(0), header: parse_header(f.header().unwrap_or(b"")) });
        max_uid = max_uid.max(uid);
    }
    store.insert_headers(account, folder, &new)?;

    // 2. Flag changes and deletions for what we already have.
    if last_uid > 0 {
        let known = store.known_uids(account, folder)?;
        if let Some(&min) = known.first() {
            let flags: Vec<Fetch> = s.uid_fetch(format!("{min}:{last_uid}"), "(UID FLAGS)").await?.try_collect().await?;
            let mut present = HashSet::new();
            let mut updates = Vec::with_capacity(flags.len());
            for f in &flags {
                if let Some(uid) = f.uid {
                    present.insert(uid);
                    let (seen, flagged) = flags_of(f);
                    updates.push((uid, seen, flagged));
                }
            }
            store.update_flags(account, folder, &updates)?;
            let gone: Vec<u32> = known.into_iter().filter(|u| *u <= last_uid && !present.contains(u)).collect();
            store.delete_uids(account, folder, &gone)?;
        }
    }
    store.set_folder_state(account, folder, uidvalidity, max_uid)?;

    // 3. Prefetch small bodies (newest first) for instant, offline reading.
    let need = store.need_body(account, folder, prefetch_bytes, 200)?;
    for chunk in need.chunks(25) {
        let set = chunk.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
        let bodies: Vec<Fetch> = s.uid_fetch(set, "(UID BODY.PEEK[])").await?.try_collect().await?;
        for f in &bodies {
            if let (Some(uid), Some(raw)) = (f.uid, f.body()) {
                store.set_body(account, folder, uid, &parse_body(raw))?;
            }
        }
    }
    Ok(new.len())
}

pub async fn fetch_raw(s: &mut Session, folder: &str, uid: u32) -> Result<Vec<u8>> {
    s.select(folder).await?;
    let v: Vec<Fetch> = s.uid_fetch(uid.to_string(), "(UID BODY.PEEK[])").await?.try_collect().await?;
    v.iter()
        .find_map(|f| f.body().map(<[u8]>::to_vec))
        .ok_or_else(|| anyhow!("服务器上找不到这封邮件（可能已被移动或删除）"))
}

pub async fn set_flag(s: &mut Session, folder: &str, uid: u32, flag: &str, on: bool) -> Result<()> {
    s.select(folder).await?;
    let op = if on { "+FLAGS.SILENT" } else { "-FLAGS.SILENT" };
    let _: Vec<Fetch> = s.uid_store(uid.to_string(), format!("{op} ({flag})")).await?.try_collect().await?;
    Ok(())
}

pub async fn move_to(s: &mut Session, folder: &str, uid: u32, dest: &str) -> Result<()> {
    s.select(folder).await?;
    if s.uid_mv(uid.to_string(), dest).await.is_ok() {
        return Ok(());
    }
    // Servers without MOVE: copy, flag deleted, expunge.
    s.uid_copy(uid.to_string(), dest).await?;
    let _: Vec<Fetch> = s.uid_store(uid.to_string(), "+FLAGS.SILENT (\\Deleted)").await?.try_collect().await?;
    let _: Vec<_> = s.expunge().await?.try_collect().await?;
    Ok(())
}

pub async fn append(s: &mut Session, folder: &str, raw: &[u8]) -> Result<()> {
    s.append(folder, Some("(\\Seen)"), None, raw).await?;
    Ok(())
}
