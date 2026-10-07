//! GNOME-native UI: libadwaita split views, list views and dialogs.
//! Data comes from the local store synchronously; network work runs on tokio
//! and comes back to the GTK main loop through oneshot channels.

use std::cell::RefCell;
use std::future::Future;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use adw::prelude::*;
use apark_core::{Account, Body, Config, Engine, ListQuery, LoginOpts, MsgRow, Outgoing, Provider, Server};
use gtk::{gdk, gio, glib};

const APP_ID: &str = "io.github.cd1s.Apark";
const ICON_PNG: &[u8] = include_bytes!("../../../assets/icon-256.png");

#[derive(Clone, PartialEq)]
enum Nav {
    Inbox,
    Category(&'static str),
    Unread,
    Flagged,
    Folder(String, String),
}

#[derive(Default)]
struct State {
    nav: Option<Nav>,
    rows: Vec<MsgRow>,
    selected: Option<i64>,
    body: Option<(i64, Body)>,
    accounts: Vec<Account>,
    syncing: bool,
    nav_items: Vec<Option<Nav>>,
}

struct Ui {
    eng: Arc<Engine>,
    rt: tokio::runtime::Runtime,
    window: adw::ApplicationWindow,
    toasts: adw::ToastOverlay,
    stack: gtk::Stack,
    welcome: adw::StatusPage,
    welcome_spinner: gtk::Spinner,
    login_link: gtk::LinkButton,
    sidebar: gtk::ListBox,
    sync_label: gtk::Label,
    list_title: adw::WindowTitle,
    store: gio::ListStore,
    selection: gtk::SingleSelection,
    list_stack: gtk::Stack,
    list_empty: adw::StatusPage,
    search: gtk::SearchEntry,
    sync_button: gtk::Button,
    reader_stack: gtk::Stack,
    subject: gtk::Label,
    avatar: adw::Avatar,
    sender: gtk::Label,
    address: gtk::Label,
    date: gtk::Label,
    recipients: gtk::Label,
    attachments: gtk::Box,
    body: gtk::TextView,
    html_button: gtk::Button,
    star_button: gtk::Button,
    seen_button: gtk::Button,
    state: RefCell<State>,
}

pub fn run() -> anyhow::Result<()> {
    let eng = Engine::open()?;
    let app = adw::Application::builder().application_id(APP_ID).build();
    app.connect_activate(move |app| build(app, eng.clone()));
    app.run_with_args::<&str>(&[]);
    Ok(())
}

fn icon_button(icon: &str, tip: &str) -> gtk::Button {
    let b = gtk::Button::from_icon_name(icon);
    b.set_tooltip_text(Some(tip));
    b
}

fn short_time(ts: i64) -> String {
    use chrono::{Datelike, Local, TimeZone};
    let Some(t) = Local.timestamp_opt(ts, 0).single() else { return String::new() };
    let now = Local::now();
    if t.date_naive() == now.date_naive() {
        t.format("%H:%M").to_string()
    } else if (now.date_naive() - t.date_naive()).num_days() == 1 {
        "昨天".into()
    } else if t.year() == now.year() {
        format!("{}月{}日", t.month(), t.day())
    } else {
        t.format("%Y/%m/%d").to_string()
    }
}

fn long_time(ts: i64) -> String {
    use chrono::{Local, TimeZone};
    Local.timestamp_opt(ts, 0).single().map(|t| t.format("%Y年%m月%d日 %H:%M").to_string()).unwrap_or_default()
}

fn human_size(n: usize) -> String {
    glib::format_size(n as u64).to_string()
}

fn account_hue(email: &str) -> f64 {
    let h = email.bytes().fold(0x811c9dc5u32, |h, b| (h ^ b as u32).wrapping_mul(0x0100_0193));
    (h % 3600) as f64 / 10.0
}

const CSS: &str = "
.unread-dot { background: @accent_bg_color; border-radius: 99px; min-width: 8px; min-height: 8px; }
.read-dot { min-width: 8px; min-height: 8px; }
.msg-row { padding: 8px 6px; }
.msg-sender { font-weight: 500; }
.msg-row.unread .msg-sender { font-weight: 800; }
.msg-row.unread .msg-subject { font-weight: 700; }
.msg-snippet { font-size: 0.92em; }
.msg-date { font-size: 0.85em; font-feature-settings: 'tnum'; }
.account-dot { border-radius: 99px; min-width: 6px; min-height: 6px; }
.reader-subject { font-size: 1.45em; font-weight: 700; }
.reader-body { font-size: 1.05em; background: transparent; }
.star { color: #f5a623; }
";

fn build(app: &adw::Application, eng: Arc<Engine>) {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(CSS);
    gtk::style_context_add_provider_for_display(
        &gdk::Display::default().expect("display"),
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    let window = adw::ApplicationWindow::builder().application(app).title("Apark").default_width(1220).default_height(780).build();
    let toasts = adw::ToastOverlay::new();
    let stack = gtk::Stack::builder().transition_type(gtk::StackTransitionType::Crossfade).build();
    toasts.set_child(Some(&stack));
    window.set_content(Some(&toasts));

    // ---- welcome --------------------------------------------------------
    let welcome = adw::StatusPage::builder()
        .title("欢迎使用 Apark")
        .description("用一个 Google 账号登录，所有邮箱都会回来。")
        .build();
    welcome.set_paintable(Some(&gdk::Texture::from_bytes(&glib::Bytes::from_static(ICON_PNG)).expect("icon")));
    let google = gtk::Button::builder().label("使用 Google 账号登录").halign(gtk::Align::Center).build();
    google.add_css_class("pill");
    google.add_css_class("suggested-action");
    let other = gtk::Button::builder().label("添加其他邮箱…").halign(gtk::Align::Center).build();
    other.add_css_class("flat");
    let settings_btn = gtk::Button::builder().label("设置").halign(gtk::Align::Center).build();
    settings_btn.add_css_class("flat");
    let welcome_spinner = gtk::Spinner::new();
    let login_link = gtk::LinkButton::with_label("https://accounts.google.com", "浏览器没有打开？点这里");
    login_link.set_visible(false);
    let welcome_box = gtk::Box::new(gtk::Orientation::Vertical, 10);
    for w in [google.upcast_ref::<gtk::Widget>(), other.upcast_ref(), settings_btn.upcast_ref(), welcome_spinner.upcast_ref(), login_link.upcast_ref()] {
        welcome_box.append(w);
    }
    welcome.set_child(Some(&welcome_box));
    let welcome_view = adw::ToolbarView::new();
    welcome_view.add_top_bar(&adw::HeaderBar::new());
    welcome_view.set_content(Some(&welcome));
    stack.add_named(&welcome_view, Some("welcome"));

    // ---- sidebar --------------------------------------------------------
    let sidebar = gtk::ListBox::new();
    sidebar.add_css_class("navigation-sidebar");
    let sync_label = gtk::Label::builder().xalign(0.0).margin_start(14).margin_bottom(10).margin_top(6).build();
    sync_label.add_css_class("dim-label");
    sync_label.add_css_class("caption");
    let sidebar_scroll = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).child(&sidebar).build();
    let sidebar_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
    sidebar_box.append(&sidebar_scroll);
    sidebar_box.append(&sync_label);
    let sidebar_header = adw::HeaderBar::new();
    let add_btn = icon_button("list-add-symbolic", "添加账号");
    let menu_btn = icon_button("open-menu-symbolic", "设置");
    sidebar_header.pack_start(&add_btn);
    sidebar_header.pack_end(&menu_btn);
    sidebar_header.set_title_widget(Some(&adw::WindowTitle::new("Apark", "")));
    let sidebar_view = adw::ToolbarView::new();
    sidebar_view.add_top_bar(&sidebar_header);
    sidebar_view.set_content(Some(&sidebar_box));

    // ---- message list ---------------------------------------------------
    let store = gio::ListStore::new::<glib::BoxedAnyObject>();
    let selection = gtk::SingleSelection::new(Some(store.clone()));
    selection.set_autoselect(false);
    selection.set_can_unselect(true);
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_bind(|_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().expect("list item");
        if let Some(obj) = item.item().and_downcast::<glib::BoxedAnyObject>() {
            item.set_child(Some(&message_row(&obj.borrow::<MsgRow>())));
        }
    });
    let list = gtk::ListView::new(Some(selection.clone()), Some(factory));
    list.add_css_class("navigation-sidebar");
    let list_scroll = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).child(&list).build();
    let list_empty = adw::StatusPage::builder().icon_name("mail-read-symbolic").title("没有邮件").build();
    list_empty.add_css_class("compact");
    let list_stack = gtk::Stack::new();
    list_stack.add_named(&list_scroll, Some("list"));
    list_stack.add_named(&list_empty, Some("empty"));
    let search = gtk::SearchEntry::builder().placeholder_text("搜索邮件").margin_start(10).margin_end(10).margin_bottom(6).build();
    let list_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
    list_box.append(&search);
    list_box.append(&list_stack);
    let list_header = adw::HeaderBar::new();
    let list_title = adw::WindowTitle::new("收件箱", "");
    list_header.set_title_widget(Some(&list_title));
    let sync_button = icon_button("view-refresh-symbolic", "收取新邮件 (F5)");
    let compose_btn = icon_button("document-edit-symbolic", "写邮件 (Ctrl+N)");
    list_header.pack_start(&sync_button);
    list_header.pack_end(&compose_btn);
    let list_view = adw::ToolbarView::new();
    list_view.add_top_bar(&list_header);
    list_view.set_content(Some(&list_box));

