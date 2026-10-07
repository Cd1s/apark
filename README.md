# Apark

一个快速、简洁的多账号邮件客户端，思路来自 Spark：**登录一次，所有邮箱自动回来**。同步账号列表可以用 Google，也可以用自己的服务器、WebDAV 或同步文件夹，不依赖任何第三方。

- **原生桌面版**：每个平台用自己的原生界面，不是 Electron / 网页套壳：
  - macOS：**SwiftUI**（通用二进制，Apple Silicon + Intel）
  - Windows：**WinUI 3**（Win11 Fluent 风格、Mica 材质，x64 / ARM64）
  - Linux：**GTK4 + libadwaita**（GNOME 原生风格，x64 / ARM64）
  三个界面共用同一个 Rust 内核，列表都是虚拟化的，几万封邮件也顺滑。
- **CLI / 无头版**：`apark` 单文件，所有功能都能用命令完成，`--json` 输出稳定，适合 AI agent 和服务器。桌面版的可执行文件同样接受全部 CLI 命令。
- **账号同步（四选一）**：
  - **自建服务器**：`apark server` 一行命令跑在任意 VPS 上；账号列表在本机用你的密码加密后才上传，服务器只存密文；
  - **WebDAV**：坚果云、Nextcloud、群晖 NAS……；
  - **同步文件夹**：iCloud Drive、Dropbox、OneDrive、Syncthing 里的一个文件；
  - **Google**：账号列表放在总账号 Google Drive 的隐藏应用目录。

| macOS (SwiftUI) | Windows (WinUI 3) | Linux (GTK4 / libadwaita) |
|---|---|---|
| ![macOS](docs/screenshot-macos.png) | ![Windows](docs/screenshot-windows.png) | ![Linux](docs/screenshot-linux.png) |

## 功能

| | 桌面版 | CLI |
|---|---|---|
| Google 总账号登录、恢复全部账号 | ✓ | `apark login` |
| Gmail / Google Workspace（OAuth） | ✓ | `apark add google` |
| Outlook / Microsoft 365（OAuth） | ✓ | `apark add microsoft` |
| 任意 IMAP/SMTP 邮箱（QQ、163、iCloud、Fastmail、自建…） | ✓ | `apark add imap` |
| 统一收件箱 + 智能分类（个人 / 通知 / 订阅） | ✓ | `apark list -c people` |
| 本地全文搜索（离线、即时） | ✓ | `apark search` |
| 阅读、回复、全部回复、转发（带原附件） | ✓ | `read` `reply` `forward` |
| 归档、删除、移动、星标、已读/未读 | ✓ | `archive` `trash` `move` `mark` |
| 附件下载 / 发送附件（桌面版拖入即添加） | ✓ | `attachments` `--attach` |
| 文件夹管理 | 浏览 | `folder create/rename/delete` |
| 后台定时同步 | ✓ | `apark daemon` |

快捷键（macOS 用 ⌘，Windows/Linux 用 Ctrl）：新邮件 `⌘N`、回复 `⌘R`、全部回复 `⇧⌘R`、转发 `⇧⌘F`、归档 `⌃⌘A`（Win/Linux `Ctrl+E`）、删除 `⌘⌫`（Win/Linux `Delete`）、标为未读 `⇧⌘U`、星标 `⇧⌘L`、收取 `⇧⌘N`（Win/Linux `F5`）、发送 `⌘↩`。

## 下载

在 [Releases](../../releases) 下载对应平台的包：

| 文件 | 内容 |
|---|---|
| `Apark-macos-universal.zip` | `Apark.app`（SwiftUI，内含 CLI） |
| `Apark-windows-x64.zip` / `Apark-windows-arm64.zip` | `Apark.exe`（WinUI 3）+ `apark-cli.exe` + `apark_ffi.dll` |
| `Apark-linux-x64.tar.gz` / `Apark-linux-arm64.tar.gz` | `apark-desktop`（GTK4）+ `apark` + `.desktop` 文件 |
| `apark-cli-*` | 只有 CLI（无头服务器用） |
| `apark-cli-linux-*-static.tar.gz` | 静态链接 CLI，任何 Linux 都能跑 |

