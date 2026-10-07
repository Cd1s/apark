//! Account-list sync: sign in once on a new device and every mailbox comes back.
//!
//! The list is a single small JSON blob. Where it lives is pluggable:
//! - `google`: the master Google account's hidden Drive app folder (`drive.appdata`);
//! - `server`: a self-hosted `apark server` (end-to-end encrypted, the server
//!   only ever sees ciphertext under an id derived from user + password);
//! - `webdav`: any WebDAV URL (Nextcloud, 坚果云, NAS…);
//! - `file`: a path inside a folder synced by iCloud Drive / Dropbox / Syncthing.
//!
//! Everything except `google` requires a passphrase, so the blob is always encrypted.

use anyhow::{bail, Context, Result};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::account::Account;

const FILE_NAME: &str = "apark-accounts.json";
const DRIVE_FILES: &str = "https://www.googleapis.com/drive/v3/files";
const DRIVE_UPLOAD: &str = "https://www.googleapis.com/upload/drive/v3/files";

/// Where the account list is stored (persisted in config.json).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum SyncTarget {
    Google,
    Server {
        url: String,
        user: String,
        password: String,
        /// Shared access token, if the server was started with one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        token: Option<String>,
    },
    Webdav {
        url: String,
        user: String,
        password: String,
    },
    File {
        path: String,
    },
}

impl SyncTarget {
    pub fn kind(&self) -> &'static str {
        match self {
            SyncTarget::Google => "google",
            SyncTarget::Server { .. } => "server",
            SyncTarget::Webdav { .. } => "webdav",
            SyncTarget::File { .. } => "file",
        }
    }

    /// Human-readable location without secrets.
    pub fn describe(&self) -> String {
        match self {
            SyncTarget::Google => "Google Drive".into(),
            SyncTarget::Server { url, user, .. } => format!("{user} @ {url}"),
            SyncTarget::Webdav { url, user, .. } => format!("{user} @ {url}"),
            SyncTarget::File { path } => path.clone(),
        }
    }
}

/// A resolved backend, ready for I/O.
pub enum Backend {
    Google { token: String },
    Server { url: String, id: String, token: Option<String> },
    Webdav { url: String, user: String, password: String },
    File { path: std::path::PathBuf },
}

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

fn argon(passphrase: &str, salt: &[u8]) -> Result<[u8; 32]> {
    let mut out = [0u8; 32];
    argon2::Argon2::default()
        .hash_password_into(passphrase.as_bytes(), salt, &mut out)
        .map_err(|e| anyhow::anyhow!("密钥派生失败: {e}"))?;
    Ok(out)
}

/// Blob id on a sync server: slow hash of user + password, so the server can
/// neither read the data nor cheaply guess whose it is.
pub fn server_blob_id(user: &str, password: &str) -> Result<String> {
    let salt = Sha256::digest(format!("apark-sync-id:{}", user.trim().to_lowercase()));
    let id = argon(password, &salt[..16])?;
    Ok(id.iter().map(|b| format!("{b:02x}")).collect())
}

fn seal(accounts: &[Account], passphrase: &str) -> Result<Cipher> {
    let mut salt = [0u8; 16];
    let mut nonce = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut salt);
    rand::thread_rng().fill_bytes(&mut nonce);
    let key: Key = argon(passphrase, &salt)?.into();
    let data = ChaCha20Poly1305::new(&key)
        .encrypt(Nonce::from_slice(&nonce), serde_json::to_vec(accounts)?.as_slice())
        .map_err(|_| anyhow::anyhow!("加密失败"))?;
    Ok(Cipher { salt: B64.encode(salt), nonce: B64.encode(nonce), data: B64.encode(data) })
}

fn open(c: &Cipher, passphrase: &str) -> Result<Vec<Account>> {
    let key: Key = argon(passphrase, &B64.decode(&c.salt)?)?.into();
    let plain = ChaCha20Poly1305::new(&key)
        .decrypt(Nonce::from_slice(&B64.decode(&c.nonce)?), B64.decode(&c.data)?.as_slice())
        .map_err(|_| anyhow::anyhow!("同步密码错误，无法解密账号列表"))?;
    Ok(serde_json::from_slice(&plain)?)
}

pub fn encode(snap: &Snapshot, passphrase: Option<&str>) -> Result<Vec<u8>> {
    let accounts: Vec<Account> = snap.accounts.iter().map(Account::portable).collect();
    let stored = match passphrase {
        Some(p) => Stored { apark: 1, updated_at: snap.updated_at, accounts: vec![], cipher: Some(seal(&accounts, p)?) },
        None => Stored { apark: 1, updated_at: snap.updated_at, accounts, cipher: None },
    };
    Ok(serde_json::to_vec(&stored)?)
}

pub fn decode(bytes: &[u8], passphrase: Option<&str>) -> Result<Snapshot> {
    let stored: Stored = serde_json::from_slice(bytes).context("云端账号列表格式错误")?;
    let accounts = match &stored.cipher {
        None => stored.accounts,
        Some(c) => match passphrase {
            Some(p) => open(c, p)?,
            None => bail!("账号列表已加密，请先设置同步密码"),
        },
    };
    Ok(Snapshot { updated_at: stored.updated_at, accounts })
}

// ---- transports -----------------------------------------------------------