    // ---- reader ---------------------------------------------------------
    let subject = gtk::Label::builder().xalign(0.0).wrap(true).selectable(true).build();
    subject.add_css_class("reader-subject");
    let avatar = adw::Avatar::new(42, None, true);
    let sender = gtk::Label::builder().xalign(0.0).build();
    sender.add_css_class("heading");
    let address = gtk::Label::builder().xalign(0.0).selectable(true).build();
    address.add_css_class("dim-label");
    let date = gtk::Label::builder().xalign(1.0).valign(gtk::Align::Start).hexpand(true).build();
    date.add_css_class("dim-label");
    date.add_css_class("caption");
    let recipients = gtk::Label::builder().xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).build();
    recipients.add_css_class("dim-label");
    recipients.add_css_class("caption");
    let who = gtk::Box::new(gtk::Orientation::Vertical, 2);
    let name_line = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    name_line.append(&sender);
    name_line.append(&address);
    who.append(&name_line);
    who.append(&recipients);
    let from_line = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    from_line.append(&avatar);
    from_line.append(&who);
    from_line.append(&date);
    let attachments = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let body = gtk::TextView::builder()
        .editable(false)
        .cursor_visible(false)
        .wrap_mode(gtk::WrapMode::WordChar)
        .top_margin(8)
        .bottom_margin(40)
        .build();
    body.add_css_class("reader-body");
    let reader_box = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(14).margin_start(28).margin_end(28).margin_top(20).build();
    reader_box.append(&subject);
    reader_box.append(&from_line);
    reader_box.append(&attachments);
    reader_box.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    reader_box.append(&body);
    let reader_scroll = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).child(&reader_box).build();
    let reader_empty = adw::StatusPage::builder().icon_name("mail-unread-symbolic").title("没有选中邮件").build();
    let reader_stack = gtk::Stack::builder().transition_type(gtk::StackTransitionType::Crossfade).build();
    reader_stack.add_named(&reader_empty, Some("empty"));
    reader_stack.add_named(&reader_scroll, Some("message"));
    let reader_header = adw::HeaderBar::new();
    reader_header.set_show_title(false);
    let reply_btn = icon_button("mail-reply-sender-symbolic", "回复 (Ctrl+R)");
    let reply_all_btn = icon_button("mail-reply-all-symbolic", "全部回复 (Ctrl+Shift+R)");
    let forward_btn = icon_button("mail-forward-symbolic", "转发 (Ctrl+Shift+F)");
    let archive_btn = icon_button("folder-download-symbolic", "归档 (Ctrl+E)");
    let trash_btn = icon_button("user-trash-symbolic", "删除 (Ctrl+Delete)");
    let star_button = icon_button("non-starred-symbolic", "星标");
    let seen_button = icon_button("mail-unread-symbolic", "标为未读");
    let html_button = icon_button("send-to-symbolic", "在浏览器中查看原始排版");
    for b in [&reply_btn, &reply_all_btn, &forward_btn] {
        reader_header.pack_start(b);
    }
    for b in [&html_button, &star_button, &seen_button, &trash_btn, &archive_btn] {
        reader_header.pack_end(b);
    }
    let reader_view = adw::ToolbarView::new();
    reader_view.add_top_bar(&reader_header);
    reader_view.set_content(Some(&reader_stack));

    // ---- splits ---------------------------------------------------------
    let inner = adw::NavigationSplitView::new();
    inner.set_sidebar(Some(&adw::NavigationPage::new(&list_view, "邮件")));
    inner.set_content(Some(&adw::NavigationPage::new(&reader_view, "阅读")));
    inner.set_min_sidebar_width(320.0);
    inner.set_max_sidebar_width(520.0);
    inner.set_sidebar_width_fraction(0.42);
    let outer = adw::NavigationSplitView::new();
    outer.set_sidebar(Some(&adw::NavigationPage::new(&sidebar_view, "Apark")));
    outer.set_content(Some(&adw::NavigationPage::new(&inner, "收件箱")));
    outer.set_min_sidebar_width(200.0);
    outer.set_max_sidebar_width(300.0);
    outer.set_sidebar_width_fraction(0.2);
    stack.add_named(&outer, Some("main"));

    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().expect("tokio");
    let ui = Rc::new(Ui {
        eng,
        rt,
        window: window.clone(),
        toasts,
        stack,
        welcome,
        welcome_spinner,
        login_link,
        sidebar,
        sync_label,
        list_title,
        store,
        selection: selection.clone(),
        list_stack,
        list_empty,
        search: search.clone(),
        sync_button: sync_button.clone(),
        reader_stack,
        subject,
        avatar,
        sender,
        address,
        date,
        recipients,
        attachments,
        body,
        html_button: html_button.clone(),
        star_button: star_button.clone(),
        seen_button: seen_button.clone(),
        state: RefCell::new(State { nav: Some(Nav::Inbox), ..Default::default() }),
    });

    // ---- signals --------------------------------------------------------
    {
        let u = ui.clone();
        ui.sidebar.connect_row_selected(move |_, row| {
            let Some(row) = row else { return };
            let nav = u.state.borrow().nav_items.get(row.index() as usize).cloned().flatten();
            if let Some(nav) = nav {
                if u.state.borrow().nav.as_ref() != Some(&nav) {
                    u.set_nav(nav);
                }
            }
        });
    }
    {
        let u = ui.clone();
        selection.connect_selected_notify(move |sel| {
            let id = sel.selected_item().and_downcast::<glib::BoxedAnyObject>().map(|o| o.borrow::<MsgRow>().id);
            u.show_message(id);
        });
    }
    {
        let u = ui.clone();
        search.connect_search_changed(move |_| u.reload());
    }
    let actions: [(&gtk::Button, fn(&Rc<Ui>)); 13] = [
        (&sync_button, |u| u.sync()),
        (&compose_btn, |u| u.compose(None, Outgoing::default())),
        (&reply_btn, |u| u.reply(false)),
        (&reply_all_btn, |u| u.reply(true)),
        (&forward_btn, |u| u.forward()),
        (&archive_btn, |u| u.take_out(true)),
        (&trash_btn, |u| u.take_out(false)),
        (&star_button, |u| u.toggle(true)),
        (&seen_button, |u| u.toggle(false)),
        (&html_button, |u| u.open_html()),
        (&google, |u| u.login_master()),
        (&other, |u| u.add_account_dialog()),
        (&add_btn, |u| u.add_account_dialog()),
    ];
    for (button, f) in actions {
        let u = ui.clone();
        button.connect_clicked(move |_| f(&u));
    }
    for b in [&menu_btn, &settings_btn] {
        let u = ui.clone();
        b.connect_clicked(move |_| u.settings_dialog());
    }

    // Keyboard shortcuts.
    let shortcuts: [(&str, &[&str], fn(&Rc<Ui>)); 9] = [
        ("compose", &["<Ctrl>n"], |u| u.compose(None, Outgoing::default())),
        ("reply", &["<Ctrl>r"], |u| u.reply(false)),
        ("reply-all", &["<Ctrl><Shift>r"], |u| u.reply(true)),
        ("forward", &["<Ctrl><Shift>f"], |u| u.forward()),
        ("archive", &["<Ctrl>e"], |u| u.take_out(true)),
        ("trash", &["<Ctrl>Delete"], |u| u.take_out(false)),
        ("toggle-seen", &["<Ctrl><Shift>u"], |u| u.toggle(false)),
        ("sync", &["F5"], |u| u.sync()),
        ("search", &["<Ctrl>f"], |u| {
            u.search.grab_focus();
        }),
    ];
    for (name, accels, f) in shortcuts {
        let action = gio::SimpleAction::new(name, None);
        let u = ui.clone();
        action.connect_activate(move |_, _| f(&u));
        window.add_action(&action);
        app.set_accels_for_action(&format!("win.{name}"), accels);
    }

    ui.refresh_accounts();
    ui.reload();
    // Background sync on the configured interval.
    ui.sync();
    {
        let u = ui.clone();
        let secs = ui.eng.config().sync_interval_secs.max(30) as u32;
        glib::timeout_add_seconds_local(secs, move || {
            u.sync();
            glib::ControlFlow::Continue
        });
    }
    if let Ok(path) = std::env::var("APARK_SNAPSHOT") {
        snapshot(ui.clone(), path);
    }
    window.present();
}

