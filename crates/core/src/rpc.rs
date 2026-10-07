//! JSON RPC over [`Engine`], shared by every native UI (SwiftUI, WinUI, GTK).
//!
//! Request: `{"method": "...", "params": {...}}`
//! Reply:   `{"ok": true, "result": ...}` or `{"ok": false, "error": "..."}`
//! Events (login URL, background sync results) go to the `emit` sink.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};

use crate::account::Server;
use crate::{Account, Config, Engine, ListQuery, LoginOpts, Outgoing, Provider};

pub type Emit = Arc<dyn Fn(Value) + Send + Sync>;

pub struct Rpc {
    pub eng: Arc<Engine>,
    emit: Emit,
    auto_sync: std::sync::atomic::AtomicBool,
}

fn arg<T: DeserializeOwned>(p: &Value, key: &str) -> Result<T> {
    serde_json::from_value(p.get(key).cloned().unwrap_or(Value::Null)).with_context(|| format!("参数 {key} 缺失或无效"))
}

fn opt<T: DeserializeOwned>(p: &Value, key: &str) -> Option<T> {
    p.get(key).cloned().and_then(|v| serde_json::from_value(v).ok())
}

pub fn account_json(a: &Account) -> Value {
    json!({
        "email": a.email, "name": a.name, "provider": a.provider, "master": a.master,
        "imap": format!("{}:{}", a.imap.host, a.imap.port), "smtp": format!("{}:{}", a.smtp.host, a.smtp.port),
    })
}

fn sync_json(results: &[(String, Result<usize>)]) -> Value {
    Value::Array(
        results
            .iter()
            .map(|(a, r)| match r {
                Ok(n) => json!({ "account": a, "ok": true, "new": n }),
                Err(e) => json!({ "account": a, "ok": false, "error": format!("{e:#}") }),
            })
            .collect(),
    )
}

impl Rpc {
    pub fn new(eng: Arc<Engine>, emit: Emit) -> Arc<Rpc> {
        Arc::new(Rpc { eng, emit, auto_sync: false.into() })
    }

    fn login_opts(&self) -> LoginOpts {
        let emit = self.emit.clone();
        LoginOpts { manual: false, on_url: Arc::new(move |url| emit(json!({ "type": "login_url", "url": url }))) }
    }

    /// Handle one raw request string; never fails (errors become `ok: false`).
    pub async fn handle(self: &Arc<Self>, request: &str) -> String {
        let reply = match serde_json::from_str::<Value>(request) {
            Ok(req) => {
                let method = req["method"].as_str().unwrap_or_default().to_owned();
                let params = req.get("params").cloned().unwrap_or(Value::Null);
                match self.call(&method, &params).await {
                    Ok(result) => json!({ "ok": true, "result": result }),
                    Err(e) => json!({ "ok": false, "error": format!("{e:#}") }),
                }
            }
            Err(e) => json!({ "ok": false, "error": format!("请求不是有效 JSON: {e}") }),
        };
        reply.to_string()
    }

