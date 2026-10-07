//! `apark` command line. Built for both people and AI agents: every command is
//! non-interactive when given its flags, and `--json` gives stable machine output.

use std::ffi::OsString;
use std::io::{IsTerminal, Read};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use apark_core::account::Server;
use apark_core::rpc::account_json;
use apark_core::{categorize, Engine, ListQuery, LoginOpts, MsgRow, Outgoing, Provider};
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde_json::json;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

mod guide;

#[derive(Parser)]
#[command(name = "apark", version, about = "Apark —— 快速的多账号邮件客户端（CLI / 无头版）", after_help = "给 AI agent 的完整说明: apark guide")]
struct Cli {
    /// 输出 JSON（机器可读）
    #[arg(long, global = true, env = "APARK_JSON")]
    json: bool,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// 用 Google 总账号登录，并恢复云端同步的其他邮箱
    Login(LoginArgs),
    /// 添加邮箱账号
    #[command(subcommand)]
    Add(AddCmd),
    /// 列出账号
    Accounts,
    /// 删除账号（同时删除本地缓存）
    Remove { email: String },
    /// 立即同步（默认全部账号的收件箱）
    Sync {
        #[arg(long, short)]
        account: Option<String>,
        /// 额外同步的文件夹（需配合 --account）
        #[arg(long, short)]
        folder: Vec<String>,
    },
    /// 列出账号的文件夹
    Folders {
        #[arg(long, short)]
        account: Option<String>,
    },
    /// 新建、重命名或删除文件夹
    Folder {
        #[arg(value_enum)]
        action: FolderAction,
        #[arg(long, short)]
        account: String,
        name: String,
        /// 重命名的新名字
        new_name: Option<String>,
    },
    /// 列出邮件（默认：所有账号的收件箱）
    List(ListArgs),
    /// 搜索本地邮件（主题、发件人、收件人、正文）
    Search {
        query: Vec<String>,
        #[command(flatten)]
        filter: Filter,
    },
    /// 阅读邮件
    Read {
        id: i64,
        /// 输出 HTML 正文
        #[arg(long)]
        html: bool,
        /// 输出原始 RFC 822 邮件
        #[arg(long)]
        raw: bool,
        /// 同时标为已读
        #[arg(long)]
        mark_read: bool,
    },
    /// 发送新邮件
    Send(SendArgs),
    /// 回复邮件（自动引用原文、设置会话头）
    Reply {
        id: i64,
        /// 回复全部
        #[arg(long)]
        all: bool,
        /// 不附带原文引用
        #[arg(long)]
        no_quote: bool,
        #[command(flatten)]
        body: BodyArgs,
        #[arg(long)]
        attach: Vec<PathBuf>,
    },
    /// 转发邮件（带原附件）
    Forward {
        id: i64,
        #[arg(long, required = true)]
        to: Vec<String>,
        #[arg(long)]
        cc: Vec<String>,
        #[command(flatten)]
        body: BodyArgs,
    },
    /// 标记邮件
    Mark {
        #[arg(value_enum)]
        how: Mark,
        #[arg(required = true)]
        ids: Vec<i64>,
    },
    /// 归档邮件
    Archive {
        #[arg(required = true)]
        ids: Vec<i64>,
    },
    /// 移到废纸篓
    Trash {
        #[arg(required = true)]
        ids: Vec<i64>,
    },
    /// 移动到指定文件夹
    Move {
        #[arg(long)]
        to: String,
        #[arg(required = true)]
        ids: Vec<i64>,
    },
    /// 列出或保存附件
    Attachments {
        id: i64,
        /// 保存到目录
        #[arg(long)]
        save: Option<PathBuf>,
        /// 只保存第 N 个附件（从 0 开始）
        #[arg(long)]
        index: Option<usize>,
    },
    /// 持续同步，每封新邮件输出一行 JSON（给 agent 订阅新邮件）
    Watch {
        /// 同步间隔（秒），默认取配置
        #[arg(long)]
        interval: Option<u64>,
        /// 只输出收件箱的新邮件
        #[arg(long)]
        inbox_only: bool,
    },
    /// 无头模式：常驻后台定时同步
    Daemon {
        /// 同步间隔（秒），默认取配置
        #[arg(long)]
        interval: Option<u64>,
    },
    /// 账号同步：Google、自建服务器、WebDAV 或本地同步文件夹
    #[command(subcommand)]
    Cloud(CloudCmd),
    /// 运行自建同步服务器（存放加密的账号列表）
    Server {
        /// 监听地址
        #[arg(long, default_value = "0.0.0.0:8787")]
        listen: String,
        /// 数据目录
        #[arg(long, default_value = "./apark-sync-data")]
        data: PathBuf,
        /// 访问令牌（客户端需要提供），也可用环境变量 APARK_SERVER_TOKEN
        #[arg(long, env = "APARK_SERVER_TOKEN")]
        token: Option<String>,
    },
    /// 查看或修改配置
    #[command(subcommand)]
    Config(ConfigCmd),
    /// 打印给 AI agent 的使用说明
    Guide,
    /// 写入演示数据（截图/界面开发用）
    #[command(hide = true)]
    DevSeed,
}

