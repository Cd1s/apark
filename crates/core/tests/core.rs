use apark_core::account::{preset, Server};
use apark_core::engine::emails_in;
use apark_core::imap::{parse_body, parse_header};
use apark_core::store::{Body, NewMsg, Store};
use apark_core::ListQuery;

const NEWSLETTER: &[u8] = b"From: Weekly <news@example.com>\r\nTo: Me <me@example.org>, \"Doe, John\" <john@example.org>\r\n\
Subject: =?UTF-8?B?5L2g5aW9?= digest\r\nDate: Tue, 07 Oct 2026 10:00:00 +0000\r\nMessage-ID: <abc@example.com>\r\n\
List-Unsubscribe: <mailto:unsub@example.com>\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<p>Hello <b>world</b></p>\r\n";

const PERSONAL: &[u8] = b"From: Alice <alice@example.com>\r\nTo: me@example.org\r\nSubject: lunch?\r\n\
Date: Wed, 08 Oct 2026 09:00:00 +0800\r\nMessage-ID: <m1@example.com>\r\nIn-Reply-To: <m0@example.com>\r\n\
Content-Type: multipart/mixed; boundary=XX\r\n\r\n--XX\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n\
noon at the usual place\r\n--XX\r\nContent-Type: application/pdf\r\nContent-Disposition: attachment; filename=menu.pdf\r\n\
Content-Transfer-Encoding: base64\r\n\r\nJVBERi0xLjQK\r\n--XX--\r\n";

#[test]
fn headers_and_categories() {
    let h = parse_header(NEWSLETTER);
    assert_eq!(h.subject, "你好 digest");
    assert_eq!(h.from_addr, "news@example.com");
    assert_eq!(h.category, "newsletter");
    assert_eq!(h.message_id, "<abc@example.com>");
    assert_eq!(emails_in(&h.to), vec!["me@example.org", "john@example.org"]);

    let p = parse_header(PERSONAL);
    assert_eq!(p.category, "people");
    assert_eq!(p.in_reply_to, "<m0@example.com>");
    assert_eq!(p.from_name, "Alice");
}

#[test]
fn bodies() {
    let b = parse_body(NEWSLETTER);
    assert!(b.html.is_some());
    assert!(b.text.contains("Hello"));
    let p = parse_body(PERSONAL);
    assert!(p.html.is_none());
    assert_eq!(p.text.trim(), "noon at the usual place");
    assert_eq!(p.attachments.len(), 1);
    assert_eq!(p.attachments[0].name, "menu.pdf");
}

#[test]
fn store_roundtrip_and_search() {
    let dir = std::env::temp_dir().join(format!("apark-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let store = Store::open(&dir.join("t.db")).unwrap();
    let msgs = vec![
        NewMsg { uid: 1, seen: false, flagged: false, size: 100, header: parse_header(NEWSLETTER) },
        NewMsg { uid: 2, seen: true, flagged: true, size: 100, header: parse_header(PERSONAL) },
    ];
    store.insert_headers("me@example.org", "INBOX", &msgs).unwrap();
    store.set_body("me@example.org", "INBOX", 2, &parse_body(PERSONAL)).unwrap();

    let all = store.list(&ListQuery::default()).unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].uid, 2, "newest first");
    assert_eq!(all[0].snippet, "noon at the usual place");

    let q = |s: &str| store.list(&ListQuery { search: Some(s.into()), ..Default::default() }).unwrap();
    assert_eq!(q("usual noon").len(), 1);
    assert_eq!(q("100%").len(), 0);
    assert_eq!(q("alice").len(), 1);
    let unread = store.list(&ListQuery { unread: true, ..Default::default() }).unwrap();
    assert_eq!(unread.len(), 1);
    let news = store.list(&ListQuery { category: Some("newsletter".into()), ..Default::default() }).unwrap();
    assert_eq!(news[0].uid, 1);

    store.update_flags("me@example.org", "INBOX", &[(1, true, false)]).unwrap();
    assert!(store.unread_counts().unwrap().is_empty());
    store.delete_uids("me@example.org", "INBOX", &[1]).unwrap();
    assert_eq!(store.known_uids("me@example.org", "INBOX").unwrap(), vec![2]);
    assert!(matches!(store.body(all[0].id).unwrap(), Some(Body { .. })));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn servers() {
    assert_eq!(preset("a@qq.com").0, Server::new("imap.qq.com", 993));
    assert_eq!(preset("a@corp.io").1, Server::new("smtp.corp.io", 465));
    assert_eq!(Server::parse("mail.x.com:587", 465).unwrap(), Server::new("mail.x.com", 587));
    assert_eq!(Server::parse("mail.x.com", 993).unwrap().port, 993);
}

#[test]
fn modified_utf7_folder_names() {
    use apark_core::imap::decode_mutf7;
    assert_eq!(decode_mutf7("&XfJSoGYfaAc-"), "已加星标");
    assert_eq!(decode_mutf7("[Gmail]/&V4NXPpCuTvY-"), "[Gmail]/垃圾邮件");
    assert_eq!(decode_mutf7("Tom &- Jerry"), "Tom & Jerry");
    assert_eq!(decode_mutf7("Notes"), "Notes");
    assert_eq!(decode_mutf7("broken &zz"), "broken &zz");
}
