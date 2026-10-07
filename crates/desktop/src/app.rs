//! Desktop UI. All data comes from the local store (synchronous, ~1 ms);
//! network work runs on a tokio runtime and reports back through a channel.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use apark_core::{categorize, Account, Body, Config, Engine, Folder, ListQuery, LoginOpts, MsgRow, Outgoing, Provider, Server};
use eframe::egui::{self, Color32, Key, RichText};

use crate::widgets::{self, ROW_HEIGHT};

pub fn run() -> anyhow::Result<()> {
    let eng = Engine::open()?;
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Apark")
            .with_inner_size([1200.0, 780.0])
            .with_min_inner_size([760.0, 480.0])
            .with_icon(Arc::new(widgets::icon(128))),
        ..Default::default()
    };
    eframe::run_native("Apark", options, Box::new(move |cc| Ok(Box::new(App::new(cc, eng)))))
        .map_err(|e| anyhow::anyhow!("{e}"))
}

#[derive(Clone, PartialEq)]
enum Nav {
    Smart(Option<&'static str>),
    Unread,
    Flagged,
    Folder { account: String, folder: String },
}

enum Ev {
    Synced(Vec<String>),
    Body(i64, Result<Body, String>),
    Done(Result<String, String>),
    Accounts(Result<String, String>),
    LoginUrl(String),
    Draft(Result<(String, Outgoing), String>),
    Sent(u64, Result<(), String>),
}

struct Composer {
    id: u64,
    from: String,
    to: String,
    cc: String,
    bcc: String,
    subject: String,
    body: String,
    attachments: Vec<PathBuf>,
    in_reply_to: Option<String>,
    references: Option<String>,
    show_cc: bool,
    sending: bool,
    error: Option<String>,
}

#[derive(Default)]
struct AddDialog {
    email: String,
    password: String,
    name: String,
    imap: String,
    smtp: String,
    advanced: bool,
}

struct App {
    eng: Arc<Engine>,
    rt: tokio::runtime::Runtime,
    tx: Sender<Ev>,
    rx: Receiver<Ev>,

    nav: Nav,
    search: String,
    reload_at: Option<Instant>,
    rows: Vec<MsgRow>,
    selected: Option<i64>,
    scroll_to_selected: bool,
    body: Option<(i64, Body)>,
    body_error: Option<String>,

    accounts: Vec<Account>,
    folders: HashMap<String, Vec<Folder>>,
    expanded: HashSet<String>,
    unread: HashMap<String, usize>,

    syncing: bool,
    last_sync: Option<Instant>,
    status: String,
    error: Option<String>,

    composers: Vec<Composer>,
    next_composer: u64,
    add: Option<AddDialog>,
    settings: Option<Config>,
    busy: Option<String>,
    login_url: Option<String>,
    focus_search: bool,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>, eng: Arc<Engine>) -> App {
        crate::fonts::install(&cc.egui_ctx);
        cc.egui_ctx.all_styles_mut(|s| {
            s.spacing.item_spacing = egui::vec2(8.0, 6.0);
            s.spacing.button_padding = egui::vec2(10.0, 5.0);
        });
        let (tx, rx) = channel();
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("tokio runtime");
        let mut app = App {
            eng,
            rt,
            tx,
            rx,
            nav: Nav::Smart(None),
            search: String::new(),
            reload_at: None,
            rows: vec![],
            selected: None,
            scroll_to_selected: false,
            body: None,
            body_error: None,
            accounts: vec![],
            folders: HashMap::new(),
            expanded: HashSet::new(),
            unread: HashMap::new(),
            syncing: false,
            last_sync: None,
            status: String::new(),
            error: None,
            composers: vec![],
            next_composer: 1,
            add: None,
            settings: None,
            busy: None,
            login_url: None,
            focus_search: false,
        };
        app.reload_accounts();
        app.reload();
        app
    }

    fn spawn<F>(&self, ctx: &egui::Context, fut: F)
    where
        F: Future<Output = Ev> + Send + 'static,
    {
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        self.rt.spawn(async move {
            let _ = tx.send(fut.await);
            ctx.request_repaint();
        });
    }

    fn login_opts(&self, ctx: &egui::Context) -> LoginOpts {
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        LoginOpts {
            manual: false,
            on_url: Arc::new(move |url| {
                let _ = tx.send(Ev::LoginUrl(url.to_owned()));
                ctx.request_repaint();
            }),
        }
    }

    // ---- data -----------------------------------------------------------

    fn reload_accounts(&mut self) {
        self.accounts = self.eng.accounts();
        self.folders = self
            .accounts
            .iter()
            .map(|a| (a.email.clone(), self.eng.folders(&a.email).unwrap_or_default()))
            .collect();
    }

    fn query(&self) -> ListQuery {
        let search = Some(self.search.trim().to_owned()).filter(|s| !s.is_empty());
        let mut q = ListQuery { search, limit: 5000, ..Default::default() };
        match &self.nav {
            Nav::Smart(cat) => {
                q.category = cat.map(str::to_owned);
                if q.search.is_some() {
                    q.folder = None;
                }
            }
            Nav::Unread => q.unread = true,
            Nav::Flagged => q.flagged = true,
            Nav::Folder { account, folder } => {
                q.account = Some(account.clone());
                q.folder = Some(folder.clone());
            }
        }
        if matches!(self.nav, Nav::Unread | Nav::Flagged) && q.search.is_none() {
            q.folder = Some("INBOX".into());
        }
        q
    }

