//! High-level operations shared by the desktop app and the CLI.
//!
//! Mutations are applied to the local store first and then pushed to the
//! server, so the UI never waits on the network.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

use anyhow::{anyhow, bail, Context, Result};

use crate::account::{preset, Account, AccountBook, Auth, Provider, Server};
use crate::config::Config;
use crate::imap::{self, Secret};
use crate::store::{Body, Folder, ListQuery, MsgRow, Store};
use crate::cloud::SyncTarget;
use crate::{cloud, oauth, paths, smtp, Outgoing};

pub struct LoginOpts {
    /// Don't open a browser; read the redirect URL from stdin (headless servers).
    pub manual: bool,
    pub on_url: Arc<dyn Fn(&str) + Send + Sync>,
}

impl Default for LoginOpts {
    fn default() -> Self {
        LoginOpts { manual: false, on_url: Arc::new(|_| {}) }
    }
}

#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct CloudResult {
    /// "joined", "pushed", "pulled", "unchanged" or "no-sync".
    pub action: &'static str,
    pub added: Vec<String>,
    pub removed: Vec<String>,
}

pub struct Engine {
    cfg: RwLock<Config>,
    pub store: Store,
    book: Mutex<AccountBook>,
    http: reqwest::Client,
    sync_lock: tokio::sync::Mutex<()>,
}

