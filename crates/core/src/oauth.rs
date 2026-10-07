//! OAuth 2.0 for installed apps: loopback redirect + PKCE.
//!
//! Headless machines use `manual`: open the printed URL on any device, then
//! paste the (failed-to-load) 127.0.0.1 redirect URL back into the terminal.

use anyhow::{anyhow, bail, Context, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64URL;
use base64::Engine as _;
use rand::RngCore;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

use crate::account::Provider;

pub struct Client {
    pub provider: Provider,
    pub id: String,
    pub secret: Option<String>,
}

impl Client {
    fn auth_url(&self) -> &'static str {
        match self.provider {
            Provider::Microsoft => "https://login.microsoftonline.com/common/oauth2/v2.0/authorize",
            _ => "https://accounts.google.com/o/oauth2/v2/auth",
        }
    }

    fn token_url(&self) -> &'static str {
        match self.provider {
            Provider::Microsoft => "https://login.microsoftonline.com/common/oauth2/v2.0/token",
            _ => "https://oauth2.googleapis.com/token",
        }
    }

    /// `master` additionally asks for the hidden Drive app folder that stores the account list.
    fn scopes(&self, master: bool) -> String {
        match self.provider {
            Provider::Microsoft => "openid email profile offline_access \
                https://outlook.office.com/IMAP.AccessAsUser.All https://outlook.office.com/SMTP.Send"
                .into(),
            _ if master => "openid email profile https://mail.google.com/ \
                https://www.googleapis.com/auth/drive.appdata"
                .into(),
            _ => "openid email profile https://mail.google.com/".into(),
        }
    }
}

#[derive(Deserialize, Debug)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
    #[serde(default)]
    id_token: Option<String>,
}

pub struct Tokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: i64,
}

pub struct Identity {
    pub email: String,
    pub name: String,
}

fn random_b64(n: usize) -> String {
    let mut buf = vec![0u8; n];
    rand::thread_rng().fill_bytes(&mut buf);
    B64URL.encode(buf)
}

fn identity(id_token: &str) -> Result<Identity> {
    let payload = id_token.split('.').nth(1).context("id_token 格式错误")?;
    let claims: serde_json::Value = serde_json::from_slice(&B64URL.decode(payload.trim_end_matches('='))?)?;
    let email = claims["email"]
        .as_str()
        .or_else(|| claims["preferred_username"].as_str())
        .context("授权结果中没有邮箱地址")?
        .to_owned();
    let name = claims["name"].as_str().unwrap_or("").to_owned();
    Ok(Identity { email, name })
}

/// Run the interactive login. `on_url` receives the authorization URL (the
/// browser is also opened automatically unless `manual`).
pub async fn login(
    http: &reqwest::Client,
    client: &Client,
    master: bool,
    manual: bool,
    on_url: &(dyn Fn(&str) + Send + Sync),
) -> Result<(Tokens, Identity)> {
    let verifier = random_b64(48);
    let challenge = B64URL.encode(Sha256::digest(verifier.as_bytes()));
    let state = random_b64(16);
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let redirect = format!("http://127.0.0.1:{}", listener.local_addr()?.port());

    let mut url = url::Url::parse(client.auth_url())?;
    url.query_pairs_mut()
        .append_pair("client_id", &client.id)
        .append_pair("redirect_uri", &redirect)
        .append_pair("response_type", "code")
        .append_pair("scope", &client.scopes(master))
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", &state);
    if client.provider == Provider::Google {
        url.query_pairs_mut()
            .append_pair("access_type", "offline")
            .append_pair("prompt", "consent select_account");
    } else {
        url.query_pairs_mut().append_pair("prompt", "select_account");
    }
    on_url(url.as_str());
    if !manual {
        let _ = webbrowser::open(url.as_str());
    }

    let params = tokio::select! {
        r = wait_redirect(&listener) => r?,
        r = read_pasted(), if manual => r?,
    };
    let get = |k: &str| params.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
    if let Some(err) = get("error") {
        bail!("授权被拒绝: {err}");
    }
    if get("state").as_deref() != Some(state.as_str()) {
        bail!("授权回调 state 不匹配");
    }
    let code = get("code").context("授权回调中没有 code")?;

    let mut form = vec![
        ("grant_type", "authorization_code".to_owned()),
        ("code", code),
        ("redirect_uri", redirect),
        ("client_id", client.id.clone()),
        ("code_verifier", verifier),
    ];
    if let Some(s) = &client.secret {
        form.push(("client_secret", s.clone()));
    }
    let tr = token_request(http, client, &form).await?;
    let ident = identity(tr.id_token.as_deref().context("授权结果中没有 id_token")?)?;
    Ok((tokens(tr), ident))
}