    pub async fn call(self: &Arc<Self>, method: &str, p: &Value) -> Result<Value> {
        let eng = &self.eng;
        Ok(match method {
            "info" => {
                let cfg = eng.config();
                json!({
                    "version": env!("CARGO_PKG_VERSION"),
                    "data_dir": crate::paths::home(),
                    "has_master": eng.has_master(),
                    "google_ready": cfg.google_client().is_some(),
                    "microsoft_ready": cfg.microsoft_client().is_some(),
                })
            }
            "accounts" => Value::Array(eng.accounts().iter().map(account_json).collect()),
            "login_master" => {
                let (a, r) = eng.login_master(&self.login_opts()).await?;
                json!({ "master": account_json(&a), "cloud": r })
            }
            "add_oauth" => {
                let provider = match arg::<String>(p, "provider")?.as_str() {
                    "microsoft" => Provider::Microsoft,
                    _ => Provider::Google,
                };
                account_json(&eng.add_oauth(provider, &self.login_opts()).await?)
            }
            "add_password" => {
                let server = |k: &str, port| -> Result<Option<Server>> {
                    opt::<String>(p, k).filter(|s| !s.trim().is_empty()).map(|s| Server::parse(&s, port)).transpose()
                };
                let a = eng
                    .add_password(
                        &arg::<String>(p, "email")?,
                        &arg::<String>(p, "password")?,
                        &opt::<String>(p, "name").unwrap_or_default(),
                        server("imap", 993)?,
                        server("smtp", 465)?,
                    )
                    .await?;
                account_json(&a)
            }
            "remove_account" => {
                eng.remove_account(&arg::<String>(p, "email")?).await?;
                Value::Null
            }
            "sync_all" => {
                let r = eng.sync_all().await.unwrap_or_default();
                let v = sync_json(&r);
                (self.emit)(json!({ "type": "synced", "results": v }));
                v
            }
            "sync_account" => {
                let email = arg::<String>(p, "email")?;
                let folders: Vec<String> = opt(p, "folders").unwrap_or_default();
                json!({ "new": eng.sync_account(&email, &folders).await? })
            }
            "start_auto_sync" => {
                if !self.auto_sync.swap(true, std::sync::atomic::Ordering::SeqCst) {
                    let me = self.clone();
                    tokio::spawn(async move {
                        loop {
                            if let Some(r) = me.eng.sync_all().await {
                                (me.emit)(json!({ "type": "synced", "results": sync_json(&r) }));
                            }
                            let secs = me.eng.config().sync_interval_secs.max(30);
                            tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
                        }
                    });
                }
                Value::Null
            }
            "folders" => serde_json::to_value(eng.folders(&arg::<String>(p, "email")?)?)?,
            "edit_folder" => {
                let to: Option<String> = opt(p, "to");
                eng.edit_folder(&arg::<String>(p, "email")?, &arg::<String>(p, "name")?, opt(p, "create").unwrap_or(false), to.as_deref())
                    .await?;
                Value::Null
            }
            "list" => {
                let q = ListQuery {
                    account: opt(p, "account"),
                    folder: opt(p, "folder"),
                    category: opt(p, "category"),
                    unread: opt(p, "unread").unwrap_or(false),
                    flagged: opt(p, "flagged").unwrap_or(false),
                    search: opt::<String>(p, "search").filter(|s| !s.trim().is_empty()),
                    limit: opt(p, "limit").unwrap_or(5000),
                };
                serde_json::to_value(eng.list(&q)?)?
            }
            "unread_counts" => {
                let m: serde_json::Map<String, Value> =
                    eng.store.unread_counts()?.into_iter().map(|(k, v)| (k, json!(v))).collect();
                Value::Object(m)
            }
            "get" => serde_json::to_value(eng.get(arg(p, "id")?)?)?,
            "cached_body" => serde_json::to_value(eng.store.body(arg(p, "id")?)?)?,
            "body" => serde_json::to_value(eng.body(arg(p, "id")?).await?)?,
            "set_seen" => {
                eng.set_seen(arg(p, "id")?, arg(p, "seen")?).await?;
                Value::Null
            }
            "set_flagged" => {
                eng.set_flagged(arg(p, "id")?, arg(p, "flagged")?).await?;
                Value::Null
            }
            "archive" => {
                eng.archive(arg(p, "id")?).await?;
                Value::Null
            }
            "trash" => {
                eng.trash(arg(p, "id")?).await?;
                Value::Null
            }
            "move" => {
                eng.move_to(arg(p, "id")?, &arg::<String>(p, "to")?).await?;
                Value::Null
            }
            "send" => {
                let from: String = arg(p, "from")?;
                let out: Outgoing = serde_json::from_value(p.clone()).context("邮件参数无效")?;
                eng.send(&from, &out).await?;
                Value::Null
            }
            "reply_draft" => {
                let (from, d) = eng.reply_draft(arg(p, "id")?, opt(p, "all").unwrap_or(false)).await?;
                json!({ "from": from, "draft": d })
            }
            "forward_draft" => {
                let (from, d) = eng.forward_draft(arg(p, "id")?).await?;
                json!({ "from": from, "draft": d })
            }
            "save_attachments" => {
                let dir: PathBuf = opt(p, "dir").or_else(dirs::download_dir).unwrap_or_else(std::env::temp_dir);
                serde_json::to_value(eng.save_attachments(arg(p, "id")?, opt(p, "index"), &dir).await?)?
            }
            "config" => serde_json::to_value(eng.config())?,
            "set_config" => {
                let cfg: Config = serde_json::from_value(p.clone()).context("配置无效")?;
                eng.set_config(cfg)?;
                Value::Null
            }
            "cloud_sync" => serde_json::to_value(eng.cloud_sync().await?)?,
            _ => return Err(anyhow!("未知方法: {method}")),
        })
    }
}