#[derive(Args)]
struct LoginArgs {
    /// 无浏览器环境：手动打开链接，再粘贴跳转地址
    #[arg(long)]
    manual: bool,
}

#[derive(Subcommand)]
enum AddCmd {
    /// Gmail / Google Workspace（OAuth）
    Google(LoginArgs),
    /// Outlook / Hotmail / Microsoft 365（OAuth）
    Microsoft(LoginArgs),
    /// 其他邮箱：IMAP + SMTP（密码或应用专用密码）
    Imap {
        #[arg(long)]
        email: String,
        #[arg(long, default_value = "")]
        name: String,
        /// 从 stdin 读取密码
        #[arg(long)]
        password_stdin: bool,
        /// 从环境变量读取密码
        #[arg(long)]
        password_env: Option<String>,
        /// IMAP 服务器 host[:port]，默认按域名推断（端口 993，SSL）
        #[arg(long)]
        imap: Option<String>,
        /// SMTP 服务器 host[:port]，465 为 SSL，其他端口用 STARTTLS
        #[arg(long)]
        smtp: Option<String>,
    },
}

#[derive(Args, Default)]
struct Filter {
    #[arg(long, short)]
    account: Option<String>,
    #[arg(long, short)]
    folder: Option<String>,
    #[arg(long, default_value_t = 50)]
    limit: usize,
}

#[derive(Args)]
struct ListArgs {
    #[command(flatten)]
    filter: Filter,
    /// 智能收件箱分类
    #[arg(long, short, value_enum)]
    category: Option<Category>,
    #[arg(long)]
    unread: bool,
    #[arg(long)]
    flagged: bool,
    /// 列出前先同步
    #[arg(long)]
    sync: bool,
}

#[derive(Clone, Copy, ValueEnum)]
enum Category {
    People,
    Notification,
    Newsletter,
}

#[derive(Clone, Copy, ValueEnum)]
enum Mark {
    Read,
    Unread,
    Star,
    Unstar,
}

#[derive(Clone, Copy, ValueEnum)]
enum FolderAction {
    Create,
    Rename,
    Delete,
}

#[derive(Subcommand)]
enum CloudCmd {
    /// 查看当前同步方式
    Status,
    /// 立即同步账号列表（新的一方覆盖旧的一方）
    Sync,
    /// 把本机账号列表上传覆盖云端
    Push,
    /// 用自建同步服务器（apark server）同步
    Server {
        #[arg(long)]
        url: String,
        #[arg(long)]
        user: String,
        /// 服务器访问令牌（如果服务器设置了）
        #[arg(long, env = "APARK_SERVER_TOKEN")]
        token: Option<String>,
        #[command(flatten)]
        secret: SecretArgs,
    },
    /// 用 WebDAV 同步（坚果云、Nextcloud、NAS…）
    Webdav {
        /// 账号列表文件的完整 URL，例如 https://dav.jianguoyun.com/dav/apark/accounts.json
        #[arg(long)]
        url: String,
        #[arg(long)]
        user: String,
        #[command(flatten)]
        secret: SecretArgs,
    },
    /// 用本地同步文件夹（iCloud Drive、Dropbox、Syncthing…）里的文件同步
    File {
        #[arg(long)]
        path: String,
    },
    /// 停止同步账号列表
    Off,
}

