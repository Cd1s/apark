//! The JSON RPC used by the native UIs, driven against demo data (no network).

use std::sync::{Arc, Mutex};

use apark_core::rpc::Rpc;
use apark_core::Engine;
use serde_json::{json, Value};

async fn call(rpc: &Arc<Rpc>, method: &str, params: Value) -> Value {
    let reply: Value = serde_json::from_str(&rpc.handle(&json!({ "method": method, "params": params }).to_string()).await).unwrap();
    assert_eq!(reply["ok"], true, "{method} failed: {reply}");
    reply["result"].clone()
}

#[tokio::test]
async fn rpc_against_demo_mailbox() {
    let home = std::env::temp_dir().join(format!("apark-rpc-{}", std::process::id()));
    std::env::set_var("APARK_HOME", &home);
    let eng = Engine::open().unwrap();
    apark_core::demo::seed(&eng).unwrap();
    let events = Arc::new(Mutex::new(vec![]));
    let sink = events.clone();
    let rpc = Rpc::new(eng, Arc::new(move |e| sink.lock().unwrap().push(e)));

    let info = call(&rpc, "info", json!({})).await;
    assert_eq!(info["has_master"], true);
    assert_eq!(call(&rpc, "accounts", json!({})).await.as_array().unwrap().len(), 3);

    let inbox = call(&rpc, "list", json!({})).await;
    assert_eq!(inbox.as_array().unwrap().len(), 10);
    let people = call(&rpc, "list", json!({ "category": "people" })).await;
    assert!(people.as_array().unwrap().iter().all(|m| m["category"] == "people"));
    let found = call(&rpc, "list", json!({ "search": "预算" })).await;
    assert_eq!(found[0]["subject"], "Re: Q4 预算");

    let id = inbox[0]["id"].as_i64().unwrap();
    let body = call(&rpc, "cached_body", json!({ "id": id })).await;
    assert!(body["text"].as_str().unwrap().contains("评审"));

    let before = call(&rpc, "unread_counts", json!({})).await;
    call(&rpc, "set_seen", json!({ "id": id, "seen": true })).await;
    let after = call(&rpc, "unread_counts", json!({})).await;
    assert_eq!(after["people"].as_i64().unwrap(), before["people"].as_i64().unwrap() - 1);

    call(&rpc, "archive", json!({ "id": id })).await;
    assert_eq!(call(&rpc, "list", json!({})).await.as_array().unwrap().len(), 9);

    let draft = call(&rpc, "reply_draft", json!({ "id": inbox[1]["id"], "all": false })).await;
    assert_eq!(draft["draft"]["to"][0], "noreply@github.com");
    assert!(draft["draft"]["subject"].as_str().unwrap().starts_with("Re: "));

    let sync = call(&rpc, "sync_all", json!({})).await;
    assert!(sync.as_array().unwrap().iter().all(|r| r["ok"] == true));
    assert!(events.lock().unwrap().iter().any(|e| e["type"] == "synced"));

    let bad: Value = serde_json::from_str(&rpc.handle(r#"{"method":"nope"}"#).await).unwrap();
    assert_eq!(bad["ok"], false);
    let _ = std::fs::remove_dir_all(&home);
}