macOS 包没有经过 Apple 公证，第一次打开如提示“已损坏”，执行：

```sh
xattr -dr com.apple.quarantine /Applications/Apark.app
```

桌面版本身也能当 CLI 用：`Apark.app/Contents/MacOS/Apark list`、`Apark.exe list`、`apark-desktop list`。
macOS 上让 `apark` 命令可直接调用：`ln -s /Applications/Apark.app/Contents/MacOS/apark-cli /usr/local/bin/apark`。
Linux 需要 GTK 4.12+ 和 libadwaita 1.5+（Ubuntu 24.04、Debian 13、Fedora 40 及更新版本）。

## 第一次使用

### 方式一：自建同步服务器（不需要 Google）

服务器上（任意 Linux，用静态版 CLI 即可）：

```sh
apark server --listen 127.0.0.1:8787 --data /var/lib/apark-sync --token 一串随机令牌
# 再用 Caddy / nginx / Cloudflare Tunnel 加上 HTTPS，例如 https://sync.example.com
```

每台设备：桌面版点“使用自建服务器 / WebDAV / 同步文件夹”，或者命令行：

```sh
apark cloud server --url https://sync.example.com --user me --token 一串随机令牌   # 会提示输入密码
apark add imap --email me@qq.com        # 第一台设备上添加邮箱，之后会自动同步到其他设备
```

密码同时用来加密：服务器只能看到一段密文，也不知道它属于谁。忘记密码就无法解密，只能重新添加邮箱。

### 方式二：WebDAV / 同步文件夹

```sh
export APARK_SYNC_PASSPHRASE=你的同步密码      # 用来加密账号列表，每台设备相同
apark cloud webdav --url https://dav.jianguoyun.com/dav/apark/accounts.json --user me@example.com
apark cloud file --path ~/Dropbox/Apark/accounts.json
```

### 方式三：Google 总账号

Google 不允许第三方客户端共用别人的 OAuth 凭据访问邮箱，所以需要用你自己的 Google Cloud 项目创建一个（一次性，5 分钟）：