fn message_row(m: &MsgRow) -> gtk::Widget {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.add_css_class("msg-row");
    if !m.seen {
        row.add_css_class("unread");
    }
    let dot = gtk::Box::builder().valign(gtk::Align::Start).margin_top(6).build();
    dot.add_css_class(if m.seen { "read-dot" } else { "unread-dot" });
    row.append(&dot);
    let col = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).hexpand(true).build();
    let top = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let sender = gtk::Label::builder().label(m.sender()).xalign(0.0).hexpand(true).ellipsize(gtk::pango::EllipsizeMode::End).build();
    sender.add_css_class("msg-sender");
    top.append(&sender);
    if m.flagged {
        let star = gtk::Image::from_icon_name("starred-symbolic");
        star.add_css_class("star");
        star.set_pixel_size(12);
        top.append(&star);
    }
    let date = gtk::Label::new(Some(&short_time(m.date)));
    date.add_css_class("dim-label");
    date.add_css_class("msg-date");
    top.append(&date);
    col.append(&top);
    let subject = gtk::Label::builder()
        .label(if m.subject.is_empty() { "（无主题）" } else { m.subject.as_str() })
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    subject.add_css_class("msg-subject");
    col.append(&subject);
    if !m.snippet.is_empty() {
        let snippet = gtk::Label::builder()
            .label(m.snippet.as_str())
            .xalign(0.0)
            .wrap(true)
            .lines(2)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        snippet.add_css_class("dim-label");
        snippet.add_css_class("msg-snippet");
        col.append(&snippet);
    }
    let acct = gtk::Box::new(gtk::Orientation::Horizontal, 5);
    let adot = gtk::Box::builder().valign(gtk::Align::Center).build();
    adot.add_css_class("account-dot");
    adot.inline_css(&format!("background: hsl({:.0}, 55%, 60%);", account_hue(&m.account)));
    acct.append(&adot);
    let alabel = gtk::Label::builder().label(m.account.as_str()).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).build();
    alabel.add_css_class("dim-label");
    alabel.add_css_class("caption");
    acct.append(&alabel);
    col.append(&acct);
    row.append(&col);
    row.upcast()
}

