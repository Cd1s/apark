//! Local SQLite cache. Every UI read hits this, never the network, which is
//! what keeps the app instant. A separate reader connection (WAL mode) means
//! background sync never blocks the UI thread.

use std::path::Path;
use std::sync::Mutex;

use anyhow::Result;
use rusqlite::types::Value;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

pub struct Store {
    w: Mutex<Connection>,
    r: Mutex<Connection>,
}

#[derive(Clone, Debug, Default)]
pub struct Header {
    pub message_id: String,
    pub in_reply_to: String,
    pub references: String,
    pub subject: String,
    pub from_name: String,
    pub from_addr: String,
    pub to: String,
    pub cc: String,
    pub date: i64,
    pub category: String,
}

pub struct NewMsg {
    pub uid: u32,
    pub seen: bool,
    pub flagged: bool,
    pub size: u32,
    pub header: Header,
}

#[derive(Clone, Debug, Serialize)]
pub struct MsgRow {
    pub id: i64,
    pub account: String,
    pub folder: String,
    pub uid: u32,
    pub message_id: String,
    pub references: String,
    pub subject: String,
    pub from_name: String,
    pub from_addr: String,
    pub to: String,
    pub cc: String,
    pub date: i64,
    pub size: u32,
    pub seen: bool,
    pub flagged: bool,
    pub category: String,
    pub snippet: String,
    pub has_body: bool,
}