#[derive(Args)]
struct SecretArgs {
    /// 从环境变量读取密码（服务器/WebDAV 的登录密码）
    #[arg(long)]
    password_env: Option<String>,
    /// 从 stdin 第一行读取密码
    #[arg(long)]
    password_stdin: bool,
}

fn read_secret(a: &SecretArgs, prompt: &str) -> Result<String> {
    if let Some(var) = &a.password_env {
        return std::env::var(var).with_context(|| format!("环境变量 {var} 未设置"));
    }
    if a.password_stdin {
        let mut s = String::new();
        std::io::stdin().read_line(&mut s)?;
        return Ok(s.trim_end_matches(['\r', '\n']).to_owned());
    }
    Ok(rpassword::prompt_password(prompt)?)
}

/// Encryption passphrase for WebDAV / file sync: APARK_SYNC_PASSPHRASE, config, or prompt.
fn sync_passphrase(eng: &Engine) -> Result<Option<String>> {
    if eng.config().passphrase().is_some() {
        return Ok(None);
    }
    let p = rpassword::prompt_password("设置同步密码（用来加密账号列表，每台设备填写相同的密码）: ")?;
    if p.is_empty() {
        bail!("同步密码不能为空");
    }
    Ok(Some(p))
}

#[derive(Args)]
struct BodyArgs {
    /// 正文；省略时读取 --body-file 或 stdin
    #[arg(long)]
    body: Option<String>,
    #[arg(long)]
    body_file: Option<PathBuf>,
    /// HTML 正文文件（作为纯文本的替代版本）
    #[arg(long)]
    html_file: Option<PathBuf>,
}

#[derive(Args)]
struct SendArgs {
    /// 发件账号（默认第一个账号）
    #[arg(long)]
    from: Option<String>,
    #[arg(long)]
    to: Vec<String>,
    #[arg(long)]
    cc: Vec<String>,
    #[arg(long)]
    bcc: Vec<String>,
    #[arg(long, short, default_value = "")]
    subject: String,
    #[command(flatten)]
    body: BodyArgs,
    #[arg(long)]
    attach: Vec<PathBuf>,
    /// 从 stdin 读取 JSON：{"from","to":[],"cc":[],"bcc":[],"subject","body","html","attachments":[]}
    #[arg(long)]
    stdin_json: bool,
}

#[derive(Subcommand)]
enum ConfigCmd {
    /// 显示配置（密钥脱敏）与数据目录
    Show,
    /// 设置：google_client_id google_client_secret microsoft_client_id sync_passphrase sync_interval_secs initial_limit prefetch_kb
    Set { key: String, value: String },
    Unset { key: String },
}

pub fn run(args: impl IntoIterator<Item = OsString>) -> ExitCode {
    let cli = match Cli::try_parse_from(args) {
        Ok(c) => c,
        Err(e) => {
            let _ = e.print();
            return if e.use_stderr() { ExitCode::from(2) } else { ExitCode::SUCCESS };
        }
    };
    let json = cli.json;
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("tokio runtime");
    match rt.block_on(dispatch(cli)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            if json {
                println!("{}", json!({ "ok": false, "error": format!("{e:#}") }));
            } else {
                eprintln!("错误: {e:#}");
            }
            ExitCode::FAILURE
        }
    }
}

fn out(json: bool, value: serde_json::Value, human: impl FnOnce()) {
    if json {
        println!("{}", serde_json::to_string_pretty(&value).unwrap_or_default());
    } else {
        human();
    }
}

fn ok(json: bool, msg: &str) {
    out(json, json!({ "ok": true, "message": msg }), || println!("{msg}"));
}

fn login_opts(manual: bool) -> LoginOpts {
    LoginOpts {
        manual,
        on_url: Arc::new(|url| eprintln!("在浏览器中打开以下链接完成授权：\n{url}\n")),
    }
}


