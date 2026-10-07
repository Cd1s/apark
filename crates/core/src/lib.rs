//! Apark core: everything that is not UI.
//!
//! The desktop app and the headless CLI are thin shells over [`Engine`].

pub mod account;
pub mod categorize;
pub mod cloud;
pub mod config;
pub mod engine;
pub mod imap;
pub mod oauth;
pub mod paths;
pub mod smtp;
pub mod store;

pub use account::{Account, Auth, Provider, Server};
pub use config::Config;
pub use engine::{Engine, LoginOpts};
pub use smtp::Outgoing;
pub use store::{Body, Folder, ListQuery, MsgRow};

/// Install the process-wide rustls crypto provider. Safe to call more than once.
pub fn init_crypto() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

pub fn now() -> i64 {
    chrono::Utc::now().timestamp()
}