    fn reload(&mut self) {
        match self.eng.list(&self.query()) {
            Ok(rows) => self.rows = rows,
            Err(e) => self.error = Some(format!("{e:#}")),
        }
        self.unread = self.eng.store.unread_counts().unwrap_or_default().into_iter().collect();
    }

    fn selected_row(&self) -> Option<&MsgRow> {
        self.selected.and_then(|id| self.rows.iter().find(|m| m.id == id))
    }

    fn select(&mut self, ctx: &egui::Context, id: i64) {
        if self.selected == Some(id) {
            return;
        }
        self.selected = Some(id);
        self.body_error = None;
        self.body = None;
        match self.eng.store.body(id) {
            Ok(Some(b)) => self.body = Some((id, b)),
            _ => {
                let eng = self.eng.clone();
                self.spawn(ctx, async move { Ev::Body(id, eng.body(id).await.map_err(|e| format!("{e:#}"))) });
            }
        }
        // Opening a message marks it read, like Spark.
        if let Some(m) = self.rows.iter_mut().find(|m| m.id == id) {
            if !m.seen {
                m.seen = true;
                let eng = self.eng.clone();
                self.spawn(ctx, async move { Ev::Done(eng.set_seen(id, true).await.map(|_| String::new()).map_err(|e| format!("{e:#}"))) });
            }
        }
    }

    fn move_selection(&mut self, ctx: &egui::Context, delta: isize) {
        if self.rows.is_empty() {
            return;
        }
        let idx = self.selected.and_then(|id| self.rows.iter().position(|m| m.id == id));
        let next = match idx {
            Some(i) => (i as isize + delta).clamp(0, self.rows.len() as isize - 1) as usize,
            None => 0,
        };
        let id = self.rows[next].id;
        self.select(ctx, id);
        self.scroll_to_selected = true;
    }

    fn start_sync(&mut self, ctx: &egui::Context) {
        if self.syncing || self.accounts.is_empty() {
            return;
        }
        self.syncing = true;
        self.last_sync = Some(Instant::now());
        let eng = self.eng.clone();
        let extra = match &self.nav {
            Nav::Folder { account, folder } if folder != "INBOX" => Some((account.clone(), folder.clone())),
            _ => None,
        };
        self.spawn(ctx, async move {
            let mut errors = vec![];
            if let Some(results) = eng.sync_all().await {
                for (acct, r) in results {
                    if let Err(e) = r {
                        errors.push(format!("{acct}: {e:#}"));
                    }
                }
            }
            if let Some((a, f)) = extra {
                if let Err(e) = eng.sync_account(&a, &[f]).await {
                    errors.push(format!("{a}: {e:#}"));
                }
            }
            Ev::Synced(errors)
        });
    }

    // ---- actions on the selected message --------------------------------

    /// Remove from the list right away, select the neighbour, then do the network part.
    fn take_out(&mut self, ctx: &egui::Context, id: i64, label: &'static str, op: fn(Arc<Engine>, i64) -> futures_lite::BoxedOp) {
        let idx = self.rows.iter().position(|m| m.id == id);
        if let Some(i) = idx {
            self.rows.remove(i);
            self.body = None;
            self.selected = None;
            if !self.rows.is_empty() {
                let next = self.rows[i.min(self.rows.len() - 1)].id;
                self.select(ctx, next);
            }
        }
        let fut = op(self.eng.clone(), id);
        self.spawn(ctx, async move { Ev::Done(fut.await.map(|_| label.to_owned()).map_err(|e| format!("{e:#}"))) });
    }

    fn archive(&mut self, ctx: &egui::Context, id: i64) {
        self.take_out(ctx, id, "已归档", |eng, id| Box::pin(async move { eng.archive(id).await }));
    }

    fn trash(&mut self, ctx: &egui::Context, id: i64) {
        self.take_out(ctx, id, "已移到废纸篓", |eng, id| Box::pin(async move { eng.trash(id).await }));
    }

    fn toggle_seen(&mut self, ctx: &egui::Context, id: i64) {
        let Some(m) = self.rows.iter_mut().find(|m| m.id == id) else { return };
        m.seen = !m.seen;
        let seen = m.seen;
        let eng = self.eng.clone();
        self.spawn(ctx, async move { Ev::Done(eng.set_seen(id, seen).await.map(|_| String::new()).map_err(|e| format!("{e:#}"))) });
    }

    fn toggle_flag(&mut self, ctx: &egui::Context, id: i64) {
        let Some(m) = self.rows.iter_mut().find(|m| m.id == id) else { return };
        m.flagged = !m.flagged;
        let flagged = m.flagged;
        let eng = self.eng.clone();
        self.spawn(ctx, async move { Ev::Done(eng.set_flagged(id, flagged).await.map(|_| String::new()).map_err(|e| format!("{e:#}"))) });
    }