fn read_body(b: &BodyArgs) -> Result<(String, Option<String>)> {
    let text = match (&b.body, &b.body_file) {
        (Some(t), _) => t.clone(),
        (None, Some(f)) => std::fs::read_to_string(f).with_context(|| format!("读取 {}", f.display()))?,
        (None, None) if !std::io::stdin().is_terminal() => {
            let mut s = String::new();
            std::io::stdin().read_to_string(&mut s)?;
            s
        }
        (None, None) => bail!("缺少正文：用 --body、--body-file 或通过 stdin 传入"),
    };
    let html = b.html_file.as_ref().map(std::fs::read_to_string).transpose()?;
    Ok((text, html))
}

fn default_from(eng: &Engine, from: Option<String>) -> Result<String> {
    match from {
        Some(f) => Ok(eng.account(&f)?.email),
        None => eng.accounts().first().map(|a| a.email.clone()).context("还没有账号，先运行 apark login 或 apark add"),
    }
}

fn mask(s: &Option<String>) -> serde_json::Value {
    match s {
        Some(v) if v.len() > 8 => json!(format!("{}…{}", &v[..4], &v[v.len() - 4..])),
        Some(_) => json!("****"),
        None => serde_json::Value::Null,
    }
}

async fn dispatch(cli: Cli) -> Result<()> {
    let json = cli.json;
    if let Cmd::Guide = cli.cmd {
        print!("{}", guide::GUIDE);
        return Ok(());
    }
    if let Cmd::Server { listen, data, token } = cli.cmd {
        apark_core::init_crypto();
        return apark_core::syncserver::serve(&listen, data, token).await;
    }
    let eng = Engine::open()?;
    match cli.cmd {
        Cmd::Guide => unreachable!(),
        Cmd::DevSeed => {
            let n = apark_core::demo::seed(&eng)?;
            ok(json, &format!("已写入 {n} 封演示邮件"));
        }
        Cmd::Login(a) => {
            let (acct, res) = eng.login_master(&login_opts(a.manual)).await?;
            out(json, json!({ "ok": true, "master": acct.email, "cloud": res }), || {
                println!("已登录总账号 {}", acct.email);
                if !res.added.is_empty() {
                    println!("从云端恢复了 {} 个账号: {}", res.added.len(), res.added.join(", "));
                }
            });
        }
        Cmd::Add(add) => {
            let acct = match add {
                AddCmd::Google(a) => eng.add_oauth(Provider::Google, &login_opts(a.manual)).await?,
                AddCmd::Microsoft(a) => eng.add_oauth(Provider::Microsoft, &login_opts(a.manual)).await?,
                AddCmd::Imap { email, name, password_stdin, password_env, imap, smtp } => {
                    let password = if password_stdin {
                        let mut s = String::new();
                        std::io::stdin().read_line(&mut s)?;
                        s.trim_end_matches(['\r', '\n']).to_owned()
                    } else if let Some(var) = password_env {
                        std::env::var(&var).with_context(|| format!("环境变量 {var} 未设置"))?
                    } else {
                        rpassword::prompt_password(format!("{email} 的密码（或应用专用密码）: "))?
                    };
                    let imap = imap.map(|s| Server::parse(&s, 993)).transpose()?;
                    let smtp = smtp.map(|s| Server::parse(&s, 465)).transpose()?;
                    eng.add_password(&email, &password, &name, imap, smtp).await?
                }
            };
            out(json, json!({ "ok": true, "account": account_json(&acct) }), || {
                println!("已添加 {}（{}）", acct.email, acct.provider.label())
            });
        }
        Cmd::Accounts => {
            let accts = eng.accounts();
            out(json, json!(accts.iter().map(account_json).collect::<Vec<_>>()), || {
                if accts.is_empty() {
                    println!("还没有账号。运行 apark login 用 Google 总账号登录。");
                }
                for a in &accts {
                    println!(
                        "{}{}  {}  {}",
                        a.email,
                        if a.master { " [总账号]" } else { "" },
                        a.provider.label(),
                        a.name
                    );
                }
            });
        }
        Cmd::Remove { email } => {
            eng.remove_account(&email).await?;
            ok(json, &format!("已删除 {email}"));
        }
        Cmd::Sync { account, folder } => {
            let results: Vec<(String, Result<usize>)> = match account {
                Some(a) => {
                    let email = eng.account(&a)?.email;
                    let r = eng.sync_account(&email, &folder).await;
                    vec![(email, r)]
                }
                None => eng.sync_all().await.unwrap_or_default(),
            };
            print_sync(json, &results);
            if results.iter().all(|(_, r)| r.is_err()) && !results.is_empty() {
                bail!("同步失败");
            }
        }
        Cmd::Folders { account } => {
            let emails: Vec<String> = match account {
                Some(a) => vec![eng.account(&a)?.email],
                None => eng.accounts().into_iter().map(|a| a.email).collect(),
            };
            let mut all = vec![];
            for e in &emails {
                let mut f = eng.folders(e)?;
                if f.is_empty() {
                    eng.sync_account(e, &[]).await?;
                    f = eng.folders(e)?;
                }
                all.push((e.clone(), f));
            }
            out(
                json,
                json!(all.iter().map(|(e, f)| json!({ "account": e, "folders": f })).collect::<Vec<_>>()),
                || {
                    for (e, fs) in &all {
                        println!("{e}");
                        for f in fs {
                            println!("  {}{}", f.label, if f.role.is_empty() { String::new() } else { format!("  [{}]", f.role) });
                        }
                    }
                },
            );
        }
        Cmd::Folder { action, account, name, new_name } => {
            let email = eng.account(&account)?.email;
            match action {
                FolderAction::Create => eng.edit_folder(&email, &name, true, None).await?,
                FolderAction::Rename => {
                    let to = new_name.context("rename 需要新名字")?;
                    eng.edit_folder(&email, &name, false, Some(&to)).await?
                }
                FolderAction::Delete => eng.edit_folder(&email, &name, false, None).await?,
            }
            ok(json, "完成");
        }
        Cmd::List(a) => {
            // A folder that was never synced would look empty; fetch it first.
            let mut a = a;
            if let (Some(acct), Some(folder)) = (&a.filter.account, &a.filter.folder) {
                let email = eng.account(acct)?.email;
                let raw = eng.store.resolve_folder(&email, folder)?;
                if eng.store.folder_state(&email, &raw)? == (0, 0) {
                    eng.sync_account(&email, std::slice::from_ref(&raw)).await?;
                }
                a.filter.folder = Some(raw);
            }
            if a.sync {
                let r = eng.sync_all().await.unwrap_or_default();
                if !json {
                    print_sync(false, &r);
                }
            }
            let q = ListQuery {
                account: a.filter.account,
                folder: a.filter.folder,
                category: a.category.map(|c| match c {
                    Category::People => "people",
                    Category::Notification => "notification",
                    Category::Newsletter => "newsletter",
                }
                .to_owned()),
                unread: a.unread,
                flagged: a.flagged,
                search: None,
                limit: a.filter.limit,
            };
            print_rows(json, &eng.list(&q)?);
        }
        Cmd::Search { query, filter } => {
            let q = ListQuery {
                account: filter.account,
                folder: filter.folder,
                search: Some(query.join(" ")),
                limit: filter.limit,
                ..Default::default()
            };
            print_rows(json, &eng.list(&q)?);
        }
        Cmd::Read { id, html, raw, mark_read } => {
            if raw {
                use std::io::Write;
                std::io::stdout().write_all(&eng.raw(id).await?)?;
                return Ok(());
            }
            let m = eng.get(id)?;
            let body = eng.body(id).await?;
            if mark_read && !m.seen {
                eng.set_seen(id, true).await?;
            }
            out(json, json!({ "message": m, "body": body }), || {
                println!("主题: {}", m.subject);
                println!("发件人: {} <{}>", m.from_name, m.from_addr);
                println!("收件人: {}", m.to);
                if !m.cc.is_empty() {
                    println!("抄送: {}", m.cc);
                }
                println!("日期: {}", fmt_time(m.date, true));
                println!("账号: {}  文件夹: {}  分类: {}", m.account, m.folder, categorize::label(&m.category));
                for (i, a) in body.attachments.iter().enumerate() {
                    println!("附件 {i}: {} ({})", a.name, human_size(a.size));
                }
                println!("{}", "─".repeat(60));
                match (&body.html, html) {
                    (Some(h), true) => println!("{h}"),
                    _ => println!("{}", body.text),
                }
            });
        }
        Cmd::Send(a) => {
            let (from, msg) = if a.stdin_json {
                let mut s = String::new();
                std::io::stdin().read_to_string(&mut s)?;
                let v: serde_json::Value = serde_json::from_str(&s).context("stdin 不是有效 JSON")?;
                let from = v["from"].as_str().map(str::to_owned);
                (from, serde_json::from_value::<Outgoing>(v)?)
            } else {
                let (body, html) = read_body(&a.body)?;
                (
                    a.from,
                    Outgoing {
                        to: a.to,
                        cc: a.cc,
                        bcc: a.bcc,
                        subject: a.subject,
                        body,
                        html,
                        attachments: a.attach,
                        ..Default::default()
                    },
                )
            };
            if msg.to.is_empty() && msg.cc.is_empty() && msg.bcc.is_empty() {
                bail!("至少需要一个收件人（--to）");
            }
            let from = default_from(&eng, from)?;
            eng.send(&from, &msg).await?;
            ok(json, &format!("已从 {from} 发送"));
        }
        Cmd::Reply { id, all, no_quote, body, attach } => {
            let (from, mut draft) = eng.reply_draft(id, all).await?;
            let (text, html) = read_body(&body)?;
            draft.body = if no_quote { text } else { format!("{text}{}", draft.body) };
            draft.html = html;
            draft.attachments = attach;
            eng.send(&from, &draft).await?;
            ok(json, &format!("已回复（{}）", draft.to.join(", ")));
        }
        Cmd::Forward { id, to, cc, body } => {
            let (from, mut draft) = eng.forward_draft(id).await?;
            let (text, _) = read_body(&body).unwrap_or_default();
            draft.to = to;
            draft.cc = cc;
            draft.body = format!("{text}{}", draft.body);
            eng.send(&from, &draft).await?;
            ok(json, &format!("已转发给 {}", draft.to.join(", ")));
        }
        Cmd::Mark { how, ids } => {
            for id in &ids {
                match how {
                    Mark::Read => eng.set_seen(*id, true).await?,
                    Mark::Unread => eng.set_seen(*id, false).await?,
                    Mark::Star => eng.set_flagged(*id, true).await?,
                    Mark::Unstar => eng.set_flagged(*id, false).await?,
                }
            }
            ok(json, &format!("已标记 {} 封", ids.len()));
        }
        Cmd::Archive { ids } => {
            for id in &ids {
                eng.archive(*id).await?;
            }
            ok(json, &format!("已归档 {} 封", ids.len()));
        }
        Cmd::Trash { ids } => {
            for id in &ids {
                eng.trash(*id).await?;
            }
            ok(json, &format!("已移到废纸篓 {} 封", ids.len()));
        }
        Cmd::Move { to, ids } => {
            for id in &ids {
                eng.move_to(*id, &to).await?;
            }
            ok(json, &format!("已移动 {} 封到 {to}", ids.len()));
        }
        Cmd::Attachments { id, save, index } => match save {
            Some(dir) => {
                let paths = eng.save_attachments(id, index, &dir).await?;
                out(json, json!({ "ok": true, "saved": paths }), || {
                    for p in &paths {
                        println!("{}", p.display());
                    }
                });
            }
            None => {
                let body = eng.body(id).await?;
                out(json, json!(body.attachments), || {
                    for (i, a) in body.attachments.iter().enumerate() {
                        println!("{i}  {}  {}", a.name, human_size(a.size));
                    }
                });
            }
        },
        Cmd::Daemon { interval } => {
            let secs = interval.unwrap_or_else(|| eng.config().sync_interval_secs).max(15);
            eprintln!("Apark 无头同步已启动，每 {secs} 秒同步一次（Ctrl-C 退出）");
            loop {
                if let Some(r) = eng.sync_all().await {
                    print_sync(json, &r);
                }
                tokio::select! {
                    _ = tokio::time::sleep(std::time::Duration::from_secs(secs)) => {}
                    _ = tokio::signal::ctrl_c() => break,
                }
            }
        }
        Cmd::Watch { interval, inbox_only } => {
            use std::io::Write;
            let secs = interval.unwrap_or_else(|| eng.config().sync_interval_secs).max(15);
            let mut last = eng.store.max_id()?;
            eprintln!("apark watch: 每 {secs} 秒同步一次，新邮件以 JSON 行输出到 stdout（Ctrl-C 退出）");
            loop {
                if let Some(results) = eng.sync_all().await {
                    for (account, r) in &results {
                        if let Err(e) = r {
                            eprintln!("{account}: {e:#}");
                        }
                    }
                    for m in eng.store.since(last)? {
                        last = last.max(m.id);
                        if inbox_only && m.folder != "INBOX" {
                            continue;
                        }
                        println!("{}", serde_json::to_string(&json!({ "type": "new_message", "message": m }))?);
                    }
                    std::io::stdout().flush()?;
                }
                tokio::select! {
                    _ = tokio::time::sleep(std::time::Duration::from_secs(secs)) => {}
                    _ = tokio::signal::ctrl_c() => break,
                }
            }
        }
        Cmd::Cloud(c) => {
            let res = match c {
                CloudCmd::Status => {
                    let t = eng.sync_target();
                    out(json, json!(t.as_ref().map(|t| json!({ "type": t.kind(), "where": t.describe() }))), || match &t {
                        Some(t) => println!("账号同步：{}（{}）", t.kind(), t.describe()),
                        None => println!("没有开启账号同步。可选：apark login（Google）/ apark cloud server|webdav|file"),
                    });
                    return Ok(());
                }
                CloudCmd::Sync => eng.cloud_sync().await?,
                CloudCmd::Push => {
                    eng.cloud_push().await?;
                    ok(json, "已上传账号列表");
                    return Ok(());
                }
                CloudCmd::Off => {
                    eng.disable_sync()?;
                    ok(json, "已停止账号同步（本机账号保留）");
                    return Ok(());
                }
                CloudCmd::Server { url, user, token, secret } => {
                    let password = read_secret(&secret, &format!("{user} 在同步服务器上的密码（也用于加密）: "))?;
                    eng.setup_sync(apark_core::SyncTarget::Server { url, user, password, token }, None).await?
                }
                CloudCmd::Webdav { url, user, secret } => {
                    let password = read_secret(&secret, &format!("{user} 的 WebDAV 密码: "))?;
                    let pass = sync_passphrase(&eng)?;
                    eng.setup_sync(apark_core::SyncTarget::Webdav { url, user, password }, pass).await?
                }
                CloudCmd::File { path } => {
                    let pass = sync_passphrase(&eng)?;
                    eng.setup_sync(apark_core::SyncTarget::File { path }, pass).await?
                }
            };
            out(json, json!(res), || {
                println!("账号同步：{}", res.action);
                if !res.added.is_empty() {
                    println!("新增账号：{}", res.added.join(", "));
                }
                if !res.removed.is_empty() {
                    println!("移除账号：{}", res.removed.join(", "));
                }
            });
        }
        Cmd::Server { .. } => unreachable!(),
        Cmd::Config(c) => {
            let mut cfg = eng.config();
            match c {
                ConfigCmd::Show => {
                    let v = json!({
                        "data_dir": apark_core::paths::home(),
                        "google_client_id": cfg.google_client().map(|c| c.0),
                        "google_client_secret": mask(&cfg.google_client().and_then(|c| c.1)),
                        "microsoft_client_id": cfg.microsoft_client(),
                        "sync_passphrase": mask(&cfg.passphrase()),
                        "sync_interval_secs": cfg.sync_interval_secs,
                        "initial_limit": cfg.initial_limit,
                        "prefetch_kb": cfg.prefetch_kb,
                    });
                    println!("{}", serde_json::to_string_pretty(&v)?);
                }
                ConfigCmd::Set { key, value } => {
                    set_key(&mut cfg, &key, Some(value))?;
                    eng.set_config(cfg)?;
                    ok(json, &format!("已设置 {key}"));
                }
                ConfigCmd::Unset { key } => {
                    set_key(&mut cfg, &key, None)?;
                    eng.set_config(cfg)?;
                    ok(json, &format!("已清除 {key}"));
                }
            }
        }
    }
    Ok(())
}

