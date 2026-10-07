use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::paths;

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Google,
    Microsoft,
    Imap,
}

impl Provider {
    pub fn label(self) -> &'static str {
        match self {
            Provider::Google => "Google",
            Provider::Microsoft => "Microsoft",
            Provider::Imap => "IMAP",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Server {
    pub host: String,
    pub port: u16,
}

impl Server {
    pub fn new(host: &str, port: u16) -> Server {
        Server { host: host.to_owned(), port }
    }

    /// Parse `host` or `host:port`.
    pub fn parse(s: &str, default_port: u16) -> Result<Server> {
        let s = s.trim();
        match s.rsplit_once(':') {
            Some((h, p)) => Ok(Server::new(h, p.parse().context("端口无效")?)),
            None if !s.is_empty() => Ok(Server::new(s, default_port)),
            None => bail!("服务器地址为空"),
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Auth {
    Password {
        password: String,
    },
    Oauth {
        refresh_token: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        access_token: Option<String>,
        #[serde(default)]
        expires_at: i64,
    },
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Account {
    pub email: String,
    #[serde(default)]
    pub name: String,
    pub provider: Provider,
    pub imap: Server,
    pub smtp: Server,
    pub auth: Auth,
    /// The Google account whose Drive holds the synced account list.
    #[serde(default)]
    pub master: bool,
}

impl Account {
    pub fn display_name(&self) -> &str {
        if self.name.is_empty() {
            &self.email
        } else {
            &self.name
        }
    }

    /// Copy without short-lived tokens, for uploading to the cloud.
    pub fn portable(&self) -> Account {
        let mut a = self.clone();
        if let Auth::Oauth { access_token, expires_at, .. } = &mut a.auth {
            *access_token = None;
            *expires_at = 0;
        }
        a.master = false;
        a
    }
}

/// Well-known IMAP/SMTP servers by mail domain; falls back to imap./smtp.<domain>.
pub fn preset(email: &str) -> (Server, Server) {
    let domain = email.rsplit('@').next().unwrap_or("").to_ascii_lowercase();
    let (i, s, sp) = match domain.as_str() {
        "gmail.com" | "googlemail.com" => ("imap.gmail.com", "smtp.gmail.com", 465),
        "outlook.com" | "hotmail.com" | "live.com" | "msn.com" => {
            ("outlook.office365.com", "smtp.office365.com", 587)
        }
        "icloud.com" | "me.com" | "mac.com" => ("imap.mail.me.com", "smtp.mail.me.com", 587),
        "qq.com" | "foxmail.com" => ("imap.qq.com", "smtp.qq.com", 465),
        "163.com" => ("imap.163.com", "smtp.163.com", 465),
        "126.com" => ("imap.126.com", "smtp.126.com", 465),
        "yeah.net" => ("imap.yeah.net", "smtp.yeah.net", 465),
        "yahoo.com" => ("imap.mail.yahoo.com", "smtp.mail.yahoo.com", 465),
        "fastmail.com" => ("imap.fastmail.com", "smtp.fastmail.com", 465),
        "aliyun.com" => ("imap.aliyun.com", "smtp.aliyun.com", 465),
        "yandex.com" | "yandex.ru" => ("imap.yandex.com", "smtp.yandex.com", 465),
        "zoho.com" => ("imap.zoho.com", "smtp.zoho.com", 465),
        _ => {
            return (
                Server::new(&format!("imap.{domain}"), 993),
                Server::new(&format!("smtp.{domain}"), 465),
            )
        }
    };
    (Server::new(i, 993), Server::new(s, sp))
}

/// All accounts on this device, stored in `<home>/accounts.json` (mode 0600).
#[derive(Serialize, Deserialize, Default, Clone)]
pub struct AccountBook {
    pub accounts: Vec<Account>,
    /// Last local change to the (non-master) account set; compared with the cloud copy.
    #[serde(default)]
    pub updated_at: i64,
}

impl AccountBook {
    pub fn load() -> Result<AccountBook> {
        let path = paths::home().join("accounts.json");
        match std::fs::read(&path) {
            Ok(b) => serde_json::from_slice(&b).context("accounts.json 损坏"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(AccountBook::default()),
            Err(e) => Err(e.into()),
        }
    }

    pub fn save(&self) -> Result<()> {
        let dir = paths::ensure_home()?;
        paths::write_private(&dir.join("accounts.json"), &serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }

    pub fn get(&self, email: &str) -> Option<&Account> {
        self.accounts.iter().find(|a| a.email.eq_ignore_ascii_case(email))
    }

    pub fn get_mut(&mut self, email: &str) -> Option<&mut Account> {
        self.accounts.iter_mut().find(|a| a.email.eq_ignore_ascii_case(email))
    }

    pub fn master(&self) -> Option<&Account> {
        self.accounts.iter().find(|a| a.master)
    }

    pub fn upsert(&mut self, acct: Account) {
        match self.get_mut(&acct.email) {
            Some(a) => *a = acct,
            None => self.accounts.push(acct),
        }
    }

    pub fn remove(&mut self, email: &str) -> bool {
        let n = self.accounts.len();
        self.accounts.retain(|a| !a.email.eq_ignore_ascii_case(email));
        n != self.accounts.len()
    }
}