impl MsgRow {
    pub fn sender(&self) -> &str {
        if self.from_name.is_empty() {
            &self.from_addr
        } else {
            &self.from_name
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Attachment {
    pub name: String,
    pub size: usize,
}

#[derive(Clone, Debug, Serialize, Default)]
pub struct Body {
    pub text: String,
    pub html: Option<String>,
    pub attachments: Vec<Attachment>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Folder {
    pub name: String,
    pub role: String,
}

#[derive(Clone, Debug, Default)]
pub struct ListQuery {
    pub account: Option<String>,
    /// None = INBOX, or every folder when searching.
    pub folder: Option<String>,
    pub category: Option<String>,
    pub unread: bool,
    pub flagged: bool,
    pub search: Option<String>,
    pub limit: usize,
}

const SCHEMA: &str = "
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
CREATE TABLE IF NOT EXISTS folders(
    account TEXT NOT NULL, name TEXT NOT NULL, role TEXT NOT NULL DEFAULT '',
    uidvalidity INTEGER NOT NULL DEFAULT 0, last_uid INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY(account, name));
CREATE TABLE IF NOT EXISTS messages(
    id INTEGER PRIMARY KEY,
    account TEXT NOT NULL, folder TEXT NOT NULL, uid INTEGER NOT NULL,
    message_id TEXT NOT NULL DEFAULT '', in_reply_to TEXT NOT NULL DEFAULT '', refs TEXT NOT NULL DEFAULT '',
    subject TEXT NOT NULL DEFAULT '', from_name TEXT NOT NULL DEFAULT '', from_addr TEXT NOT NULL DEFAULT '',
    to_addrs TEXT NOT NULL DEFAULT '', cc_addrs TEXT NOT NULL DEFAULT '',
    date INTEGER NOT NULL DEFAULT 0, size INTEGER NOT NULL DEFAULT 0,
    seen INTEGER NOT NULL DEFAULT 0, flagged INTEGER NOT NULL DEFAULT 0,
    category TEXT NOT NULL DEFAULT 'people', snippet TEXT NOT NULL DEFAULT '',
    body_text TEXT, body_html TEXT, attachments TEXT, has_body INTEGER NOT NULL DEFAULT 0,
    UNIQUE(account, folder, uid));
CREATE INDEX IF NOT EXISTS messages_folder_date ON messages(folder, date DESC);
CREATE INDEX IF NOT EXISTS messages_account_folder_date ON messages(account, folder, date DESC);
";

const ROW_COLS: &str = "id, account, folder, uid, message_id, refs, subject, from_name, from_addr, \
    to_addrs, cc_addrs, date, size, seen, flagged, category, snippet, has_body";

fn row(r: &rusqlite::Row) -> rusqlite::Result<MsgRow> {
    Ok(MsgRow {
        id: r.get(0)?,
        account: r.get(1)?,
        folder: r.get(2)?,
        uid: r.get(3)?,
        message_id: r.get(4)?,
        references: r.get(5)?,
        subject: r.get(6)?,
        from_name: r.get(7)?,
        from_addr: r.get(8)?,
        to: r.get(9)?,
        cc: r.get(10)?,
        date: r.get(11)?,
        size: r.get(12)?,
        seen: r.get(13)?,
        flagged: r.get(14)?,
        category: r.get(15)?,
        snippet: r.get(16)?,
        has_body: r.get(17)?,
    })
}

fn role_rank(role: &str) -> u8 {
    match role {
        "inbox" => 0,
        "flagged" => 1,
        "drafts" => 2,
        "sent" => 3,
        "archive" | "all" => 4,
        "junk" => 5,
        "trash" => 6,
        _ => 9,
    }
}

impl Store {
    pub fn open(path: &Path) -> Result<Store> {
        let w = Connection::open(path)?;
        w.execute_batch(SCHEMA)?;
        w.busy_timeout(std::time::Duration::from_secs(10))?;
        let r = Connection::open(path)?;
        r.busy_timeout(std::time::Duration::from_secs(10))?;
        Ok(Store { w: Mutex::new(w), r: Mutex::new(r) })
    }

    fn w(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.w.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn r(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.r.lock().unwrap_or_else(|e| e.into_inner())
    }

    // ---- folders -------------------------------------------------------

    pub fn folder_state(&self, account: &str, folder: &str) -> Result<(u32, u32)> {
        Ok(self
            .w()
            .query_row(
                "SELECT uidvalidity, last_uid FROM folders WHERE account=?1 AND name=?2",
                params![account, folder],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
            .unwrap_or((0, 0)))
    }

    pub fn set_folder_state(&self, account: &str, folder: &str, uidvalidity: u32, last_uid: u32) -> Result<()> {
        self.w().execute(
            "INSERT INTO folders(account, name, uidvalidity, last_uid) VALUES(?1, ?2, ?3, ?4)
             ON CONFLICT(account, name) DO UPDATE SET uidvalidity=?3, last_uid=?4",
            params![account, folder, uidvalidity, last_uid],
        )?;
        Ok(())
    }

    pub fn save_folders(&self, account: &str, folders: &[Folder]) -> Result<()> {
        let mut c = self.w();
        let tx = c.transaction()?;
        for f in folders {
            tx.execute(
                "INSERT INTO folders(account, name, role) VALUES(?1, ?2, ?3)
                 ON CONFLICT(account, name) DO UPDATE SET role=?3",
                params![account, f.name, f.role],
            )?;
        }
        let names: Vec<&str> = folders.iter().map(|f| f.name.as_str()).collect();
        let existing: Vec<String> = {
            let mut st = tx.prepare("SELECT name FROM folders WHERE account=?1")?;
            let v = st.query_map([account], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
            v
        };
        for name in existing.iter().filter(|n| !names.contains(&n.as_str())) {
            tx.execute("DELETE FROM folders WHERE account=?1 AND name=?2", params![account, name])?;
            tx.execute("DELETE FROM messages WHERE account=?1 AND folder=?2", params![account, name])?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn folders(&self, account: &str) -> Result<Vec<Folder>> {
        let c = self.r();
        let mut st = c.prepare("SELECT name, role FROM folders WHERE account=?1")?;
        let mut v: Vec<Folder> = st
            .query_map([account], |r| Ok(Folder { name: r.get(0)?, role: r.get(1)? }))?
            .collect::<rusqlite::Result<_>>()?;
        v.sort_by(|a, b| role_rank(&a.role).cmp(&role_rank(&b.role)).then_with(|| a.name.cmp(&b.name)));
        Ok(v)
    }

    pub fn folder_by_role(&self, account: &str, roles: &[&str]) -> Result<Option<String>> {
        let folders = self.folders(account)?;
        Ok(roles.iter().find_map(|role| folders.iter().find(|f| f.role == *role).map(|f| f.name.clone())))
    }

    // ---- messages ------------------------------------------------------

    pub fn reset_folder(&self, account: &str, folder: &str) -> Result<()> {
        self.w().execute("DELETE FROM messages WHERE account=?1 AND folder=?2", params![account, folder])?;
        Ok(())
    }

    pub fn insert_headers(&self, account: &str, folder: &str, msgs: &[NewMsg]) -> Result<()> {
        let mut c = self.w();
        let tx = c.transaction()?;
        {
            let mut st = tx.prepare_cached(
                "INSERT INTO messages(account, folder, uid, message_id, in_reply_to, refs, subject, from_name,
                    from_addr, to_addrs, cc_addrs, date, size, seen, flagged, category)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)
                 ON CONFLICT(account, folder, uid) DO UPDATE SET seen=?14, flagged=?15",
            )?;
            for m in msgs {
                let h = &m.header;
                st.execute(params![
                    account, folder, m.uid, h.message_id, h.in_reply_to, h.references, h.subject, h.from_name,
                    h.from_addr, h.to, h.cc, h.date, m.size, m.seen, m.flagged, h.category
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn known_uids(&self, account: &str, folder: &str) -> Result<Vec<u32>> {
        let c = self.w();
        let mut st = c.prepare_cached("SELECT uid FROM messages WHERE account=?1 AND folder=?2 ORDER BY uid")?;
        let v = st.query_map(params![account, folder], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        Ok(v)
    }

    pub fn update_flags(&self, account: &str, folder: &str, flags: &[(u32, bool, bool)]) -> Result<()> {
        let mut c = self.w();
        let tx = c.transaction()?;
        {
            let mut st = tx.prepare_cached(
                "UPDATE messages SET seen=?4, flagged=?5 WHERE account=?1 AND folder=?2 AND uid=?3
                   AND (seen<>?4 OR flagged<>?5)",
            )?;
            for (uid, seen, flagged) in flags {
                st.execute(params![account, folder, uid, seen, flagged])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn delete_uids(&self, account: &str, folder: &str, uids: &[u32]) -> Result<()> {
        let mut c = self.w();
        let tx = c.transaction()?;
        for uid in uids {
            tx.execute("DELETE FROM messages WHERE account=?1 AND folder=?2 AND uid=?3", params![account, folder, uid])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Newest messages that still lack a cached body, up to `max_size` bytes each.
    pub fn need_body(&self, account: &str, folder: &str, max_size: u32, limit: usize) -> Result<Vec<u32>> {
        let c = self.w();
        let mut st = c.prepare_cached(
            "SELECT uid FROM messages WHERE account=?1 AND folder=?2 AND has_body=0 AND size<=?3
             ORDER BY date DESC LIMIT ?4",
        )?;
        let v = st
            .query_map(params![account, folder, max_size, limit as i64], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(v)
    }

    pub fn set_body(&self, account: &str, folder: &str, uid: u32, body: &Body) -> Result<()> {
        let snippet = snippet(&body.text);
        self.w().execute(
            "UPDATE messages SET body_text=?4, body_html=?5, attachments=?6, snippet=?7, has_body=1
             WHERE account=?1 AND folder=?2 AND uid=?3",
            params![
                account,
                folder,
                uid,
                body.text,
                body.html,
                serde_json::to_string(&body.attachments)?,
                snippet
            ],
        )?;
        Ok(())
    }

    pub fn list(&self, q: &ListQuery) -> Result<Vec<MsgRow>> {
        let mut sql = format!("SELECT {ROW_COLS} FROM messages WHERE 1=1");
        let mut args: Vec<Value> = Vec::new();
        let mut bind = |v: String| {
            args.push(Value::Text(v));
            args.len()
        };
        match (&q.folder, &q.search) {
            (Some(f), _) => sql += &format!(" AND folder = ?{}", bind(f.clone())),
            (None, None) => sql += " AND folder = 'INBOX'",
            (None, Some(_)) => {}
        }
        if let Some(a) = &q.account {
            sql += &format!(" AND account = ?{}", bind(a.clone()));
        }
        if let Some(c) = &q.category {
            sql += &format!(" AND category = ?{}", bind(c.clone()));
        }
        if q.unread {
            sql += " AND seen = 0";
        }
        if q.flagged {
            sql += " AND flagged = 1";
        }
        for word in q.search.as_deref().unwrap_or("").split_whitespace() {
            let n = bind(format!("%{}%", word.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")));
            sql += &format!(
                " AND (subject LIKE ?{n} ESCAPE '\\' OR from_name LIKE ?{n} ESCAPE '\\' \
                   OR from_addr LIKE ?{n} ESCAPE '\\' OR to_addrs LIKE ?{n} ESCAPE '\\' \
                   OR body_text LIKE ?{n} ESCAPE '\\')"
            );
        }
        sql.push_str(" ORDER BY date DESC");
        let limit = if q.limit == 0 { 5000 } else { q.limit };
        sql.push_str(&format!(" LIMIT {limit}"));
        let c = self.r();
        let mut st = c.prepare_cached(&sql)?;
        let v = st.query_map(rusqlite::params_from_iter(args), row)?.collect::<rusqlite::Result<_>>()?;
        Ok(v)
    }

    /// Messages stored after `id` (new arrivals since a previous `max_id`).
    pub fn since(&self, id: i64) -> Result<Vec<MsgRow>> {
        let c = self.r();
        let mut st = c.prepare_cached(&format!("SELECT {ROW_COLS} FROM messages WHERE id > ?1 ORDER BY id"))?;
        let v = st.query_map([id], row)?.collect::<rusqlite::Result<_>>()?;
        Ok(v)
    }

    pub fn max_id(&self) -> Result<i64> {
        Ok(self.r().query_row("SELECT COALESCE(MAX(id), 0) FROM messages", [], |r| r.get(0))?)
    }

    pub fn get(&self, id: i64) -> Result<Option<MsgRow>> {
        Ok(self
            .r()
            .query_row(&format!("SELECT {ROW_COLS} FROM messages WHERE id=?1"), [id], row)
            .optional()?)
    }

    pub fn body(&self, id: i64) -> Result<Option<Body>> {
        Ok(self
            .r()
            .query_row(
                "SELECT body_text, body_html, attachments FROM messages WHERE id=?1 AND has_body=1",
                [id],
                |r| {
                    let att: Option<String> = r.get(2)?;
                    Ok(Body {
                        text: r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                        html: r.get(1)?,
                        attachments: att.and_then(|a| serde_json::from_str(&a).ok()).unwrap_or_default(),
                    })
                },
            )
            .optional()?)
    }

    pub fn set_seen(&self, id: i64, seen: bool) -> Result<()> {
        self.w().execute("UPDATE messages SET seen=?2 WHERE id=?1", params![id, seen])?;
        Ok(())
    }

    pub fn set_flagged(&self, id: i64, flagged: bool) -> Result<()> {
        self.w().execute("UPDATE messages SET flagged=?2 WHERE id=?1", params![id, flagged])?;
        Ok(())
    }

    pub fn delete(&self, id: i64) -> Result<()> {
        self.w().execute("DELETE FROM messages WHERE id=?1", [id])?;
        Ok(())
    }

    /// Unread INBOX counts by Smart Inbox category.
    pub fn unread_counts(&self) -> Result<Vec<(String, usize)>> {
        let c = self.r();
        let mut st = c.prepare_cached(
            "SELECT category, COUNT(*) FROM messages WHERE folder='INBOX' AND seen=0 GROUP BY category",
        )?;
        let v = st
            .query_map([], |r| Ok((r.get(0)?, r.get::<_, i64>(1)? as usize)))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(v)
    }

    pub fn remove_account(&self, account: &str) -> Result<()> {
        let c = self.w();
        c.execute("DELETE FROM messages WHERE account=?1", [account])?;
        c.execute("DELETE FROM folders WHERE account=?1", [account])?;
        Ok(())
    }
}

pub fn snippet(text: &str) -> String {
    let mut out = String::with_capacity(200);
    for word in text.split_whitespace() {
        if out.len() > 180 {
            break;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word.trim_start_matches('>'));
    }
    out.chars().take(160).collect()
}