    fn compose(&mut self, from: Option<String>, draft: Outgoing) {
        let from = from.or_else(|| self.accounts.first().map(|a| a.email.clone())).unwrap_or_default();
        let show_cc = !draft.cc.is_empty();
        self.composers.push(Composer {
            id: self.next_composer,
            from,
            to: draft.to.join(", "),
            cc: draft.cc.join(", "),
            bcc: String::new(),
            subject: draft.subject,
            body: draft.body,
            attachments: draft.attachments,
            in_reply_to: draft.in_reply_to,
            references: draft.references,
            show_cc,
            sending: false,
            error: None,
        });
        self.next_composer += 1;
    }

    fn reply(&mut self, ctx: &egui::Context, id: i64, all: bool) {
        let eng = self.eng.clone();
        self.spawn(ctx, async move { Ev::Draft(eng.reply_draft(id, all).await.map_err(|e| format!("{e:#}"))) });
    }

    fn forward(&mut self, ctx: &egui::Context, id: i64) {
        let eng = self.eng.clone();
        self.spawn(ctx, async move { Ev::Draft(eng.forward_draft(id).await.map_err(|e| format!("{e:#}"))) });
    }

    // ---- event pump -----------------------------------------------------

    fn pump(&mut self, ctx: &egui::Context) {
        while let Ok(ev) = self.rx.try_recv() {
            match ev {
                Ev::Synced(errors) => {
                    self.syncing = false;
                    self.status = format!("已同步 {}", chrono::Local::now().format("%H:%M"));
                    self.error = errors.first().cloned();
                    self.reload_accounts();
                    self.reload();
                }
                Ev::Body(id, r) => {
                    if self.selected == Some(id) {
                        match r {
                            Ok(b) => self.body = Some((id, b)),
                            Err(e) => self.body_error = Some(e),
                        }
                    }
                    self.reload();
                }
                Ev::Done(r) => {
                    match r {
                        Ok(msg) if !msg.is_empty() => self.status = msg,
                        Ok(_) => {}
                        Err(e) => self.error = Some(e),
                    }
                    self.reload();
                }
                Ev::Accounts(r) => {
                    self.busy = None;
                    self.login_url = None;
                    match r {
                        Ok(msg) => {
                            self.status = msg;
                            self.add = None;
                            self.error = None;
                        }
                        Err(e) => self.error = Some(e),
                    }
                    self.reload_accounts();
                    self.reload();
                    self.syncing = false;
                    self.start_sync(ctx);
                }
                Ev::LoginUrl(url) => self.login_url = Some(url),
                Ev::Draft(Ok((from, d))) => self.compose(Some(from), d),
                Ev::Draft(Err(e)) => self.error = Some(e),
                Ev::Sent(cid, r) => match r {
                    Ok(()) => {
                        self.composers.retain(|c| c.id != cid);
                        self.status = "已发送".into();
                    }
                    Err(e) => {
                        if let Some(c) = self.composers.iter_mut().find(|c| c.id == cid) {
                            c.sending = false;
                            c.error = Some(e);
                        }
                    }
                },
            }
        }
    }

    // ---- keyboard -------------------------------------------------------