pub async fn refresh(http: &reqwest::Client, client: &Client, refresh_token: &str) -> Result<Tokens> {
    let mut form = vec![
        ("grant_type", "refresh_token".to_owned()),
        ("refresh_token", refresh_token.to_owned()),
        ("client_id", client.id.clone()),
    ];
    if let Some(s) = &client.secret {
        form.push(("client_secret", s.clone()));
    }
    Ok(tokens(token_request(http, client, &form).await?))
}

fn tokens(tr: TokenResponse) -> Tokens {
    Tokens {
        access_token: tr.access_token,
        refresh_token: tr.refresh_token,
        expires_at: crate::now() + tr.expires_in.unwrap_or(3600),
    }
}

async fn token_request(http: &reqwest::Client, client: &Client, form: &[(&str, String)]) -> Result<TokenResponse> {
    let resp = http.post(client.token_url()).form(form).send().await?;
    let status = resp.status();
    let text = resp.text().await?;
    if !status.is_success() {
        bail!("令牌请求失败 ({status}): {text}");
    }
    serde_json::from_str(&text).context("令牌响应格式错误")
}

fn query_params(target: &str) -> Vec<(String, String)> {
    let q = target.split_once('?').map(|(_, q)| q).unwrap_or("");
    url::form_urlencoded::parse(q.as_bytes()).into_owned().collect()
}

const DONE_PAGE: &str = "<!doctype html><meta charset=utf-8><title>Apark</title>\
<body style=\"font:16px -apple-system,system-ui,sans-serif;text-align:center;padding-top:80px\">\
<h2>Apark 登录完成</h2><p>可以关闭这个页面，回到 Apark。</p>";

async fn wait_redirect(listener: &TcpListener) -> Result<Vec<(String, String)>> {
    loop {
        let (mut sock, _) = listener.accept().await?;
        let mut reader = BufReader::new(&mut sock);
        let mut line = String::new();
        reader.read_line(&mut line).await?;
        // Drain the rest of the request head so the browser sees a clean response.
        let mut buf = [0u8; 4096];
        let _ = tokio::time::timeout(std::time::Duration::from_millis(50), reader.read(&mut buf)).await;
        let target = line.split_whitespace().nth(1).unwrap_or("/");
        let params = query_params(target);
        let ours = params.iter().any(|(k, _)| k == "code" || k == "error");
        let (status, body) = if ours { ("200 OK", DONE_PAGE) } else { ("404 Not Found", "") };
        let resp = format!(
            "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = sock.write_all(resp.as_bytes()).await;
        if ours {
            return Ok(params);
        }
    }
}

async fn read_pasted() -> Result<Vec<(String, String)>> {
    eprintln!("\n在浏览器完成授权后，地址栏会跳到 http://127.0.0.1:…（页面打不开没关系），把完整地址粘贴到这里：");
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    while let Some(line) = lines.next_line().await? {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let params = query_params(line);
        if params.iter().any(|(k, _)| k == "code" || k == "error") {
            return Ok(params);
        }
        eprintln!("没找到 code 参数，请粘贴完整的跳转地址：");
    }
    Err(anyhow!("输入已结束，登录取消"))
}
