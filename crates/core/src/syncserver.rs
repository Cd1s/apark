//! `apark server`: a tiny self-hosted store for encrypted account lists.
//!
//!   GET /health              -> 200 "ok"
//!   GET /v1/blob/<64 hex>    -> 200 blob | 404
//!   PUT /v1/blob/<64 hex>    -> 204       (body ≤ 4 MiB)
//!
//! Blobs are encrypted on the client and addressed by an id derived from the
//! user's name and password, so the server never learns either. An optional
//! bearer token keeps strangers from storing data. Put it behind HTTPS
//! (Caddy, nginx, Cloudflare Tunnel) when exposed to the internet.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const MAX_BODY: usize = 4 << 20;
const MAX_HEAD: usize = 16 << 10;

pub async fn serve(listen: &str, dir: PathBuf, token: Option<String>) -> Result<()> {
    std::fs::create_dir_all(&dir).with_context(|| format!("创建 {}", dir.display()))?;
    let listener = TcpListener::bind(listen).await.with_context(|| format!("监听 {listen} 失败"))?;
    eprintln!(
        "apark server 已启动：http://{}  数据目录 {}  {}",
        listener.local_addr()?,
        dir.display(),
        if token.is_some() { "（需要访问令牌）" } else { "（未设置访问令牌，任何人都能存储数据）" }
    );
    loop {
        let (sock, _) = listener.accept().await?;
        let (dir, token) = (dir.clone(), token.clone());
        tokio::spawn(async move {
            let _ = tokio::time::timeout(Duration::from_secs(30), handle(sock, &dir, token.as_deref())).await;
        });
    }
}

async fn respond(sock: &mut TcpStream, status: &str, body: &[u8]) -> Result<()> {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    sock.write_all(head.as_bytes()).await?;
    sock.write_all(body).await?;
    Ok(())
}

fn valid_id(id: &str) -> bool {
    id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Constant-time comparison for the access token.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

async fn handle(mut sock: TcpStream, dir: &Path, token: Option<&str>) -> Result<()> {
    let mut buf = Vec::with_capacity(4096);
    let head_end = loop {
        let mut chunk = [0u8; 4096];
        let n = sock.read(&mut chunk).await?;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
        if buf.len() > MAX_HEAD {
            return respond(&mut sock, "431 Request Header Fields Too Large", b"").await;
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let mut lines = head.lines();
    let mut first = lines.next().unwrap_or("").split_whitespace();
    let (method, path) = (first.next().unwrap_or(""), first.next().unwrap_or(""));
    let mut length = 0usize;
    let mut auth = String::new();
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            match k.trim().to_ascii_lowercase().as_str() {
                "content-length" => length = v.trim().parse().unwrap_or(usize::MAX),
                "authorization" => auth = v.trim().to_owned(),
                _ => {}
            }
        }
    }

    if path == "/health" {
        return respond(&mut sock, "200 OK", b"ok").await;
    }
    let Some(id) = path.strip_prefix("/v1/blob/").filter(|id| valid_id(id)) else {
        return respond(&mut sock, "404 Not Found", b"").await;
    };
    if let Some(t) = token {
        let given = auth.strip_prefix("Bearer ").or_else(|| auth.strip_prefix("bearer ")).unwrap_or("");
        if !same(given, t) {
            return respond(&mut sock, "401 Unauthorized", b"").await;
        }
    }
    let file = dir.join(format!("{}.blob", id.to_ascii_lowercase()));
    match method {
        "GET" => match tokio::fs::read(&file).await {
            Ok(data) => respond(&mut sock, "200 OK", &data).await,
            Err(_) => respond(&mut sock, "404 Not Found", b"").await,
        },
        "PUT" => {
            if length > MAX_BODY {
                return respond(&mut sock, "413 Payload Too Large", b"").await;
            }
            let mut body = buf[head_end..].to_vec();
            while body.len() < length {
                let mut chunk = vec![0u8; (length - body.len()).min(64 << 10)];
                let n = sock.read(&mut chunk).await?;
                if n == 0 {
                    break;
                }
                body.extend_from_slice(&chunk[..n]);
            }
            body.truncate(length);
            let tmp = file.with_extension("tmp");
            tokio::fs::write(&tmp, &body).await?;
            tokio::fs::rename(&tmp, &file).await?;
            respond(&mut sock, "204 No Content", b"").await
        }
        _ => respond(&mut sock, "405 Method Not Allowed", b"").await,
    }
}