    fn shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() || !self.composers.is_empty() && ctx.memory(|m| m.focused().is_some()) {
            if ctx.input(|i| i.key_pressed(Key::Escape)) {
                ctx.memory_mut(|m| {
                    if let Some(id) = m.focused() {
                        m.surrender_focus(id)
                    }
                });
            }
            return;
        }
        let (keys, cmd) = ctx.input(|i| {
            let keys: Vec<Key> = [Key::J, Key::K, Key::ArrowDown, Key::ArrowUp, Key::E, Key::Delete, Key::Backspace,
                Key::R, Key::A, Key::F, Key::C, Key::N, Key::U, Key::S, Key::Slash]
                .into_iter()
                .filter(|k| i.key_pressed(*k))
                .collect();
            (keys, i.modifiers.command || i.modifiers.ctrl)
        });
        if cmd {
            return;
        }
        let sel = self.selected;
        for k in keys {
            match (k, sel) {
                (Key::J | Key::ArrowDown, _) => self.move_selection(ctx, 1),
                (Key::K | Key::ArrowUp, _) => self.move_selection(ctx, -1),
                (Key::E, Some(id)) => self.archive(ctx, id),
                (Key::Delete | Key::Backspace, Some(id)) => self.trash(ctx, id),
                (Key::R, Some(id)) => self.reply(ctx, id, false),
                (Key::A, Some(id)) => self.reply(ctx, id, true),
                (Key::F, Some(id)) => self.forward(ctx, id),
                (Key::U, Some(id)) => self.toggle_seen(ctx, id),
                (Key::S, Some(id)) => self.toggle_flag(ctx, id),
                (Key::C | Key::N, _) => self.compose(None, Outgoing::default()),
                (Key::Slash, _) => self.focus_search = true,
                _ => {}
            }
        }
    }

    // ---- panels ---------------------------------------------------------

    fn nav_item(&mut self, ui: &mut egui::Ui, nav: Nav, label: &str, count: usize) {
        let text = if count > 0 { format!("{label}  {count}") } else { label.to_owned() };
        if ui.selectable_label(self.nav == nav, text).clicked() && self.nav != nav {
            self.nav = nav;
            self.selected = None;
            self.body = None;
            self.reload();
        }
    }

    fn sidebar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.heading(RichText::new("Apark").strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("✏ 写邮件").on_hover_text("快捷键 C").clicked() {
                    self.compose(None, Outgoing::default());
                }
            });
        });
        ui.add_space(6.0);
        let total: usize = self.unread.values().sum();
        let count = |c: &str| self.unread.get(c).copied().unwrap_or(0);
        let (people, notif, news) = (count("people"), count("notification"), count("newsletter"));
        self.nav_item(ui, Nav::Smart(None), "📥 智能收件箱", total);
        ui.indent("smart", |ui| {
            self.nav_item(ui, Nav::Smart(Some(categorize::PEOPLE)), "👤 个人", people);
            self.nav_item(ui, Nav::Smart(Some(categorize::NOTIFICATION)), "🔔 通知", notif);
            self.nav_item(ui, Nav::Smart(Some(categorize::NEWSLETTER)), "📰 订阅", news);
        });
        self.nav_item(ui, Nav::Unread, "● 未读", 0);
        self.nav_item(ui, Nav::Flagged, "★ 星标", 0);
        ui.separator();

        egui::ScrollArea::vertical().auto_shrink([false, true]).max_height(ui.available_height() - 70.0).show(ui, |ui| {
            for acct in self.accounts.clone() {
                let open = self.expanded.contains(&acct.email);
                let resp = ui
                    .horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                        ui.painter().circle_filled(r.center(), 4.0, widgets::account_color(&acct.email));
                        let label = format!("{} {}{}", if open { "⏷" } else { "⏵" }, acct.email, if acct.master { " ⭐" } else { "" });
                        ui.selectable_label(false, label)
                    })
                    .inner;
                if resp.clicked() {
                    if open {
                        self.expanded.remove(&acct.email);
                    } else {
                        self.expanded.insert(acct.email.clone());
                    }
                }
                let email = acct.email.clone();
                resp.context_menu(|ui| {
                    ui.label(RichText::new(acct.provider.label()).weak());
                    if ui.button("立即同步").clicked() {
                        self.start_sync(ctx);
                        ui.close();
                    }
                    if ui.button("删除账号").clicked() {
                        let eng = self.eng.clone();
                        let e = email.clone();
                        self.spawn(ctx, async move {
                            Ev::Accounts(eng.remove_account(&e).await.map(|_| format!("已删除 {e}")).map_err(|e| format!("{e:#}")))
                        });
                        ui.close();
                    }
                });
                if open {
                    let folders = self.folders.get(&acct.email).cloned().unwrap_or_default();
                    ui.indent(("folders", &acct.email), |ui| {
                        if folders.is_empty() {
                            ui.label(RichText::new("同步后显示文件夹").weak());
                        }
                        for f in folders {
                            let label = folder_label(&f);
                            let nav = Nav::Folder { account: acct.email.clone(), folder: f.name.clone() };
                            if ui.selectable_label(self.nav == nav, label).clicked() && self.nav != nav {
                                self.nav = nav;
                                self.selected = None;
                                self.body = None;
                                self.reload();
                                if f.name != "INBOX" {
                                    let eng = self.eng.clone();
                                    let (a, name) = (acct.email.clone(), f.name.clone());
                                    self.spawn(ctx, async move {
                                        Ev::Done(eng.sync_account(&a, &[name]).await.map(|_| String::new()).map_err(|e| format!("{e:#}")))
                                    });
                                }
                            }
                        }
                    });
                }
            }
        });

        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            ui.add_space(6.0);
            let status = if self.syncing { "⟳ 同步中…".to_owned() } else { self.status.clone() };
            ui.label(RichText::new(status).small().weak());
            ui.horizontal(|ui| {
                if ui.button("＋ 添加账号").clicked() {
                    self.add = Some(AddDialog::default());
                }
                if ui.button("⚙").on_hover_text("设置").clicked() {
                    self.settings = Some(self.eng.config());
                }
            });
        });
    }

    fn list_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let search = ui.add(
                egui::TextEdit::singleline(&mut self.search)
                    .hint_text("🔍 搜索  /")
                    .desired_width(ui.available_width() - 40.0),
            );
            if self.focus_search {
                search.request_focus();
                self.focus_search = false;
            }
            if search.changed() {
                self.reload_at = Some(Instant::now() + Duration::from_millis(120));
            }
            let sync = ui.add_enabled(!self.syncing, egui::Button::new("⟳")).on_hover_text("同步");
            if sync.clicked() {
                self.start_sync(ctx);
            }
        });
        ui.add_space(4.0);
        if self.rows.is_empty() {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new(if self.syncing { "正在同步…" } else { "这里没有邮件 🎉" }).weak());
            });
            return;
        }
        let show_account = self.accounts.len() > 1 && !matches!(self.nav, Nav::Folder { .. });
        let selected = self.selected;
        let mut clicked = None;
        let mut action: Option<(i64, &'static str)> = None;
        let mut scroll = egui::ScrollArea::vertical().auto_shrink([false, false]);
        if std::mem::take(&mut self.scroll_to_selected) {
            if let Some(i) = selected.and_then(|id| self.rows.iter().position(|m| m.id == id)) {
                let row_h = ROW_HEIGHT + ui.spacing().item_spacing.y;
                let top = i as f32 * row_h;
                let view_h = ui.available_height();
                let cur = ui.ctx().data(|d| d.get_temp::<f32>(egui::Id::new("list_offset"))).unwrap_or(0.0);
                if top < cur || top + row_h > cur + view_h {
                    scroll = scroll.vertical_scroll_offset((top - view_h / 2.0).max(0.0));
                }
            }
        }
        let out = scroll.show_rows(ui, ROW_HEIGHT, self.rows.len(), |ui, range| {
            for m in &self.rows[range] {
                let resp = widgets::message_row(ui, m, selected == Some(m.id), show_account);
                if resp.clicked() {
                    clicked = Some(m.id);
                }
                resp.context_menu(|ui| {
                    for (label, a) in [("回复", "reply"), ("转发", "forward"), (if m.seen { "标为未读" } else { "标为已读" }, "seen"),
                        (if m.flagged { "取消星标" } else { "加星标" }, "flag"), ("归档", "archive"), ("删除", "trash")]
                    {
                        if ui.button(label).clicked() {
                            action = Some((m.id, a));
                            ui.close();
                        }
                    }
                });
            }
        });
        ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("list_offset"), out.state.offset.y));
        if let Some(id) = clicked {
            self.select(ctx, id);
        }
        if let Some((id, a)) = action {
            self.run_action(ctx, id, a);
        }
    }

    fn run_action(&mut self, ctx: &egui::Context, id: i64, action: &str) {
        match action {
            "reply" => self.reply(ctx, id, false),
            "reply_all" => self.reply(ctx, id, true),
            "forward" => self.forward(ctx, id),
            "seen" => self.toggle_seen(ctx, id),
            "flag" => self.toggle_flag(ctx, id),
            "archive" => self.archive(ctx, id),
            "trash" => self.trash(ctx, id),
            _ => {}
        }
    }

    fn reader(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let Some(m) = self.selected_row().cloned() else {
            ui.centered_and_justified(|ui| {
                ui.label(RichText::new("选择一封邮件\n\nJ/K 上下 · E 归档 · R 回复 · C 写邮件 · / 搜索").weak());
            });
            return;
        };
        ui.add_space(10.0);
        ui.horizontal_wrapped(|ui| {
            let mut act = None;
            for (label, a, tip) in [("↩ 回复", "reply", "R"), ("↩↩ 全部", "reply_all", "A"), ("➡ 转发", "forward", "F"),
                ("🗄 归档", "archive", "E"), ("🗑 删除", "trash", "Delete"),
                (if m.seen { "● 未读" } else { "○ 已读" }, "seen", "U"), (if m.flagged { "★" } else { "☆" }, "flag", "S")]
            {
                if ui.button(label).on_hover_text(tip).clicked() {
                    act = Some(a);
                }
            }
            if let Some(a) = act {
                self.run_action(ctx, m.id, a);
            }
            if let Some((_, b)) = &self.body {
                if let Some(html) = &b.html {
                    if ui.button("🌐 浏览器打开").on_hover_text("用浏览器查看原始排版").clicked() {
                        let path = std::env::temp_dir().join(format!("apark-{}.html", m.id));
                        let page = format!("<meta charset=\"utf-8\"><title>{}</title>{html}", m.subject.replace('<', "&lt;"));
                        if std::fs::write(&path, page).is_ok() {
                            let _ = webbrowser::open(&path.to_string_lossy());
                        }
                    }
                }
            }
        });
        ui.separator();
        ui.add_space(4.0);
        ui.add(egui::Label::new(RichText::new(if m.subject.is_empty() { "（无主题）" } else { &m.subject }).size(20.0).strong()).wrap());
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            widgets::avatar(ui, m.sender(), widgets::account_color(&m.from_addr), 38.0);
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(m.sender()).strong());
                    ui.label(RichText::new(format!("<{}>", m.from_addr)).weak());
                });
                ui.label(RichText::new(format!("{}  ·  {}", widgets::full_time(m.date), m.account)).small().weak());
            });
        });
        ui.add(egui::Label::new(RichText::new(format!("收件人: {}", m.to)).small().weak()).truncate());
        if !m.cc.is_empty() {
            ui.add(egui::Label::new(RichText::new(format!("抄送: {}", m.cc)).small().weak()).truncate());
        }
        ui.add_space(6.0);

        match (&self.body, &self.body_error) {
            (Some((id, body)), _) if *id == m.id => {
                let body = body.clone();
                if !body.attachments.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        for (i, a) in body.attachments.iter().enumerate() {
                            if ui.button(format!("📎 {} ({})", a.name, widgets::human_size(a.size))).on_hover_text("保存到“下载”").clicked() {
                                let eng = self.eng.clone();
                                let dir = dirs::download_dir().unwrap_or_else(std::env::temp_dir);
                                let id = m.id;
                                self.spawn(ctx, async move {
                                    Ev::Done(
                                        eng.save_attachments(id, Some(i), &dir)
                                            .await
                                            .map(|p| format!("已保存 {}", p.first().map(|p| p.display().to_string()).unwrap_or_default()))
                                            .map_err(|e| format!("{e:#}")),
                                    )
                                });
                            }
                        }
                    });
                }
                ui.separator();
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    ui.add(egui::Label::new(RichText::new(body.text.trim_end()).size(14.5)).wrap().selectable(true));
                    ui.add_space(30.0);
                });
            }
            (_, Some(err)) => {
                ui.colored_label(Color32::from_rgb(220, 80, 60), err);
            }
            _ => {
                ui.add_space(20.0);
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("正在加载…");
                });
            }
        }
    }

    fn welcome(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.vertical_centered(|ui| {
            ui.add_space(ui.available_height() * 0.22);
            ui.label(RichText::new("✉").size(56.0));
            ui.label(RichText::new("Apark").size(34.0).strong());
            ui.label(RichText::new("一个账号登录，所有邮箱回来").weak());
            ui.add_space(24.0);
            let google_ready = self.eng.config().google_client().is_some();
            if let Some(msg) = &self.busy {
                ui.spinner();
                ui.label(msg);
                if let Some(url) = &self.login_url {
                    if ui.link("浏览器没打开？点这里").clicked() {
                        let _ = webbrowser::open(url);
                    }
                }
            } else {
                let btn = egui::Button::new(RichText::new("  使用 Google 账号登录  ").size(16.0)).min_size(egui::vec2(260.0, 40.0));
                if ui.add_enabled(google_ready, btn).clicked() {
                    self.busy = Some("请在浏览器中完成 Google 授权…".into());
                    let eng = self.eng.clone();
                    let opts = self.login_opts(ctx);
                    self.spawn(ctx, async move {
                        Ev::Accounts(
                            eng.login_master(&opts)
                                .await
                                .map(|(a, r)| format!("已登录 {}，恢复 {} 个账号", a.email, r.added.len()))
                                .map_err(|e| format!("{e:#}")),
                        )
                    });
                }
                if !google_ready {
                    ui.label(RichText::new("需要先在设置里填写 Google OAuth 客户端 ID").small().weak());
                }
                ui.add_space(8.0);
                if ui.link("添加其他邮箱（IMAP / Outlook）").clicked() {
                    self.add = Some(AddDialog::default());
                }
                if ui.link("设置").clicked() {
                    self.settings = Some(self.eng.config());
                }
            }
            if let Some(e) = &self.error {
                ui.add_space(12.0);
                ui.colored_label(Color32::from_rgb(220, 80, 60), e);
            }
        });
    }

    // ---- windows --------------------------------------------------------

    fn composer_windows(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        if let Some(c) = self.composers.last_mut() {
            c.attachments.extend(dropped);
        }
        let accounts: Vec<String> = self.accounts.iter().map(|a| a.email.clone()).collect();
        let mut close = vec![];
        let mut send = vec![];
        for c in &mut self.composers {
            let mut open = true;
            let title = if c.subject.is_empty() { "新邮件".to_owned() } else { c.subject.clone() };
            egui::Window::new(title)
                .id(egui::Id::new(("composer", c.id)))
                .open(&mut open)
                .default_size([600.0, 520.0])
                .collapsible(true)
                .show(ctx, |ui| {
                    egui::Grid::new(("grid", c.id)).num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                        ui.label("发件人");
                        egui::ComboBox::from_id_salt(("from", c.id)).selected_text(&c.from).width(ui.available_width()).show_ui(ui, |ui| {
                            for a in &accounts {
                                ui.selectable_value(&mut c.from, a.clone(), a);
                            }
                        });
                        ui.end_row();
                        ui.label("收件人");
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut c.to).desired_width(ui.available_width() - 60.0));
                            if !c.show_cc && ui.small_button("抄送").clicked() {
                                c.show_cc = true;
                            }
                        });
                        ui.end_row();
                        if c.show_cc {
                            ui.label("抄送");
                            ui.add(egui::TextEdit::singleline(&mut c.cc).desired_width(f32::INFINITY));
                            ui.end_row();
                            ui.label("密送");
                            ui.add(egui::TextEdit::singleline(&mut c.bcc).desired_width(f32::INFINITY));
                            ui.end_row();
                        }
                        ui.label("主题");
                        ui.add(egui::TextEdit::singleline(&mut c.subject).desired_width(f32::INFINITY));
                        ui.end_row();
                    });
                    ui.add_space(4.0);
                    let mut remove = None;
                    if !c.attachments.is_empty() {
                        ui.horizontal_wrapped(|ui| {
                            for (i, p) in c.attachments.iter().enumerate() {
                                let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                                if ui.small_button(format!("📎 {name} ✕")).clicked() {
                                    remove = Some(i);
                                }
                            }
                        });
                    }
                    if let Some(i) = remove {
                        c.attachments.remove(i);
                    }
                    egui::ScrollArea::vertical().max_height(ui.available_height() - 44.0).show(ui, |ui| {
                        ui.add(egui::TextEdit::multiline(&mut c.body).desired_width(f32::INFINITY).desired_rows(14));
                    });
                    if let Some(e) = &c.error {
                        ui.colored_label(Color32::from_rgb(220, 80, 60), e);
                    }
                    ui.horizontal(|ui| {
                        let can = !c.sending && !c.to.trim().is_empty() && !c.from.is_empty();
                        let send_clicked = ui.add_enabled(can, egui::Button::new(RichText::new("发送").strong())).clicked()
                            || (can && ui.input(|i| (i.modifiers.command || i.modifiers.ctrl) && i.key_pressed(Key::Enter)));
                        if send_clicked {
                            c.sending = true;
                            c.error = None;
                            send.push(c.id);
                        }
                        if c.sending {
                            ui.spinner();
                        }
                        ui.label(RichText::new("拖入文件即可添加附件 · ⌘/Ctrl+Enter 发送").small().weak());
                    });
                });
            if !open {
                close.push(c.id);
            }
        }
        self.composers.retain(|c| !close.contains(&c.id));
        for cid in send {
            let Some(c) = self.composers.iter().find(|c| c.id == cid) else { continue };
            let split = |s: &str| s.split([',', ';']).map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).collect::<Vec<_>>();
            let out = Outgoing {
                to: split(&c.to),
                cc: split(&c.cc),
                bcc: split(&c.bcc),
                subject: c.subject.clone(),
                body: c.body.clone(),
                html: None,
                in_reply_to: c.in_reply_to.clone(),
                references: c.references.clone(),
                attachments: c.attachments.clone(),
            };
            let (eng, from) = (self.eng.clone(), c.from.clone());
            self.spawn(ctx, async move { Ev::Sent(cid, eng.send(&from, &out).await.map_err(|e| format!("{e:#}"))) });
        }
    }

    fn add_window(&mut self, ctx: &egui::Context) {
        let Some(mut d) = self.add.take() else { return };
        let mut open = true;
        let cfg = self.eng.config();
        let mut submit: Option<&'static str> = None;
        egui::Window::new("添加账号").open(&mut open).collapsible(false).resizable(false).default_width(380.0).show(ctx, |ui| {
            if let Some(msg) = &self.busy {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(msg);
                });
                if let Some(url) = &self.login_url {
                    if ui.link("浏览器没打开？点这里").clicked() {
                        let _ = webbrowser::open(url);
                    }
                }
                return;
            }
            ui.horizontal(|ui| {
                if ui.add_enabled(cfg.google_client().is_some(), egui::Button::new("Google / Gmail")).clicked() {
                    submit = Some("google");
                }
                if ui.add_enabled(cfg.microsoft_client().is_some(), egui::Button::new("Outlook / Microsoft")).clicked() {
                    submit = Some("microsoft");
                }
            });
            if cfg.google_client().is_none() || cfg.microsoft_client().is_none() {
                ui.label(RichText::new("OAuth 登录需要在设置里配置客户端 ID").small().weak());
            }
            ui.separator();
            ui.label(RichText::new("其他邮箱（IMAP，推荐使用应用专用密码）").strong());
            egui::Grid::new("imap").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                ui.label("邮箱");
                ui.add(egui::TextEdit::singleline(&mut d.email).hint_text("you@example.com"));
                ui.end_row();
                ui.label("密码");
                ui.add(egui::TextEdit::singleline(&mut d.password).password(true));
                ui.end_row();
                ui.label("名字");
                ui.add(egui::TextEdit::singleline(&mut d.name).hint_text("可选"));
                ui.end_row();
            });
            ui.checkbox(&mut d.advanced, "手动填写服务器");
            if d.advanced {
                egui::Grid::new("servers").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                    ui.label("IMAP");
                    ui.add(egui::TextEdit::singleline(&mut d.imap).hint_text("imap.example.com:993"));
                    ui.end_row();
                    ui.label("SMTP");
                    ui.add(egui::TextEdit::singleline(&mut d.smtp).hint_text("smtp.example.com:465"));
                    ui.end_row();
                });
            }
            if ui.add_enabled(d.email.contains('@') && !d.password.is_empty(), egui::Button::new("添加")).clicked() {
                submit = Some("imap");
            }
            if let Some(e) = &self.error {
                ui.colored_label(Color32::from_rgb(220, 80, 60), e);
            }
        });
        match submit {
            Some(kind @ ("google" | "microsoft")) => {
                let provider = if kind == "google" { Provider::Google } else { Provider::Microsoft };
                self.busy = Some("请在浏览器中完成授权…".into());
                self.error = None;
                let eng = self.eng.clone();
                let opts = self.login_opts(ctx);
                self.spawn(ctx, async move {
                    Ev::Accounts(eng.add_oauth(provider, &opts).await.map(|a| format!("已添加 {}", a.email)).map_err(|e| format!("{e:#}")))
                });
            }
            Some(_) => {
                self.busy = Some("正在验证登录…".into());
                self.error = None;
                let eng = self.eng.clone();
                let (email, password, name) = (d.email.trim().to_owned(), d.password.clone(), d.name.clone());
                let imap = (d.advanced && !d.imap.trim().is_empty()).then(|| Server::parse(&d.imap, 993));
                let smtp = (d.advanced && !d.smtp.trim().is_empty()).then(|| Server::parse(&d.smtp, 465));
                self.spawn(ctx, async move {
                    let r = async {
                        let imap = imap.transpose()?;
                        let smtp = smtp.transpose()?;
                        eng.add_password(&email, &password, &name, imap, smtp).await
                    }
                    .await;
                    Ev::Accounts(r.map(|a| format!("已添加 {}", a.email)).map_err(|e| format!("{e:#}")))
                });
            }
            None => {}
        }
        if open {
            self.add = Some(d);
        } else {
            self.busy = None;
            self.login_url = None;
        }
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        let Some(mut cfg) = self.settings.take() else { return };
        let mut open = true;
        let mut save = false;
        let mut cloud = false;
        egui::Window::new("设置").open(&mut open).collapsible(false).default_width(460.0).show(ctx, |ui| {
            let opt = |ui: &mut egui::Ui, label: &str, v: &mut Option<String>, password: bool| {
                ui.label(label);
                let mut s = v.clone().unwrap_or_default();
                ui.add(egui::TextEdit::singleline(&mut s).password(password).desired_width(300.0));
                *v = Some(s).filter(|s| !s.trim().is_empty());
                ui.end_row();
            };
            egui::Grid::new("settings").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
                opt(ui, "Google 客户端 ID", &mut cfg.google_client_id, false);
                opt(ui, "Google 客户端密钥", &mut cfg.google_client_secret, true);
                opt(ui, "Microsoft 客户端 ID", &mut cfg.microsoft_client_id, false);
                opt(ui, "云同步密码（可选）", &mut cfg.sync_passphrase, true);
                ui.label("同步间隔（秒）");
                ui.add(egui::DragValue::new(&mut cfg.sync_interval_secs).range(30..=3600));
                ui.end_row();
                ui.label("首次同步封数");
                ui.add(egui::DragValue::new(&mut cfg.initial_limit).range(50..=20000));
                ui.end_row();
            });
            ui.label(RichText::new(format!("数据目录: {}", apark_core::paths::home().display())).small().weak());
            ui.label(RichText::new("账号列表会加密保存在总账号的 Google Drive 隐藏目录；设置同步密码后需在每台设备填写相同密码。").small().weak());
            ui.horizontal(|ui| {
                if ui.button("保存").clicked() {
                    save = true;
                }
                if self.eng.has_master() && ui.button("立即同步账号列表").clicked() {
                    cloud = true;
                }
            });
        });
        if save {
            match self.eng.set_config(cfg.clone()) {
                Ok(()) => {
                    self.status = "设置已保存".into();
                    open = false;
                }
                Err(e) => self.error = Some(format!("{e:#}")),
            }
        }
        if cloud {
            let eng = self.eng.clone();
            self.spawn(ctx, async move {
                Ev::Accounts(eng.cloud_sync().await.map(|r| format!("账号同步: {}", r.action)).map_err(|e| format!("{e:#}")))
            });
        }
        if open {
            self.settings = Some(cfg);
        }
    }
}