1. 打开 [Google Cloud Console](https://console.cloud.google.com/)，新建项目。
2. **API 和服务 → 库**：启用 **Google Drive API**（账号同步用；IMAP 不需要额外启用 API）。
3. **OAuth 同意屏幕**：用户类型选“外部”，填应用名 Apark；
   作用域添加 `https://mail.google.com/` 和 `.../auth/drive.appdata`；
   **发布状态改为“正式版（In production）”**——“测试”状态下的授权 7 天就会过期。
   未经 Google 审核的应用登录时会提示“Google 尚未验证此应用”，点“高级 → 继续”即可（自用不影响）。
4. **凭据 → 创建凭据 → OAuth 客户端 ID**，应用类型选 **桌面应用**，得到客户端 ID 和客户端密钥。
5. 填进 Apark：桌面版点 **设置**；或命令行：

   ```sh
   apark config set google_client_id     xxxx.apps.googleusercontent.com
   apark config set google_client_secret GOCSPX-xxxx
   apark login
   ```

只想用 Gmail 收发信但不想配 OAuth：在 Google 账号里开两步验证后创建“应用专用密码”，用 `apark add imap --email you@gmail.com` 添加即可。

### 不同步

直接添加邮箱就能用：`apark add imap --email me@qq.com`（QQ/163/iCloud 等用“授权码/应用专用密码”）。

**Microsoft 账号（可选）**：在 [Azure 门户](https://portal.azure.com/) → 应用注册 → 新注册，账户类型选“任何组织目录中的帐户和个人 Microsoft 帐户”，平台选“移动和桌面应用程序”，重定向 URI 填 `http://127.0.0.1`，然后 `apark config set microsoft_client_id <应用程序ID>`。

**无浏览器的服务器**：`apark login --manual`，在任意设备打开输出的链接，授权后浏览器会跳到一个打不开的 `http://127.0.0.1:…` 地址，把这个完整地址粘贴回终端即可。

## CLI（给人和 AI agent）

```sh
apark guide                      # 给 agent 的完整命令说明
apark list --unread --json       # 所有账号未读邮件
apark search 发票 2026 --json
apark read 42 --json             # {"message": {...}, "body": {"text","html","attachments"}}
apark reply 42 --body "收到，周五前处理"
apark send --from me@gmail.com --to a@b.com -s "周报" --body-file report.md --attach report.pdf
echo '{"from":"me@gmail.com","to":["a@b.com"],"subject":"hi","body":"..."}' | apark send --stdin-json
apark archive 42 43 44
apark daemon --interval 120      # 无头常驻同步
apark watch --inbox-only         # 持续同步，每封新邮件输出一行 JSON，agent 可以直接订阅
```

所有命令：成功退出码 0；失败退出码 1，`--json` 模式下输出 `{"ok": false, "error": "..."}`。读取类命令只查本地缓存，毫秒级返回；`apark sync` 或常驻的 `apark daemon` 负责刷新。

systemd 无头部署示例：

```ini
[Unit]
Description=Apark mail sync
After=network-online.target

[Service]
ExecStart=/usr/local/bin/apark daemon
Environment=APARK_HOME=/var/lib/apark
Restart=always

[Install]
WantedBy=multi-user.target
```

## 数据与安全

- 数据目录：macOS `~/Library/Application Support/Apark`，Windows `%APPDATA%\Apark`，Linux `~/.local/share/Apark`；可用 `APARK_HOME` 覆盖。
- `accounts.json`（凭据）权限为 0600；邮件缓存在 `apark.db`（SQLite）。
- 同步出去的账号列表用 Argon2 + ChaCha20-Poly1305 加密（自建服务器、WebDAV、同步文件夹一律加密；Google 方式设置 `sync_passphrase` 后加密）。
- 自建邮件服务器使用私有 CA 时，可设置 `APARK_EXTRA_CA=/path/ca.pem`。

## 为什么快

- 界面只读本地 SQLite（WAL，读写分离连接），从不在 UI 线程等网络。
- 同步先拉邮件头（列表立刻出现），再在后台预取较小的正文，打开邮件即时显示。
- 已读、星标、归档、删除先在本地生效，再异步同步到服务器。
- 列表只绘制可见行；无动画、无 Web 引擎，常驻内存很小。

HTML 邮件：macOS 用 WebKit、Windows 用 WebView2 直接渲染（禁用脚本，链接在浏览器打开）；Linux 显示纯文本版本，可一键在浏览器查看原始排版。

## 构建

```sh
cargo test                                   # 内核 + CLI 测试
cargo build --release -p apark-cli           # CLI：target/release/apark
cargo build --release -p apark-gtk           # Linux 桌面版（需要 libgtk-4-dev libadwaita-1-dev）
scripts/e2e-greenmail.sh                     # 端到端测试（Docker 里起一个 GreenMail 邮件服务器）
```

macOS / Windows 桌面版的构建步骤见 `.github/workflows/build.yml`：先编译 `apark-ffi`（Rust 内核的 C 接口），再用 `swift build` / `dotnet publish` 编译原生界面。打 `v*` 标签后，GitHub Actions 会为全部平台和架构构建并发布 Release。

## 结构

```
crates/core      内核：账号、OAuth、IMAP 同步、SMTP、SQLite 缓存、云端账号同步、JSON RPC
crates/cli       apark 命令（lib + bin，桌面版复用）
crates/ffi       C 接口：apark_call(json) → json，给 Swift / C# 调用
apps/macos       SwiftUI 界面
apps/windows     WinUI 3 界面
apps/linux       GTK4 + libadwaita 界面
assets/          图标
```

## License

MIT