fn set_key(cfg: &mut apark_core::Config, key: &str, value: Option<String>) -> Result<()> {
    let num = |v: &Option<String>, default: u64| -> Result<u64> {
        v.as_deref().map(|s| s.parse().context("需要数字")).transpose().map(|n| n.unwrap_or(default))
    };
    match key {
        "google_client_id" => cfg.google_client_id = value,
        "google_client_secret" => cfg.google_client_secret = value,
        "microsoft_client_id" => cfg.microsoft_client_id = value,
        "sync_passphrase" => cfg.sync_passphrase = value,
        "sync_interval_secs" => cfg.sync_interval_secs = num(&value, 120)?,
        "initial_limit" => cfg.initial_limit = num(&value, 500)? as u32,
        "prefetch_kb" => cfg.prefetch_kb = num(&value, 512)? as u32,
        _ => bail!("未知配置项: {key}"),
    }
    Ok(())
}

fn print_sync(json: bool, results: &[(String, Result<usize>)]) {
    let v: Vec<_> = results
        .iter()
        .map(|(a, r)| match r {
            Ok(n) => json!({ "account": a, "ok": true, "new": n }),
            Err(e) => json!({ "account": a, "ok": false, "error": format!("{e:#}") }),
        })
        .collect();
    if json {
        println!("{}", serde_json::Value::Array(v));
        return;
    }
    for (a, r) in results {
        match r {
            Ok(n) => println!("✓ {a}  新邮件 {n}"),
            Err(e) => println!("✗ {a}  {e:#}"),
        }
    }
}

