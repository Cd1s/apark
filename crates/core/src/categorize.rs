//! Smart Inbox buckets, decided from headers alone so it works before bodies arrive.

use mail_parser::Message;

pub const PEOPLE: &str = "people";
pub const NOTIFICATION: &str = "notification";
pub const NEWSLETTER: &str = "newsletter";

pub fn label(cat: &str) -> &'static str {
    match cat {
        NOTIFICATION => "通知",
        NEWSLETTER => "订阅",
        _ => "个人",
    }
}

pub fn categorize(msg: &Message, from_addr: &str) -> &'static str {
    let has = |h: &str| msg.header_raw(h).is_some();
    if has("List-Unsubscribe") || has("List-Id") {
        return NEWSLETTER;
    }
    if let Some(p) = msg.header_raw("Precedence") {
        let p = p.trim().to_ascii_lowercase();
        if p == "bulk" || p == "list" {
            return NEWSLETTER;
        }
    }
    if msg.header_raw("Auto-Submitted").is_some_and(|v| !v.trim().eq_ignore_ascii_case("no")) {
        return NOTIFICATION;
    }
    let local = from_addr.split('@').next().unwrap_or("").to_ascii_lowercase();
    const MACHINE: [&str; 9] =
        ["noreply", "no-reply", "no_reply", "donotreply", "do-not-reply", "notification", "notify", "alert", "mailer-daemon"];
    if MACHINE.iter().any(|m| local.contains(m)) || local == "info" || local == "support" {
        return NOTIFICATION;
    }
    PEOPLE
}