impl Engine {
    pub fn open() -> Result<Arc<Engine>> {
        crate::init_crypto();
        let home = paths::ensure_home()?;
        Ok(Arc::new(Engine {
            cfg: RwLock::new(Config::load()),
            store: Store::open(&home.join("apark.db"))?,
            book: Mutex::new(AccountBook::load()?),
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(60))
                .user_agent(concat!("Apark/", env!("CARGO_PKG_VERSION")))
                .build()?,
            sync_lock: tokio::sync::Mutex::new(()),
        }))
    }

    // ---- settings & accounts --------------------------------------------

    pub fn config(&self) -> Config {
        self.cfg.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn set_config(&self, cfg: Config) -> Result<()> {
        cfg.save()?;
        *self.cfg.write().unwrap_or_else(|e| e.into_inner()) = cfg;
        Ok(())
    }

    fn book(&self) -> std::sync::MutexGuard<'_, AccountBook> {
        self.book.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn accounts(&self) -> Vec<Account> {
        self.book().accounts.clone()
    }

    pub fn account(&self, email: &str) -> Result<Account> {
        self.book().get(email).cloned().ok_or_else(|| anyhow!("没有这个账号: {email}"))
    }

    pub fn has_master(&self) -> bool {
        self.book().master().is_some()
    }

    fn oauth_client(&self, provider: Provider) -> Result<oauth::Client> {
        let cfg = self.config();
        match provider {
            Provider::Google => {
                let (id, secret) = cfg.google_client().context(
                    "未配置 Google OAuth 客户端：设置 APARK_GOOGLE_CLIENT_ID / APARK_GOOGLE_CLIENT_SECRET，\
                     或 `apark config set google_client_id …`（见 README）",
                )?;
                Ok(oauth::Client { provider, id, secret })
            }
            Provider::Microsoft => {
                let id = cfg.microsoft_client().context(
                    "未配置 Microsoft OAuth 客户端：设置 APARK_MS_CLIENT_ID 或 `apark config set microsoft_client_id …`",
                )?;
                Ok(oauth::Client { provider, id, secret: None })
            }
            Provider::Imap => bail!("IMAP 账号不使用 OAuth"),
        }
    }

    /// Credentials for IMAP/SMTP, refreshing the OAuth access token when needed.
    pub async fn secret(&self, email: &str) -> Result<Secret> {
        let acct = self.account(email)?;
        let refresh_token = match &acct.auth {
            Auth::Password { password } => return Ok(Secret::Password(password.clone())),
            Auth::Oauth { access_token: Some(t), expires_at, .. } if *expires_at > crate::now() + 120 => {
                return Ok(Secret::Bearer(t.clone()))
            }
            Auth::Oauth { refresh_token, .. } => refresh_token.clone(),
        };
        let client = self.oauth_client(acct.provider)?;
        let tokens = oauth::refresh(&self.http, &client, &refresh_token)
            .await
            .with_context(|| format!("{email} 授权已失效，请重新添加该账号"))?;
        let mut book = self.book();
        if let Some(Account { auth: Auth::Oauth { refresh_token, access_token, expires_at }, .. }) = book.get_mut(email) {
            *access_token = Some(tokens.access_token.clone());
            *expires_at = tokens.expires_at;
            if let Some(r) = tokens.refresh_token {
                *refresh_token = r;
            }
        }
        book.save()?;
        Ok(Secret::Bearer(tokens.access_token))
    }

    fn oauth_account(provider: Provider, tokens: oauth::Tokens, ident: oauth::Identity, master: bool) -> Result<Account> {
        let (imap, smtp) = match provider {
            Provider::Google => (Server::new("imap.gmail.com", 993), Server::new("smtp.gmail.com", 465)),
            _ => (Server::new("outlook.office365.com", 993), Server::new("smtp.office365.com", 587)),
        };
        Ok(Account {
            email: ident.email,
            name: ident.name,
            provider,
            imap,
            smtp,
            auth: Auth::Oauth {
                refresh_token: tokens.refresh_token.context("没有拿到 refresh_token，请撤销授权后重试")?,
                access_token: Some(tokens.access_token),
                expires_at: tokens.expires_at,
            },
            master,
        })
    }

    /// Sign in with the master Google account and restore every synced account.
    pub async fn login_master(&self, opts: &LoginOpts) -> Result<(Account, CloudResult)> {
        let client = self.oauth_client(Provider::Google)?;
        let (tokens, ident) = oauth::login(&self.http, &client, true, opts.manual, opts.on_url.as_ref()).await?;
        let acct = Self::oauth_account(Provider::Google, tokens, ident, true)?;
        {
            let mut book = self.book();
            for a in &mut book.accounts {
                a.master = false;
            }
            book.upsert(acct.clone());
            book.save()?;
        }
        let mut cfg = self.config();
        cfg.sync = Some(SyncTarget::Google);
        self.set_config(cfg)?;
        let res = self.join_sync().await?;
        Ok((acct, res))
    }

    /// Sync the account list through a self-hosted server, WebDAV or a file,
    /// merging with whatever is already on this device. Reverts on failure.
    pub async fn setup_sync(&self, target: SyncTarget, passphrase: Option<String>) -> Result<CloudResult> {
        if target == SyncTarget::Google {
            bail!("Google 同步请使用 login（apark login）");
        }
        let before = self.config();
        let mut cfg = before.clone();
        cfg.sync = Some(target);
        if let Some(p) = passphrase.filter(|p| !p.is_empty()) {
            cfg.sync_passphrase = Some(p);
        }
        self.set_config(cfg)?;
        match self.join_sync().await {
            Ok(r) => Ok(r),
            Err(e) => {
                self.set_config(before)?;
                Err(e)
            }
        }
    }

    /// Stop syncing the account list (accounts stay on this device).
    pub fn disable_sync(&self) -> Result<()> {
        let mut cfg = self.config();
        cfg.sync = None;
        self.set_config(cfg)?;
        let mut book = self.book();
        for a in &mut book.accounts {
            a.master = false;
        }
        book.save()
    }

    /// Current sync target; legacy setups with a master account mean Google.
    pub fn sync_target(&self) -> Option<SyncTarget> {
        self.config().sync.or_else(|| self.has_master().then_some(SyncTarget::Google))
    }

    pub async fn add_oauth(&self, provider: Provider, opts: &LoginOpts) -> Result<Account> {
        let client = self.oauth_client(provider)?;
        let (tokens, ident) = oauth::login(&self.http, &client, false, opts.manual, opts.on_url.as_ref()).await?;
        let mut acct = Self::oauth_account(provider, tokens, ident, false)?;
        if let Ok(old) = self.account(&acct.email) {
            acct.master = old.master;
        }
        self.save_account(acct.clone()).await;
        Ok(acct)
    }

    /// Add a password / app-password account after verifying the IMAP login.
    pub async fn add_password(
        &self,
        email: &str,
        password: &str,
        name: &str,
        imap_server: Option<Server>,
        smtp_server: Option<Server>,
    ) -> Result<Account> {
        let (pi, ps) = preset(email);
        let acct = Account {
            email: email.trim().to_owned(),
            name: name.trim().to_owned(),
            provider: Provider::Imap,
            imap: imap_server.unwrap_or(pi),
            smtp: smtp_server.unwrap_or(ps),
            auth: Auth::Password { password: password.to_owned() },
            master: false,
        };
        let mut s = imap::connect(&acct, &Secret::Password(password.to_owned())).await?;
        let _ = s.logout().await;
        self.save_account(acct.clone()).await;
        Ok(acct)
    }

    async fn save_account(&self, acct: Account) {
        {
            let mut book = self.book();
            book.upsert(acct);
            book.updated_at = crate::now();
            let _ = book.save();
        }
        let _ = self.cloud_push().await;
    }

    pub async fn remove_account(&self, email: &str) -> Result<()> {
        let was_master = {
            let mut book = self.book();
            let was_master = book.get(email).map(|a| a.master).context("没有这个账号")?;
            book.remove(email);
            book.updated_at = crate::now();
            book.save()?;
            was_master
        };
        self.store.remove_account(email)?;
        if was_master {
            // Removing the Google master ends Google sync.
            if self.config().sync == Some(SyncTarget::Google) {
                self.disable_sync()?;
            }
        } else {
            self.cloud_push().await?;
        }
        Ok(())
    }

    /// Add an account without verifying it (demo data, tests).
    pub fn insert_account_unchecked(&self, acct: Account) -> Result<()> {
        let mut book = self.book();
        book.upsert(acct);
        book.save()
    }

    // ---- cloud account sync -----------------------------------------------

    async fn master_token(&self) -> Result<Option<String>> {
        let Some(email) = self.book().master().map(|a| a.email.clone()) else { return Ok(None) };
        match self.secret(&email).await? {
            Secret::Bearer(t) => Ok(Some(t)),
            Secret::Password(_) => Ok(None),
        }
    }

    /// Resolve the sync target into (backend, passphrase, is_google).
    async fn backend(&self) -> Result<Option<(cloud::Backend, Option<String>, bool)>> {
        let Some(target) = self.sync_target() else { return Ok(None) };
        let pass = self.config().passphrase();
        let need = |p: Option<String>| p.context("这种同步方式需要先设置同步密码（用来加密账号列表）");
        Ok(Some(match target {
            SyncTarget::Google => {
                let Some(token) = self.master_token().await? else { return Ok(None) };
                (cloud::Backend::Google { token }, pass, true)
            }
            SyncTarget::Server { url, user, password, token } => {
                let id = cloud::server_blob_id(&user, &password)?;
                (cloud::Backend::Server { url, id, token }, Some(password), false)
            }
            SyncTarget::Webdav { url, user, password } => (cloud::Backend::Webdav { url, user, password }, Some(need(pass)?), false),
            SyncTarget::File { path } => (cloud::Backend::File { path: path.into() }, Some(need(pass)?), false),
        }))
    }

    /// Accounts that travel to the cloud. The Google master signs in on its own.
    fn snapshot(&self, google: bool) -> cloud::Snapshot {
        let cfg = self.config();
        let book = self.book();
        cloud::Snapshot {
            updated_at: book.updated_at,
            accounts: book.accounts.iter().filter(|a| !(google && a.master)).cloned().collect(),
            clients: cloud::Clients {
                google_client_id: cfg.google_client_id.clone(),
                google_client_secret: cfg.google_client_secret.clone(),
                microsoft_client_id: cfg.microsoft_client_id.clone(),
            },
        }
    }

    /// Take OAuth client settings from the cloud copy where this device has none,
    /// so restored Google/Microsoft accounts can refresh their tokens here.
    fn adopt_clients(&self, c: &cloud::Clients) -> Result<()> {
        let mut cfg = self.config();
        let before = (cfg.google_client_id.clone(), cfg.google_client_secret.clone(), cfg.microsoft_client_id.clone());
        cfg.google_client_id = cfg.google_client_id.or_else(|| c.google_client_id.clone());
        cfg.google_client_secret = cfg.google_client_secret.or_else(|| c.google_client_secret.clone());
        cfg.microsoft_client_id = cfg.microsoft_client_id.or_else(|| c.microsoft_client_id.clone());
        if (cfg.google_client_id.clone(), cfg.google_client_secret.clone(), cfg.microsoft_client_id.clone()) != before {
            self.set_config(cfg)?;
        }
        Ok(())
    }

    pub async fn cloud_push(&self) -> Result<()> {
        let Some((backend, pass, google)) = self.backend().await? else { return Ok(()) };
        cloud::push(&self.http, &backend, &self.snapshot(google), pass.as_deref()).await
    }

    /// Replace the local account set with `remote` (keeping the Google master and
    /// still-valid access tokens). Returns the emails added and removed.
    fn apply_remote(&self, remote: Vec<Account>, google: bool, extra: Vec<Account>, updated_at: i64) -> Result<CloudResult> {
        let mut res = CloudResult { action: "pulled", ..Default::default() };
        let mut book = self.book();
        let mut next: Vec<Account> = if google { book.master().cloned().into_iter().collect() } else { vec![] };
        for mut a in remote.into_iter().chain(extra) {
            if next.iter().any(|x| x.email.eq_ignore_ascii_case(&a.email)) {
                continue;
            }
            if let Some(old) = book.get(&a.email) {
                if let (Auth::Oauth { refresh_token, access_token, expires_at }, Auth::Oauth { refresh_token: ort, access_token: oat, expires_at: oexp }) =
                    (&mut a.auth, &old.auth)
                {
                    if ort == refresh_token {
                        *access_token = oat.clone();
                        *expires_at = *oexp;
                    }
                }
            } else {
                res.added.push(a.email.clone());
            }
            next.push(a);
        }
        for old in &book.accounts {
            if !next.iter().any(|x| x.email.eq_ignore_ascii_case(&old.email)) {
                res.removed.push(old.email.clone());
            }
        }
        book.accounts = next;
        book.updated_at = updated_at;
        book.save()?;
        drop(book);
        for email in &res.removed {
            self.store.remove_account(email)?;
        }
        Ok(res)
    }

    /// First sync on this device: union of the cloud list and local accounts, then upload.
    async fn join_sync(&self) -> Result<CloudResult> {
        let Some((backend, pass, google)) = self.backend().await? else {
            return Ok(CloudResult { action: "no-sync", ..Default::default() });
        };
        let local = self.snapshot(google).accounts;
        let remote = cloud::pull(&self.http, &backend, pass.as_deref()).await?;
        if let Some(r) = &remote {
            self.adopt_clients(&r.clients)?;
        }
        let remote = remote.map(|s| s.accounts).unwrap_or_default();
        let mut res = self.apply_remote(remote, google, local, crate::now())?;
        cloud::push(&self.http, &backend, &self.snapshot(google), pass.as_deref()).await?;
        res.action = "joined";
        Ok(res)
    }

    /// Reconcile the local account list with the cloud copy; newer wins.
    pub async fn cloud_sync(&self) -> Result<CloudResult> {
        let Some((backend, pass, google)) = self.backend().await? else {
            return Ok(CloudResult { action: "no-sync", ..Default::default() });
        };
        let local = self.snapshot(google);
        match cloud::pull(&self.http, &backend, pass.as_deref()).await? {
            Some(r) if r.updated_at > local.updated_at => {
                self.adopt_clients(&r.clients)?;
                self.apply_remote(r.accounts, google, vec![], r.updated_at)
            }
            Some(r) if r.updated_at == local.updated_at => Ok(CloudResult { action: "unchanged", ..Default::default() }),
            _ => {
                cloud::push(&self.http, &backend, &local, pass.as_deref()).await?;
                Ok(CloudResult { action: "pushed", ..Default::default() })
            }
        }
    }

    // ---- sync -----------------------------------------------------------

    async fn session(&self, email: &str) -> Result<imap::Session> {
        let acct = self.account(email)?;
        let secret = self.secret(email).await?;
        imap::connect(&acct, &secret).await
    }

    /// Demo accounts (`apark dev-seed`) point at `.invalid` hosts and never touch the network.
    fn is_demo(&self, email: &str) -> bool {
        self.account(email).is_ok_and(|a| a.imap.host.ends_with(".invalid"))
    }

    /// Sync INBOX plus `extra` folders of one account. Returns new message count.
    pub async fn sync_account(&self, email: &str, extra: &[String]) -> Result<usize> {
        if self.is_demo(email) {
            return Ok(0);
        }
        let cfg = self.config();
        let mut s = self.session(email).await?;
        let folders = imap::list_folders(&mut s).await?;
        self.store.save_folders(email, &folders)?;
        let mut targets = vec!["INBOX".to_owned()];
        targets.extend(extra.iter().filter(|f| !f.eq_ignore_ascii_case("INBOX")).cloned());
        let mut new = 0;
        for f in &targets {
            new += imap::sync_folder(&mut s, &self.store, email, f, cfg.initial_limit, cfg.prefetch_kb * 1024).await?;
        }
        let _ = s.logout().await;
        Ok(new)
    }

    /// Cloud account sync, then every account in parallel. Skips if a sync is already running.
    pub async fn sync_all(&self) -> Option<Vec<(String, Result<usize>)>> {
        let _guard = self.sync_lock.try_lock().ok()?;
        let mut results = vec![];
        if let Err(e) = self.cloud_sync().await {
            results.push(("cloud".to_owned(), Err(e)));
        }
        let emails: Vec<String> = self.accounts().into_iter().map(|a| a.email).collect();
        let runs = emails.iter().map(|e| self.sync_account(e, &[]));
        let done = futures::future::join_all(runs).await;
        results.extend(emails.into_iter().zip(done));
        Some(results)
    }

    /// Create, rename (`to` = Some) or delete (`to` = None, `create` = false) a folder.
    pub async fn edit_folder(&self, email: &str, name: &str, create: bool, to: Option<&str>) -> Result<()> {
        let mut s = self.session(email).await?;
        let r = match (create, to) {
            (true, _) => s.create(name).await,
            (false, Some(to)) => s.rename(name, to).await,
            (false, None) => s.delete(name).await,
        };
        let folders = imap::list_folders(&mut s).await;
        let _ = s.logout().await;
        r.with_context(|| format!("操作文件夹失败: {name}"))?;
        self.store.save_folders(email, &folders?)?;
        Ok(())
    }

    pub fn folders(&self, email: &str) -> Result<Vec<Folder>> {
        self.store.folders(email)
    }

    pub fn list(&self, q: &ListQuery) -> Result<Vec<MsgRow>> {
        self.store.list(q)
    }

    pub fn get(&self, id: i64) -> Result<MsgRow> {
        self.store.get(id)?.ok_or_else(|| anyhow!("没有这封邮件: {id}"))
    }

    // ---- message actions ------------------------------------------------

    pub async fn body(&self, id: i64) -> Result<Body> {
        if let Some(b) = self.store.body(id)? {
            return Ok(b);
        }
        let m = self.get(id)?;
        let raw = self.raw(id).await?;
        let body = imap::parse_body(&raw);
        self.store.set_body(&m.account, &m.folder, m.uid, &body)?;
        Ok(body)
    }

    pub async fn raw(&self, id: i64) -> Result<Vec<u8>> {
        let m = self.get(id)?;
        let mut s = self.session(&m.account).await?;
        let raw = imap::fetch_raw(&mut s, &m.folder, m.uid).await;
        let _ = s.logout().await;
        raw
    }

    pub async fn save_attachments(&self, id: i64, index: Option<usize>, dir: &Path) -> Result<Vec<PathBuf>> {
        let raw = self.raw(id).await?;
        imap::extract_attachments(&raw, index, dir)
    }

    async fn remote<F>(&self, m: &MsgRow, op: F) -> Result<()>
    where
        F: for<'a> FnOnce(&'a mut imap::Session) -> futures::future::BoxFuture<'a, Result<()>>,
    {
        if self.is_demo(&m.account) {
            return Ok(());
        }
        let mut s = self.session(&m.account).await?;
        let r = op(&mut s).await;
        let _ = s.logout().await;
        r
    }

    pub async fn set_seen(&self, id: i64, seen: bool) -> Result<()> {
        let m = self.get(id)?;
        self.store.set_seen(id, seen)?;
        let (folder, uid) = (m.folder.clone(), m.uid);
        self.remote(&m, move |s| Box::pin(async move { imap::set_flag(s, &folder, uid, "\\Seen", seen).await }))
            .await
    }

    pub async fn set_flagged(&self, id: i64, flagged: bool) -> Result<()> {
        let m = self.get(id)?;
        self.store.set_flagged(id, flagged)?;
        let (folder, uid) = (m.folder.clone(), m.uid);
        self.remote(&m, move |s| Box::pin(async move { imap::set_flag(s, &folder, uid, "\\Flagged", flagged).await }))
            .await
    }

    pub async fn move_to(&self, id: i64, dest: &str) -> Result<()> {
        let m = self.get(id)?;
        if m.folder == dest {
            bail!("邮件已经在 {dest}");
        }
        self.store.delete(id)?;
        let (folder, uid, dest) = (m.folder.clone(), m.uid, dest.to_owned());
        self.remote(&m, move |s| Box::pin(async move { imap::move_to(s, &folder, uid, &dest).await }))
            .await
    }

    pub async fn archive(&self, id: i64) -> Result<()> {
        let m = self.get(id)?;
        let dest = self
            .store
            .folder_by_role(&m.account, &["archive", "all"])?
            .context("这个账号没有归档文件夹")?;
        self.move_to(id, &dest).await
    }

    pub async fn trash(&self, id: i64) -> Result<()> {
        let m = self.get(id)?;
        let dest = self.store.folder_by_role(&m.account, &["trash"])?.context("这个账号没有废纸篓文件夹")?;
        self.move_to(id, &dest).await
    }

    // ---- compose --------------------------------------------------------

    pub async fn send(&self, from: &str, out: &Outgoing) -> Result<()> {
        let acct = self.account(from)?;
        let secret = self.secret(from).await?;
        let raw = smtp::send(&acct, &secret, out).await?;
        // Gmail and Outlook file sent mail themselves; plain IMAP servers need APPEND.
        if acct.provider == Provider::Imap {
            if let Some(sent) = self.store.folder_by_role(from, &["sent"])? {
                let mut s = imap::connect(&acct, &secret).await?;
                let r = imap::append(&mut s, &sent, &raw).await;
                let _ = s.logout().await;
                r.context("已发送，但保存到“已发送”失败")?;
            }
        }
        Ok(())
    }

    /// Prefill a reply. Returns (from account, draft).
    pub async fn reply_draft(&self, id: i64, all: bool) -> Result<(String, Outgoing)> {
        let m = self.get(id)?;
        let body = self.body(id).await.unwrap_or_default();
        let me = m.account.to_ascii_lowercase();
        let mut to = vec![m.from_addr.clone()];
        let mut cc = vec![];
        if all {
            to.extend(emails_in(&m.to));
            cc.extend(emails_in(&m.cc));
        }
        let mut seen = std::collections::HashSet::new();
        let mut keep = |v: Vec<String>| -> Vec<String> {
            v.into_iter()
                .filter(|a| !a.is_empty() && a.to_ascii_lowercase() != me && seen.insert(a.to_ascii_lowercase()))
                .collect()
        };
        let to = keep(to);
        let cc = keep(cc);
        let subject = if m.subject.to_ascii_lowercase().starts_with("re:") {
            m.subject.clone()
        } else {
            format!("Re: {}", m.subject)
        };
        let references = format!("{} {}", m.references, m.message_id).trim().to_owned();
        let quoted: String = body.text.lines().map(|l| format!("> {l}\n")).collect();
        let when = chrono::DateTime::from_timestamp(m.date, 0)
            .map(|d| d.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_default();
        Ok((
            m.account.clone(),
            Outgoing {
                to,
                cc,
                subject,
                body: format!("\n\n{when}，{} 写道：\n{quoted}", m.sender()),
                in_reply_to: Some(m.message_id.clone()),
                references: Some(references),
                ..Default::default()
            },
        ))
    }

    /// Prefill a forward, including the original attachments.
    pub async fn forward_draft(&self, id: i64) -> Result<(String, Outgoing)> {
        let m = self.get(id)?;
        let body = self.body(id).await.unwrap_or_default();
        let mut attachments = vec![];
        if !body.attachments.is_empty() {
            let dir = std::env::temp_dir().join(format!("apark-fwd-{id}"));
            attachments = self.save_attachments(id, None, &dir).await?;
        }
        let when = chrono::DateTime::from_timestamp(m.date, 0)
            .map(|d| d.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_default();
        Ok((
            m.account.clone(),
            Outgoing {
                subject: format!("Fwd: {}", m.subject),
                body: format!(
                    "\n\n---------- 转发的邮件 ----------\n发件人: {} <{}>\n日期: {when}\n主题: {}\n收件人: {}\n\n{}",
                    m.from_name, m.from_addr, m.subject, m.to, body.text
                ),
                attachments,
                ..Default::default()
            },
        ))
    }
}

/// Pull bare addresses out of a "Name <a@b>, c@d" list.
pub fn emails_in(s: &str) -> Vec<String> {
    s.split(',')
        .filter_map(|part| {
            let part = part.trim();
            let addr = match (part.rfind('<'), part.rfind('>')) {
                (Some(a), Some(b)) if a < b => &part[a + 1..b],
                _ => part,
            };
            addr.contains('@').then(|| addr.trim().to_owned())
        })
        .collect()
}