trait InlineCss {
    fn inline_css(&self, css: &str);
}

impl<W: IsA<gtk::Widget>> InlineCss for W {
    /// Per-widget style via a unique class (GTK has no inline style attribute).
    fn inline_css(&self, css: &str) {
        let class = format!("c{:x}", css.bytes().fold(0u64, |h, b| h.wrapping_mul(31).wrapping_add(b as u64)));
        let provider = gtk::CssProvider::new();
        provider.load_from_string(&format!(".{class} {{ {css} }}"));
        gtk::style_context_add_provider_for_display(&self.display(), &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1);
        self.add_css_class(&class);
    }
}

impl Ui {
    fn spawn<T, F, D>(self: &Rc<Self>, fut: F, done: D)
    where
        T: Send + 'static,
        F: Future<Output = T> + Send + 'static,
        D: FnOnce(&Rc<Ui>, T) + 'static,
    {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.rt.spawn(async move {
            let _ = tx.send(fut.await);
        });
        let u = self.clone();
        glib::spawn_future_local(async move {
            if let Ok(v) = rx.await {
                done(&u, v);
            }
        });
    }

    fn toast(&self, msg: &str) {
        let t = adw::Toast::new(msg);
        t.set_timeout(4);
        self.toasts.add_toast(t);
    }

    fn report<T>(self: &Rc<Self>, r: anyhow::Result<T>) -> Option<T> {
        match r {
            Ok(v) => Some(v),
            Err(e) => {
                self.toast(&format!("{e:#}"));
                None
            }
        }
    }

    // ---- data -----------------------------------------------------------

    fn refresh_accounts(self: &Rc<Self>) {
        let accounts = self.eng.accounts();
        self.stack.set_visible_child_name(if accounts.is_empty() { "welcome" } else { "main" });
        self.state.borrow_mut().accounts = accounts;
        self.rebuild_sidebar();
    }

    fn rebuild_sidebar(self: &Rc<Self>) {
        let counts: std::collections::HashMap<String, usize> = self.eng.store.unread_counts().unwrap_or_default().into_iter().collect();
        let total: usize = counts.values().sum();
        let count = |k: &str| counts.get(k).copied().unwrap_or(0);
        let mut items: Vec<Option<Nav>> = vec![];
        while let Some(row) = self.sidebar.row_at_index(0) {
            self.sidebar.remove(&row);
        }
        let header = |text: &str, items: &mut Vec<Option<Nav>>| {
            let l = gtk::Label::builder().label(text).xalign(0.0).margin_top(12).margin_start(6).build();
            l.add_css_class("dim-label");
            l.add_css_class("caption-heading");
            let row = gtk::ListBoxRow::builder().child(&l).selectable(false).activatable(false).build();
            self.sidebar.append(&row);
            items.push(None);
        };
        let entry = |icon: &str, text: &str, n: usize, nav: Nav, items: &mut Vec<Option<Nav>>| {
            let b = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            b.append(&gtk::Image::from_icon_name(icon));
            b.append(&gtk::Label::builder().label(text).xalign(0.0).hexpand(true).ellipsize(gtk::pango::EllipsizeMode::Middle).build());
            if n > 0 {
                let c = gtk::Label::new(Some(&n.to_string()));
                c.add_css_class("dim-label");
                c.add_css_class("numeric");
                b.append(&c);
            }
            self.sidebar.append(&gtk::ListBoxRow::builder().child(&b).build());
            items.push(Some(nav));
        };
        header("智能收件箱", &mut items);
        entry("mail-read-symbolic", "收件箱", total, Nav::Inbox, &mut items);
        entry("avatar-default-symbolic", "个人", count("people"), Nav::Category("people"), &mut items);
        entry("preferences-system-notifications-symbolic", "通知", count("notification"), Nav::Category("notification"), &mut items);
        entry("bookmark-new-symbolic", "订阅", count("newsletter"), Nav::Category("newsletter"), &mut items);
        header("快速筛选", &mut items);
        entry("mail-unread-symbolic", "未读", 0, Nav::Unread, &mut items);
        entry("starred-symbolic", "星标", 0, Nav::Flagged, &mut items);
        let accounts = self.state.borrow().accounts.clone();
        for a in &accounts {
            header(&a.email, &mut items);
            for f in self.eng.folders(&a.email).unwrap_or_default() {
                let (icon, name) = folder_look(&f.role, &f.name);
                entry(icon, &name, 0, Nav::Folder(a.email.clone(), f.name.clone()), &mut items);
            }
        }
        let current = self.state.borrow().nav.clone();
        self.state.borrow_mut().nav_items = items.clone();
        if let Some(i) = items.iter().position(|n| n.is_some() && *n == current) {
            if let Some(row) = self.sidebar.row_at_index(i as i32) {
                self.sidebar.select_row(Some(&row));
            }
        }
    }

    fn set_nav(self: &Rc<Self>, nav: Nav) {
        if let Nav::Folder(account, folder) = &nav {
            if folder != "INBOX" {
                let (eng, a, f) = (self.eng.clone(), account.clone(), folder.clone());
                self.spawn(async move { eng.sync_account(&a, &[f]).await }, |u, r| {
                    u.report(r);
                    u.reload();
                });
            }
        }
        self.state.borrow_mut().nav = Some(nav);
        self.reload();
    }

    fn query(&self) -> ListQuery {
        let search = Some(self.search.text().trim().to_owned()).filter(|s| !s.is_empty());
        let mut q = ListQuery { search, limit: 5000, ..Default::default() };
        match self.state.borrow().nav.clone().unwrap_or(Nav::Inbox) {
            Nav::Inbox => {}
            Nav::Category(c) => q.category = Some(c.to_owned()),
            Nav::Unread => {
                q.unread = true;
                q.folder = Some("INBOX".into());
            }
            Nav::Flagged => {
                q.flagged = true;
                q.folder = Some("INBOX".into());
            }
            Nav::Folder(a, f) => {
                q.account = Some(a);
                q.folder = Some(f);
            }
        }
        q
    }

