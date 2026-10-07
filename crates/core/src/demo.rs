//! Sample mailbox for screenshots and UI development (`apark dev-seed`).
//! Accounts point at unreachable servers; run the UI with APARK_NO_SYNC=1.

use anyhow::Result;

use crate::account::{Account, Auth, Provider, Server};
use crate::store::{Body, Header, NewMsg};
use crate::Engine;

const MAILS: &[(&str, &str, &str, &str, &str, i64, bool, bool, &str)] = &[
    // account, from name, from addr, subject, body, minutes ago, seen, flagged, category
    ("me@gmail.com", "林晓", "lin.xiao@example.com", "周五的设计评审改到下午三点", "嗨，\n\n周五的评审会改到下午三点，地点不变。新版原型我放在共享文件夹里了，麻烦提前看一下导航部分。\n\n谢谢！\n林晓", 12, false, true, "people"),
    ("work@company.com", "GitHub", "noreply@github.com", "[Cd1s/apark] CI passed on main", "All checks have passed for commit 7ec0267.\n\nbuild / macos-arm64 ✓\nbuild / windows-x64 ✓\nbuild / linux-x64 ✓", 35, false, false, "notification"),
    ("me@gmail.com", "Sarah Chen", "sarah@example.org", "Trip photos 📸", "Hey! Finally uploaded the photos from Kyoto. The ones from Fushimi Inari came out amazing.\n\nLet me know which ones you want printed.\n\nSarah", 80, false, false, "people"),
    ("work@company.com", "王磊", "wanglei@company.com", "Re: Q4 预算", "预算表我更新了第二页的人力成本，差额主要来自新招的两个岗位。周一前确认即可。\n\n王磊", 140, true, false, "people"),
    ("me@icloud.com", "Apple", "no_reply@email.apple.com", "你的收据：iCloud+ 50GB", "感谢你的购买。\n\niCloud+ 50GB 存储空间  ¥6.00\n\n此订阅将自动续期。", 300, true, false, "notification"),
    ("me@gmail.com", "少数派", "newsletter@sspai.com", "本周值得关注的效率工具", "这一期我们聊聊邮件客户端：为什么越来越多的人开始回归原生应用……", 600, false, false, "newsletter"),
    ("work@company.com", "Jira", "jira@company.atlassian.net", "[APARK-42] 统一收件箱的未读计数不准确", "王磊 将此问题分配给了你。\n\n优先级：高", 900, true, false, "notification"),
    ("me@gmail.com", "Mom", "mom@example.com", "周末回家吃饭吗", "周末回来吗？给你做红烧肉。", 1500, true, true, "people"),
    ("me@icloud.com", "The Verge", "newsletter@theverge.com", "Command Line: the week in tech", "This week: new chips, a surprising acquisition, and why native apps still feel better.", 2600, true, false, "newsletter"),
    ("work@company.com", "HR", "hr@company.com", "十一假期安排通知", "各位同事：\n\n国庆假期为 10 月 1 日至 10 月 7 日，请提前安排好工作交接。", 4300, true, false, "people"),
];

pub fn seed(eng: &Engine) -> Result<usize> {
    for (email, name, provider) in [
        ("me@gmail.com", "Cheek", Provider::Google),
        ("work@company.com", "Cheek Xie", Provider::Imap),
        ("me@icloud.com", "", Provider::Imap),
    ] {
        eng.insert_account_unchecked(Account {
            email: email.into(),
            name: name.into(),
            provider,
            imap: Server::new("demo.invalid", 993),
            smtp: Server::new("demo.invalid", 465),
            auth: Auth::Password { password: String::new() },
            master: email == "me@gmail.com",
        })?;
    }
    let now = crate::now();
    for (i, (acct, from_name, from_addr, subject, body, ago, seen, flagged, cat)) in MAILS.iter().enumerate() {
        let uid = 1000 + i as u32;
        let header = Header {
            message_id: format!("<demo{i}@apark>"),
            subject: (*subject).into(),
            from_name: (*from_name).into(),
            from_addr: (*from_addr).into(),
            to: (*acct).into(),
            date: now - ago * 60,
            category: (*cat).into(),
            ..Default::default()
        };
        eng.store.insert_headers(acct, "INBOX", &[NewMsg { uid, seen: *seen, flagged: *flagged, size: 2000, header }])?;
        eng.store.set_body(acct, "INBOX", uid, &Body { text: (*body).into(), html: None, attachments: vec![] })?;
    }
    for acct in ["me@gmail.com", "work@company.com", "me@icloud.com"] {
        let folders: Vec<crate::Folder> = [("INBOX", "inbox"), ("Sent", "sent"), ("Archive", "archive"), ("Trash", "trash")]
            .iter()
            .map(|(n, r)| crate::Folder { name: (*n).into(), role: (*r).into() })
            .collect();
        eng.store.save_folders(acct, &folders)?;
    }
    Ok(MAILS.len())
}