async fn drive_find(http: &reqwest::Client, token: &str) -> Result<Option<String>> {
    let resp: serde_json::Value = http
        .get(DRIVE_FILES)
        .bearer_auth(token)
        .query(&[("spaces", "appDataFolder"), ("q", &format!("name = '{FILE_NAME}'")), ("fields", "files(id)")])
        .send()
        .await?
        .error_for_status()
        .context("读取 Google Drive 失败（是否已启用 Drive API？）")?
        .json()
        .await?;
    Ok(resp["files"][0]["id"].as_str().map(str::to_owned))
}

fn blob_url(url: &str, id: &str) -> String {
    format!("{}/v1/blob/{id}", url.trim_end_matches('/'))
}

pub async fn read(http: &reqwest::Client, b: &Backend) -> Result<Option<Vec<u8>>> {
    let resp = match b {
        Backend::Google { token } => {
            let Some(id) = drive_find(http, token).await? else { return Ok(None) };
            http.get(format!("{DRIVE_FILES}/{id}")).bearer_auth(token).query(&[("alt", "media")]).send().await?
        }
        Backend::Server { url, id, token } => {
            let mut req = http.get(blob_url(url, id));
            if let Some(t) = token {
                req = req.bearer_auth(t);
            }
            req.send().await.context("连接同步服务器失败")?
        }
        Backend::Webdav { url, user, password } => {
            http.get(url).basic_auth(user, Some(password)).send().await.context("连接 WebDAV 失败")?
        }
        Backend::File { path } => {
            return match std::fs::read(path) {
                Ok(b) => Ok(Some(b)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(e).with_context(|| format!("读取 {}", path.display())),
            }
        }
    };
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let resp = resp.error_for_status().context("读取账号列表失败")?;
    Ok(Some(resp.bytes().await?.to_vec()))
}

pub async fn write(http: &reqwest::Client, b: &Backend, bytes: Vec<u8>) -> Result<()> {
    match b {
        Backend::Google { token } => {
            let id = match drive_find(http, token).await? {
                Some(id) => id,
                None => {
                    let created: serde_json::Value = http
                        .post(DRIVE_FILES)
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
            http.patch(format!("{DRIVE_UPLOAD}/{id}"))
                .bearer_auth(token)
                .query(&[("uploadType", "media")])
                .header("Content-Type", "application/json")
                .body(bytes)
                .send()
                .await?
                .error_for_status()
                .context("上传账号列表失败")?;
        }
        Backend::Server { url, id, token } => {
            let mut req = http.put(blob_url(url, id)).body(bytes);
            if let Some(t) = token {
                req = req.bearer_auth(t);
            }
            req.send().await.context("连接同步服务器失败")?.error_for_status().context("上传账号列表失败")?;
        }
        Backend::Webdav { url, user, password } => {
            let put = || http.put(url).basic_auth(user, Some(password)).body(bytes.clone());
            let resp = put().send().await.context("连接 WebDAV 失败")?;
            if matches!(resp.status().as_u16(), 404 | 409) {
                // Parent collection missing: create it, then retry once.
                if let Some((parent, _)) = url.trim_end_matches('/').rsplit_once('/') {
                    let mkcol = reqwest::Method::from_bytes(b"MKCOL").expect("static method");
                    let _ = http.request(mkcol, format!("{parent}/")).basic_auth(user, Some(password)).send().await;
                }
                put().send().await?.error_for_status().context("上传账号列表失败")?;
            } else {
                resp.error_for_status().context("上传账号列表失败")?;
            }
        }
        Backend::File { path } => {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            crate::paths::write_private(path, &bytes)?;
        }
    }
    Ok(())
}

pub async fn pull(http: &reqwest::Client, b: &Backend, passphrase: Option<&str>) -> Result<Option<Snapshot>> {
    read(http, b).await?.map(|bytes| decode(&bytes, passphrase)).transpose()
}

pub async fn push(http: &reqwest::Client, b: &Backend, snap: &Snapshot, passphrase: Option<&str>) -> Result<()> {
    write(http, b, encode(snap, passphrase)?).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::{Auth, Provider, Server};

    fn acct() -> Account {
        Account {
            email: "x@y.z".into(),
            name: String::new(),
            provider: Provider::Imap,
            imap: Server::new("imap.y.z", 993),
            smtp: Server::new("smtp.y.z", 465),
            auth: Auth::Password { password: "secret".into() },
            master: false,
        }
    }

    #[test]
    fn seal_roundtrip() {
        let c = seal(&[acct()], "pass").unwrap();
        assert!(!c.data.contains("secret"));
        assert_eq!(open(&c, "pass").unwrap()[0].email, "x@y.z");
        assert!(open(&c, "wrong").is_err());
    }

    #[test]
    fn encode_decode() {
        let snap = Snapshot { updated_at: 7, accounts: vec![acct()] };
        let enc = encode(&snap, Some("pw")).unwrap();
        assert!(!String::from_utf8_lossy(&enc).contains("secret"));
        assert_eq!(decode(&enc, Some("pw")).unwrap().updated_at, 7);
        assert!(decode(&enc, None).is_err());
        let id1 = server_blob_id("Me", "pw").unwrap();
        assert_eq!(id1, server_blob_id("me", "pw").unwrap());
        assert_ne!(id1, server_blob_id("me", "pw2").unwrap());
        assert_eq!(id1.len(), 64);
    }
}