    fn title(&self) -> String {
        match self.state.borrow().nav.clone().unwrap_or(Nav::Inbox) {
            Nav::Inbox => "收件箱".into(),
            Nav::Category("people") => "个人".into(),
            Nav::Category("notification") => "通知".into(),
            Nav::Category(_) => "订阅".into(),
            Nav::Unread => "未读".into(),
            Nav::Flagged => "星标".into(),
            Nav::Folder(_, f) => folder_look("", &f).1,
        }
    }

    fn reload(self: &Rc<Self>) {
        let Some(rows) = self.report(self.eng.list(&self.query())) else { return };
        let selected = self.state.borrow().selected;
        let objs: Vec<glib::BoxedAnyObject> = rows.iter().cloned().map(glib::BoxedAnyObject::new).collect();
        let unread = rows.iter().filter(|m| !m.seen).count();
        self.state.borrow_mut().rows = rows;
        self.store.splice(0, self.store.n_items(), &objs);
        self.list_title.set_title(&self.title());
        self.list_title.set_subtitle(&if unread > 0 { format!("{unread} 封未读") } else { String::new() });
        self.list_stack.set_visible_child_name(if objs.is_empty() { "empty" } else { "list" });
        self.list_empty.set_title(if self.search.text().is_empty() { "没有邮件" } else { "没有找到" });
        let pos = selected.and_then(|id| self.state.borrow().rows.iter().position(|m| m.id == id));
        match pos {
            Some(p) => self.selection.set_selected(p as u32),
            None => self.show_message(None),
        }
    }

    fn selected_row(&self) -> Option<MsgRow> {
        let st = self.state.borrow();
        st.selected.and_then(|id| st.rows.iter().find(|m| m.id == id).cloned())
    }

    fn show_message(self: &Rc<Self>, id: Option<i64>) {
        let same = self.state.borrow().selected == id && self.state.borrow().body.as_ref().map(|b| b.0) == id;
        self.state.borrow_mut().selected = id;
        let Some(m) = self.selected_row() else {
            self.reader_stack.set_visible_child_name("empty");
            return;
        };
        self.reader_stack.set_visible_child_name("message");
        self.subject.set_label(if m.subject.is_empty() { "（无主题）" } else { &m.subject });
        self.avatar.set_text(Some(m.sender()));
        self.sender.set_label(m.sender());
        self.address.set_label(&m.from_addr);
        self.date.set_label(&long_time(m.date));
        self.recipients.set_label(&if m.cc.is_empty() { format!("收件人：{}", m.to) } else { format!("收件人：{}  抄送：{}", m.to, m.cc) });
        self.star_button.set_icon_name(if m.flagged { "starred-symbolic" } else { "non-starred-symbolic" });
        self.seen_button.set_icon_name(if m.seen { "mail-unread-symbolic" } else { "mail-read-symbolic" });
        self.seen_button.set_tooltip_text(Some(if m.seen { "标为未读" } else { "标为已读" }));
        if same {
            return;
        }
        match self.eng.store.body(m.id) {
            Ok(Some(b)) => self.show_body(m.id, b),
            _ => {
                self.body.buffer().set_text("正在加载…");
                self.set_attachments(m.id, &[]);
                self.html_button.set_visible(false);
                let (eng, id) = (self.eng.clone(), m.id);
                self.spawn(async move { eng.body(id).await }, move |u, r| {
                    if let Some(b) = u.report(r) {
                        if u.state.borrow().selected == Some(id) {
                            u.show_body(id, b);
                        }
                    }
                });
            }
        }
        if !m.seen {
            self.mark(m.id, |m| m.seen = true);
            let eng = self.eng.clone();
            let id = m.id;
            self.spawn(async move { eng.set_seen(id, true).await }, |u, r| {
                u.report(r);
                u.rebuild_sidebar();
            });
        }
    }

    fn show_body(self: &Rc<Self>, id: i64, b: Body) {
        self.body.buffer().set_text(b.text.trim_end());
        self.set_attachments(id, &b.attachments);
        self.html_button.set_visible(b.html.is_some());
        self.state.borrow_mut().body = Some((id, b));
    }

    fn set_attachments(self: &Rc<Self>, id: i64, atts: &[apark_core::store::Attachment]) {
        while let Some(c) = self.attachments.first_child() {
            self.attachments.remove(&c);
        }
        self.attachments.set_visible(!atts.is_empty());
        for (i, a) in atts.iter().enumerate() {
            let content = adw::ButtonContent::builder()
                .icon_name("mail-attachment-symbolic")
                .label(format!("{}  {}", a.name, human_size(a.size)))
                .build();
            let b = gtk::Button::builder().child(&content).tooltip_text("保存到“下载”").build();
            let u = self.clone();
            b.connect_clicked(move |_| {
                let eng = u.eng.clone();
                let dir = dirs::download_dir().unwrap_or_else(std::env::temp_dir);
                u.spawn(async move { eng.save_attachments(id, Some(i), &dir).await }, |u, r| {
                    if let Some(paths) = u.report(r) {
                        if let Some(p) = paths.first() {
                            u.toast(&format!("已保存到 {}", p.display()));
                        }
                    }
                });
            });
            self.attachments.append(&b);
        }
    }

    /// Change a row in place (optimistic update) and redraw it.
    fn mark(self: &Rc<Self>, id: i64, f: impl FnOnce(&mut MsgRow)) {
        let pos = {
            let mut st = self.state.borrow_mut();
            let Some(pos) = st.rows.iter().position(|m| m.id == id) else { return };
            f(&mut st.rows[pos]);
            pos
        };
        let row = self.state.borrow().rows[pos].clone();
        self.store.splice(pos as u32, 1, &[glib::BoxedAnyObject::new(row)]);
        self.selection.set_selected(pos as u32);
        let unread = self.state.borrow().rows.iter().filter(|m| !m.seen).count();
        self.list_title.set_subtitle(&if unread > 0 { format!("{unread} 封未读") } else { String::new() });
    }

    // ---- actions --------------------------------------------------------