fn fmt_time(ts: i64, full: bool) -> String {
    use chrono::{Datelike, Local, TimeZone};
    let Some(t) = Local.timestamp_opt(ts, 0).single() else { return String::new() };
    if full {
        return t.format("%Y-%m-%d %H:%M").to_string();
    }
    let now = Local::now();
    if t.date_naive() == now.date_naive() {
        t.format("%H:%M").to_string()
    } else if t.year() == now.year() {
        t.format("%m-%d").to_string()
    } else {
        t.format("%Y-%m-%d").to_string()
    }
}

fn human_size(n: usize) -> String {
    match n {
        n if n >= 1 << 20 => format!("{:.1} MB", n as f64 / (1 << 20) as f64),
        n if n >= 1 << 10 => format!("{:.0} KB", n as f64 / 1024.0),
        n => format!("{n} B"),
    }
}

/// Truncate/pad to a display width (CJK-aware).
fn cell(s: &str, width: usize) -> String {
    let mut out = String::new();
    let mut w = 0;
    for c in s.chars() {
        let cw = c.width().unwrap_or(0);
        if w + cw > width.saturating_sub(1) && s.width() > width {
            out.push('…');
            w += 1;
            break;
        }
        out.push(c);
        w += cw;
    }
    out.push_str(&" ".repeat(width.saturating_sub(w)));
    out
}

fn print_rows(json: bool, rows: &[MsgRow]) {
    out(json, json!(rows), || {
        if rows.is_empty() {
            println!("（没有邮件）");
        }
        for m in rows {
            println!(
                "{:>6} {}{} {} {} {}  {}",
                m.id,
                if m.seen { ' ' } else { '●' },
                if m.flagged { '★' } else { ' ' },
                cell(&fmt_time(m.date, false), 10),
                cell(m.sender(), 20),
                cell(if m.subject.is_empty() { "（无主题）" } else { &m.subject }, 50),
                m.account
            );
        }
    });
}