fn folder_label(f: &Folder) -> String {
    let icon = match f.role.as_str() {
        "inbox" => "📥",
        "sent" => "📤",
        "drafts" => "📝",
        "trash" => "🗑",
        "junk" => "⚠",
        "archive" | "all" => "🗄",
        "flagged" => "★",
        _ => "📁",
    };
    let name = if f.name == "INBOX" { "收件箱" } else { f.name.rsplit(['/', '.']).next().unwrap_or(&f.name) };
    format!("{icon} {name}")
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.pump(ctx);
        if self.reload_at.is_some_and(|t| Instant::now() >= t) {
            self.reload_at = None;
            self.reload();
        }
        let interval = Duration::from_secs(self.eng.config().sync_interval_secs.max(30));
        if !self.accounts.is_empty() && self.last_sync.is_none_or(|t| t.elapsed() >= interval) {
            self.start_sync(ctx);
        }
        ctx.request_repaint_after(Duration::from_secs(5));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.shortcuts(&ctx);

        if let Some(err) = self.error.clone() {
            if !self.accounts.is_empty() {
                egui::Panel::bottom("error").show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.colored_label(Color32::from_rgb(220, 80, 60), format!("⚠ {err}"));
                        if ui.small_button("✕").clicked() {
                            self.error = None;
                        }
                    });
                });
            }
        }

        if self.accounts.is_empty() {
            egui::CentralPanel::default().show(ui, |ui| self.welcome(ui, &ctx));
        } else {
            egui::Panel::left("nav").resizable(true).default_size(230.0).size_range(180.0..=360.0).show(ui, |ui| {
                self.sidebar(ui, &ctx)
            });
            egui::Panel::left("list").resizable(true).default_size(400.0).size_range(280.0..=640.0).show(ui, |ui| {
                self.list_panel(ui, &ctx)
            });
            egui::CentralPanel::default().show(ui, |ui| self.reader(ui, &ctx));
        }
        self.composer_windows(&ctx);
        self.add_window(&ctx);
        self.settings_window(&ctx);
    }
}

mod futures_lite {
    pub type BoxedOp = std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send>>;
}