    fn sync(self: &Rc<Self>) {
        if self.state.borrow().syncing || self.state.borrow().accounts.is_empty() || std::env::var_os("APARK_NO_SYNC").is_some() {
            return;
        }
        self.state.borrow_mut().syncing = true;
        self.sync_button.set_sensitive(false);
        self.sync_label.set_label("正在同步…");
        let eng = self.eng.clone();
        self.spawn(async move { eng.sync_all().await }, |u, r| {
            u.state.borrow_mut().syncing = false;
            u.sync_button.set_sensitive(true);
            u.sync_label.set_label(&format!("已同步 {}", chrono::Local::now().format("%H:%M")));
            if let Some(err) = r.unwrap_or_default().into_iter().find_map(|(a, r)| r.err().map(|e| format!("{a}：{e:#}"))) {
                u.toast(&err);
            }
            u.refresh_accounts();
            u.reload();
        });
    }

    fn take_out(self: &Rc<Self>, archive: bool) {
        let Some(m) = self.selected_row() else { return };
        let pos = self.state.borrow().rows.iter().position(|r| r.id == m.id).unwrap_or(0);
        self.state.borrow_mut().rows.remove(pos);
        self.store.remove(pos as u32);
        let n = self.store.n_items();
        if n > 0 {
            self.selection.set_selected(pos.min(n as usize - 1) as u32);
        } else {
            self.show_message(None);
        }
        let eng = self.eng.clone();
        let id = m.id;
        self.spawn(
            async move { if archive { eng.archive(id).await } else { eng.trash(id).await } },
            move |u, r| {
                if u.report(r).is_some() {
                    u.toast(if archive { "已归档" } else { "已移到废纸篓" });
                }
                u.rebuild_sidebar();
            },
        );
    }

    fn toggle(self: &Rc<Self>, star: bool) {
        let Some(m) = self.selected_row() else { return };
        let on = if star { !m.flagged } else { !m.seen };
        self.mark(m.id, |r| if star { r.flagged = on } else { r.seen = on });
        self.show_message(Some(m.id));
        let (eng, id) = (self.eng.clone(), m.id);
        self.spawn(
            async move { if star { eng.set_flagged(id, on).await } else { eng.set_seen(id, on).await } },
            |u, r| {
                u.report(r);
                u.rebuild_sidebar();
            },
        );
    }

    fn open_html(self: &Rc<Self>) {
        let st = self.state.borrow();
        let Some((id, Body { html: Some(html), .. })) = st.body.as_ref() else { return };
        let path = std::env::temp_dir().join(format!("apark-{id}.html"));
        if std::fs::write(&path, format!("<meta charset=\"utf-8\">{html}")).is_ok() {
            gtk::FileLauncher::new(Some(&gio::File::for_path(&path))).launch(Some(&self.window), gio::Cancellable::NONE, |_| {});
        }
    }

    fn reply(self: &Rc<Self>, all: bool) {
        let Some(m) = self.selected_row() else { return };
        let eng = self.eng.clone();
        self.spawn(async move { eng.reply_draft(m.id, all).await }, |u, r| {
            if let Some((from, d)) = u.report(r) {
                u.compose(Some(from), d);
            }
        });
    }

    fn forward(self: &Rc<Self>) {
        let Some(m) = self.selected_row() else { return };
        let eng = self.eng.clone();
        self.spawn(async move { eng.forward_draft(m.id).await }, |u, r| {
            if let Some((from, d)) = u.report(r) {
                u.compose(Some(from), d);
            }
        });
    }

    // ---- compose --------------------------------------------------------

