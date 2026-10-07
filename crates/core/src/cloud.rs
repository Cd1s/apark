//! Account sync through the master Google account.
//!
//! Like Spark, signing in with one Google account brings back every other
//! mailbox. Instead of a vendor server, the account list lives in that
//! account's hidden Google Drive app folder (`drive.appdata` scope), which
//! only Apark's OAuth client can read. An optional passphrase encrypts it.

use anyhow::{bail, Context, Result};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use rand::RngCore;
use serde::{Deserialize, Serialize};

use crate::account::Account;

const FILE_NAME: &str = "apark-accounts.json";
const FILES: &str = "https://www.googleapis.com/drive/v3/files";
const UPLOAD: &str = "https://www.googleapis.com/upload/drive/v3/files";

pub struct Snapshot {
    pub updated_at: i64,
    pub accounts: Vec<Account>,
}

#[derive(Serialize, Deserialize)]
struct Stored {
    apark: u32,
    updated_at: i64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    accounts: Vec<Account>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cipher: Option<Cipher>,
}

#[derive(Serialize, Deserialize)]
struct Cipher {
    salt: String,
    nonce: String,
    data: String,
}

fn key(passphrase: &str, salt: &[u8]) -> Result<Key> {
    let mut out = [0u8; 32];
    argon2::Argon2::default()
        .hash_password_into(passphrase.as_bytes(), salt, &mut out)
        .map_err(|e| anyhow::anyhow!("密钥派生失败: {e}"))?;
    Ok(out.into())
}

fn seal(accounts: &[Account], passphrase: &str) -> Result<Cipher> {
    let mut salt = [0u8; 16];
    let mut nonce = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut salt);
    rand::thread_rng().fill_bytes(&mut nonce);
    let data = ChaCha20Poly1305::new(&key(passphrase, &salt)?)
        .encrypt(Nonce::from_slice(&nonce), serde_json::to_vec(accounts)?.as_slice())
        .map_err(|_| anyhow::anyhow!("加密失败"))?;
    Ok(Cipher { salt: B64.encode(salt), nonce: B64.encode(nonce), data: B64.encode(data) })
}

fn open(c: &Cipher, passphrase: &str) -> Result<Vec<Account>> {
    let plain = ChaCha20Poly1305::new(&key(passphrase, &B64.decode(&c.salt)?)?)
        .decrypt(Nonce::from_slice(&B64.decode(&c.nonce)?), B64.decode(&c.data)?.as_slice())
        .map_err(|_| anyhow::anyhow!("同步密码错误，无法解密云端账号列表"))?;
    Ok(serde_json::from_slice(&plain)?)
}

async fn find(http: &reqwest::Client, token: &str) -> Result<Option<String>> {
    let resp: serde_json::Value = http
        .get(FILES)
        .bearer_auth(token)
        .query(&[
            ("spaces", "appDataFolder"),
            ("q", &format!("name = '{FILE_NAME}'")),
            ("fields", "files(id)"),
        ])
        .send()
        .await?
        .error_for_status()
        .context("读取 Google Drive 失败（是否已启用 Drive API？）")?
        .json()
        .await?;
    Ok(resp["files"][0]["id"].as_str().map(str::to_owned))
}

pub async fn pull(http: &reqwest::Client, token: &str, passphrase: Option<&str>) -> Result<Option<Snapshot>> {
    let Some(id) = find(http, token).await? else { return Ok(None) };
    let bytes = http
        .get(format!("{FILES}/{id}"))
        .bearer_auth(token)
        .query(&[("alt", "media")])
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    let stored: Stored = serde_json::from_slice(&bytes).context("云端账号列表格式错误")?;
    let accounts = match &stored.cipher {
        None => stored.accounts,
        Some(c) => match passphrase {
            Some(p) => open(c, p)?,
            None => bail!("云端账号列表已加密，请先设置同步密码（APARK_SYNC_PASSPHRASE 或 apark config set sync_passphrase）"),
        },
    };
    Ok(Some(Snapshot { updated_at: stored.updated_at, accounts }))
}

pub async fn push(http: &reqwest::Client, token: &str, snap: &Snapshot, passphrase: Option<&str>) -> Result<()> {
    let accounts: Vec<Account> = snap.accounts.iter().map(Account::portable).collect();
    let stored = match passphrase {
        Some(p) => Stored { apark: 1, updated_at: snap.updated_at, accounts: vec![], cipher: Some(seal(&accounts, p)?) },
        None => Stored { apark: 1, updated_at: snap.updated_at, accounts, cipher: None },
    };
    let id = match find(http, token).await? {
        Some(id) => id,
        None => {
            let created: serde_json::Value = http
                .post(FILES)
                .bearer_auth(token)
                .json(&serde_json::json!({ "name": FILE_NAME, "parents": ["appDataFolder"] }))
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            created["id"].as_str().context("创建云端文件失败")?.to_owned()
        }
    };
    http.patch(format!("{UPLOAD}/{id}"))
        .bearer_auth(token)
        .query(&[("uploadType", "media")])
        .header("Content-Type", "application/json")
        .body(serde_json::to_vec(&stored)?)
        .send()
        .await?
        .error_for_status()
        .context("上传账号列表失败")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::{Auth, Provider, Server};

    #[test]
    fn seal_roundtrip() {
        let a = Account {
            email: "x@y.z".into(),
            name: String::new(),
            provider: Provider::Imap,
            imap: Server::new("imap.y.z", 993),
            smtp: Server::new("smtp.y.z", 465),
            auth: Auth::Password { password: "secret".into() },
            master: false,
        };
        let c = seal(&[a], "pass").unwrap();
        assert!(!c.data.contains("secret"));
        assert_eq!(open(&c, "pass").unwrap()[0].email, "x@y.z");
        assert!(open(&c, "wrong").is_err());
    }
}