    fn compose(self: &Rc<Self>, from: Option<String>, draft: Outgoing) {
        let accounts: Vec<String> = self.state.borrow().accounts.iter().map(|a| a.email.clone()).collect();
        if accounts.is_empty() {
            return;
        }
        let win = adw::Window::builder().title("新邮件").default_width(680).default_height(600).transient_for(&self.window).build();
        let header = adw::HeaderBar::new();
        let send = gtk::Button::builder().label("发送").build();
        send.add_css_class("suggested-action");
        let attach = icon_button("mail-attachment-symbolic", "添加附件");
        header.pack_end(&send);
        header.pack_end(&attach);

        let fields = gtk::ListBox::new();
        fields.add_css_class("boxed-list");
        fields.set_selection_mode(gtk::SelectionMode::None);
        let from_row = adw::ComboRow::builder().title("发件人").model(&gtk::StringList::new(&accounts.iter().map(String::as_str).collect::<Vec<_>>())).build();
        if let Some(i) = from.and_then(|f| accounts.iter().position(|a| *a == f)) {
            from_row.set_selected(i as u32);
        }
        let to = adw::EntryRow::builder().title("收件人").text(draft.to.join(", ")).build();
        let cc = adw::EntryRow::builder().title("抄送").text(draft.cc.join(", ")).build();
        let subject = adw::EntryRow::builder().title("主题").text(draft.subject.as_str()).build();
        for r in [from_row.upcast_ref::<gtk::Widget>(), to.upcast_ref(), cc.upcast_ref(), subject.upcast_ref()] {
            fields.append(r);
        }
        let body = gtk::TextView::builder().wrap_mode(gtk::WrapMode::WordChar).top_margin(12).bottom_margin(12).left_margin(12).right_margin(12).build();
        body.buffer().set_text(&draft.body);
        let body_frame = gtk::Frame::builder().child(&gtk::ScrolledWindow::builder().child(&body).vexpand(true).build()).build();
        let files = gtk::Label::builder().xalign(0.0).wrap(true).visible(!draft.attachments.is_empty()).build();
        files.add_css_class("dim-label");
        let attachments = Rc::new(RefCell::new(draft.attachments.clone()));
        let show_files = {
            let (files, attachments) = (files.clone(), attachments.clone());
            move || {
                let names: Vec<String> = attachments.borrow().iter().filter_map(|p| p.file_name()).map(|n| format!("📎 {}", n.to_string_lossy())).collect();
                files.set_label(&names.join("   "));
                files.set_visible(!names.is_empty());
            }
        };
        show_files();
        let content = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(12).margin_start(16).margin_end(16).margin_top(12).margin_bottom(16).build();
        content.append(&fields);
        content.append(&body_frame);
        content.append(&files);
        let view = adw::ToolbarView::new();
        view.add_top_bar(&header);
        view.set_content(Some(&content));
        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&view));
        win.set_content(Some(&toasts));

        {
            let (win, attachments) = (win.clone(), attachments.clone());
            attach.connect_clicked(move |_| {
                let attachments = attachments.clone();
                let show_files = show_files.clone();
                gtk::FileDialog::new().open_multiple(Some(&win), gio::Cancellable::NONE, move |r| {
                    if let Ok(list) = r {
                        for i in 0..list.n_items() {
                            if let Some(path) = list.item(i).and_downcast::<gio::File>().and_then(|f| f.path()) {
                                attachments.borrow_mut().push(path);
                            }
                        }
                        show_files();
                    }
                });
            });
        }
        {
            let u = self.clone();
            let win2 = win.clone();
            send.connect_clicked(move |btn| {
                let split = |s: String| s.split([',', ';']).map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).collect::<Vec<_>>();
                let buf = body.buffer();
                let out = Outgoing {
                    to: split(to.text().to_string()),
                    cc: split(cc.text().to_string()),
                    subject: subject.text().to_string(),
                    body: buf.text(&buf.start_iter(), &buf.end_iter(), false).to_string(),
                    attachments: attachments.borrow().clone(),
                    in_reply_to: draft.in_reply_to.clone(),
                    references: draft.references.clone(),
                    ..Default::default()
                };
                if out.to.is_empty() && out.cc.is_empty() {
                    toasts.add_toast(adw::Toast::new("请填写收件人"));
                    return;
                }
                let from = accounts.get(from_row.selected() as usize).cloned().unwrap_or_default();
                btn.set_sensitive(false);
                btn.set_label("发送中…");
                let (eng, btn, win, toasts) = (u.eng.clone(), btn.clone(), win2.clone(), toasts.clone());
                u.spawn(async move { eng.send(&from, &out).await }, move |u, r| match r {
                    Ok(()) => {
                        win.close();
                        u.toast("已发送");
                    }
                    Err(e) => {
                        btn.set_sensitive(true);
                        btn.set_label("发送");
                        toasts.add_toast(adw::Toast::new(&format!("{e:#}")));
                    }
                });
            });
        }
        win.present();
    }

    // ---- accounts & settings ------------------------------------------------

    fn login_opts(self: &Rc<Self>) -> LoginOpts {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let link = self.login_link.clone();
        glib::spawn_future_local(async move {
            while let Some(url) = rx.recv().await {
                link.set_uri(&url);
                link.set_visible(true);
            }
        });
        LoginOpts { manual: false, on_url: Arc::new(move |url| { let _ = tx.send(url.to_owned()); }) }
    }

    fn login_master(self: &Rc<Self>) {
        if self.eng.config().google_client().is_none() {
            self.toast("请先在“设置”里填写 Google OAuth 客户端");
            self.settings_dialog();
            return;
        }
        self.welcome_spinner.set_spinning(true);
        self.welcome.set_description(Some("请在浏览器中完成 Google 授权…"));
        let opts = self.login_opts();
        let eng = self.eng.clone();
        self.spawn(async move { eng.login_master(&opts).await }, |u, r| {
            u.welcome_spinner.set_spinning(false);
            u.login_link.set_visible(false);
            u.welcome.set_description(Some("用一个 Google 账号登录，所有邮箱都会回来。"));
            if let Some((a, res)) = u.report(r) {
                u.toast(&format!("已登录 {}，恢复了 {} 个账号", a.email, res.added.len()));
                u.after_accounts_changed();
            }
        });
    }

    fn after_accounts_changed(self: &Rc<Self>) {
        self.refresh_accounts();
        self.reload();
        self.sync();
    }

    fn add_account_dialog(self: &Rc<Self>) {
        let dialog = adw::Dialog::builder().title("添加账号").content_width(440).build();
        let header = adw::HeaderBar::new();
        let page = adw::PreferencesPage::new();

        let oauth = adw::PreferencesGroup::builder().title("一键登录").build();
        let cfg = self.eng.config();
        for (label, provider, ready) in [
            ("Google / Gmail", Provider::Google, cfg.google_client().is_some()),
            ("Outlook / Microsoft 365", Provider::Microsoft, cfg.microsoft_client().is_some()),
        ] {
            let row = adw::ActionRow::builder().title(label).activatable(ready).build();
            row.set_subtitle(if ready { "在浏览器中授权" } else { "需要先在设置里填写 OAuth 客户端" });
            row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
            let (u, d) = (self.clone(), dialog.clone());
            row.connect_activated(move |row| {
                row.set_subtitle("请在浏览器中完成授权…");
                let opts = u.login_opts();
                let eng = u.eng.clone();
                let d = d.clone();
                u.spawn(async move { eng.add_oauth(provider, &opts).await }, move |u, r| {
                    if let Some(a) = u.report(r) {
                        u.toast(&format!("已添加 {}", a.email));
                        d.close();
                        u.after_accounts_changed();
                    }
                });
            });
            oauth.add(&row);
        }
        page.add(&oauth);

        let imap = adw::PreferencesGroup::builder().title("其他邮箱").description("QQ、163、iCloud 等请使用授权码 / 应用专用密码").build();
        let email = adw::EntryRow::builder().title("邮箱").build();
        let password = adw::PasswordEntryRow::builder().title("密码").build();
        let name = adw::EntryRow::builder().title("名字（可选）").build();
        let imap_host = adw::EntryRow::builder().title("IMAP 服务器（留空自动识别）").build();
        let smtp_host = adw::EntryRow::builder().title("SMTP 服务器（留空自动识别）").build();
        for r in [email.upcast_ref::<gtk::Widget>(), password.upcast_ref(), name.upcast_ref(), imap_host.upcast_ref(), smtp_host.upcast_ref()] {
            imap.add(r);
        }
        let add = gtk::Button::builder().label("添加").halign(gtk::Align::End).margin_top(12).build();
        add.add_css_class("suggested-action");
        add.add_css_class("pill");
        imap.add(&add);
        page.add(&imap);
        {
            let (u, d) = (self.clone(), dialog.clone());
            add.connect_clicked(move |btn| {
                let parse = |s: String, port| (!s.trim().is_empty()).then(|| Server::parse(&s, port));
                let (e, p, n) = (email.text().to_string(), password.text().to_string(), name.text().to_string());
                let (i, s) = (parse(imap_host.text().to_string(), 993), parse(smtp_host.text().to_string(), 465));
                btn.set_sensitive(false);
                btn.set_label("正在验证…");
                let (eng, d, btn) = (u.eng.clone(), d.clone(), btn.clone());
                u.spawn(
                    async move { eng.add_password(&e, &p, &n, i.transpose()?, s.transpose()?).await },
                    move |u, r| {
                        btn.set_sensitive(true);
                        btn.set_label("添加");
                        if let Some(a) = u.report(r) {
                            u.toast(&format!("已添加 {}", a.email));
                            d.close();
                            u.after_accounts_changed();
                        }
                    },
                );
            });
        }
        let view = adw::ToolbarView::new();
        view.add_top_bar(&header);
        view.set_content(Some(&page));
        dialog.set_child(Some(&view));
        dialog.present(Some(&self.window));
    }

    fn settings_dialog(self: &Rc<Self>) {
        let cfg = self.eng.config();
        let dialog = adw::PreferencesDialog::new();
        dialog.set_title("设置");

        let accounts_page = adw::PreferencesPage::builder().title("账号").icon_name("avatar-default-symbolic").build();
        let group = adw::PreferencesGroup::builder().title("邮箱账号").build();
        for a in self.state.borrow().accounts.clone() {
            let row = adw::ActionRow::builder().title(a.email.as_str()).subtitle(if a.master { "总账号 · 账号列表同步在这里" } else { a.provider.label() }).build();
            let avatar = adw::Avatar::new(32, Some(&a.email), true);
            row.add_prefix(&avatar);
            let remove = gtk::Button::builder().icon_name("user-trash-symbolic").valign(gtk::Align::Center).tooltip_text("删除账号").build();
            remove.add_css_class("flat");
            let (u, email, row2, group2) = (self.clone(), a.email.clone(), row.clone(), group.clone());
            remove.connect_clicked(move |_| {
                let (eng, email) = (u.eng.clone(), email.clone());
                let (row, group) = (row2.clone(), group2.clone());
                u.spawn(async move { eng.remove_account(&email).await }, move |u, r| {
                    if u.report(r).is_some() {
                        group.remove(&row);
                        u.after_accounts_changed();
                    }
                });
            });
            row.add_suffix(&remove);
            group.add(&row);
        }
        accounts_page.add(&group);
        dialog.add(&accounts_page);

        let sync_page = adw::PreferencesPage::builder().title("同步").icon_name("view-refresh-symbolic").build();
        let sync_group = adw::PreferencesGroup::builder()
            .title("同步")
            .description("账号列表保存在总账号 Google Drive 的隐藏目录；设置同步密码后先加密再上传。")
            .build();
        let interval = adw::SpinRow::with_range(30.0, 3600.0, 30.0);
        interval.set_title("同步间隔（秒）");
        interval.set_value(cfg.sync_interval_secs as f64);
        let initial = adw::SpinRow::with_range(50.0, 20000.0, 50.0);
        initial.set_title("首次同步封数");
        initial.set_value(cfg.initial_limit as f64);
        let passphrase = adw::PasswordEntryRow::builder().title("云同步密码（可选）").text(cfg.sync_passphrase.clone().unwrap_or_default()).build();
        sync_group.add(&interval);
        sync_group.add(&initial);
        sync_group.add(&passphrase);
        sync_page.add(&sync_group);
        dialog.add(&sync_page);

        let oauth_page = adw::PreferencesPage::builder().title("OAuth").icon_name("dialog-password-symbolic").build();
        let google = adw::PreferencesGroup::builder().title("Google").description("在 Google Cloud Console 创建“桌面应用”类型的 OAuth 客户端（见 README）").build();
        let gid = adw::EntryRow::builder().title("客户端 ID").text(cfg.google_client_id.clone().unwrap_or_default()).build();
        let gsecret = adw::PasswordEntryRow::builder().title("客户端密钥").text(cfg.google_client_secret.clone().unwrap_or_default()).build();
        google.add(&gid);
        google.add(&gsecret);
        let ms = adw::PreferencesGroup::builder().title("Microsoft").build();
        let mid = adw::EntryRow::builder().title("应用程序（客户端）ID").text(cfg.microsoft_client_id.clone().unwrap_or_default()).build();
        ms.add(&mid);
        oauth_page.add(&google);
        oauth_page.add(&ms);
        dialog.add(&oauth_page);

        let u = self.clone();
        dialog.connect_closed(move |_| {
            let opt = |s: glib::GString| Some(s.trim().to_owned()).filter(|s| !s.is_empty());
            let new = Config {
                google_client_id: opt(gid.text()),
                google_client_secret: opt(gsecret.text()),
                microsoft_client_id: opt(mid.text()),
                sync_passphrase: opt(passphrase.text()),
                sync_interval_secs: interval.value() as u64,
                initial_limit: initial.value() as u32,
                ..u.eng.config()
            };
            u.report(u.eng.set_config(new));
        });
        dialog.present(Some(&self.window));
    }
}

fn folder_look(role: &str, name: &str) -> (&'static str, String) {
    let icon = match role {
        "inbox" => "mail-read-symbolic",
        "sent" => "mail-send-symbolic",
        "drafts" => "document-edit-symbolic",
        "trash" => "user-trash-symbolic",
        "junk" => "mail-mark-junk-symbolic",
        "archive" | "all" => "folder-download-symbolic",
        "flagged" => "starred-symbolic",
        _ => "folder-symbolic",
    };
    let title = if name.eq_ignore_ascii_case("INBOX") {
        "收件箱".to_owned()
    } else {
        name.rsplit(['/', '.']).next().unwrap_or(name).to_owned()
    };
    (icon, title)
}

/// Dev/CI helper: with APARK_SNAPSHOT=<dir>, select the first message, save PNGs, quit.
fn snapshot(ui: Rc<Ui>, dir: String) {
    let _ = std::fs::create_dir_all(&dir);
    glib::timeout_add_seconds_local_once(2, move || {
        if ui.store.n_items() > 0 {
            ui.selection.set_selected(0);
        }
        glib::timeout_add_seconds_local_once(2, move || {
            save_widget(&ui.window, &PathBuf::from(&dir).join("main.png"));
            ui.window.application().map(|a| a.quit());
        });
    });
}

fn save_widget(w: &impl IsA<gtk::Widget>, path: &std::path::Path) {
    let w = w.as_ref();
    let paintable = gtk::WidgetPaintable::new(Some(w));
    let (width, height) = (w.width() as f64, w.height() as f64);
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(&snapshot, width, height);
    let Some(node) = snapshot.to_node() else { return };
    let Some(renderer) = w.native().and_then(|n| n.renderer()) else { return };
    let texture = renderer.render_texture(&node, Some(&gtk::graphene::Rect::new(0.0, 0.0, width as f32, height as f32)));
    let _ = texture.save_to_png(path);
}
