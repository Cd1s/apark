using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

namespace Apark;

public sealed class AddAccountDialog
{
    private readonly MainWindow _main;
    private readonly Info? _info;

    public AddAccountDialog(MainWindow main, Info? info) { _main = main; _info = info; }

    public async Task ShowAsync(XamlRoot root)
    {
        var kind = new RadioButtons { Header = "类型", SelectedIndex = 0, MaxColumns = 3 };
        kind.Items.Add("Google");
        kind.Items.Add("Microsoft");
        kind.Items.Add("其他邮箱");
        var email = new TextBox { Header = "邮箱", PlaceholderText = "you@example.com" };
        var password = new PasswordBox { Header = "密码", PlaceholderText = "密码或应用专用密码" };
        var name = new TextBox { Header = "名字（可选）" };
        var imap = new TextBox { Header = "IMAP 服务器（留空自动识别）", PlaceholderText = "imap.example.com:993" };
        var smtp = new TextBox { Header = "SMTP 服务器（留空自动识别）", PlaceholderText = "smtp.example.com:465" };
        var imapPanel = new StackPanel { Spacing = 10, Children = { email, password, name, imap, smtp } };
        var hint = new TextBlock { TextWrapping = TextWrapping.Wrap, Opacity = 0.75 };
        var error = new InfoBar { Severity = InfoBarSeverity.Error };
        var busy = new ProgressBar { IsIndeterminate = true, Visibility = Visibility.Collapsed };
        void Update()
        {
            imapPanel.Visibility = kind.SelectedIndex == 2 ? Visibility.Visible : Visibility.Collapsed;
            hint.Visibility = kind.SelectedIndex == 2 ? Visibility.Collapsed : Visibility.Visible;
            var ready = kind.SelectedIndex == 0 ? _info?.GoogleReady : _info?.MicrosoftReady;
            hint.Text = ready == true ? "会在浏览器中打开授权页面。" : "还没有配置 OAuth 客户端，请先在“设置”里填写。";
        }
        kind.SelectionChanged += (_, _) => Update();
        Update();

        var dialog = new ContentDialog
        {
            XamlRoot = root,
            Title = "添加账号",
            PrimaryButtonText = "添加",
            CloseButtonText = "取消",
            DefaultButton = ContentDialogButton.Primary,
            Content = new StackPanel { Spacing = 14, MinWidth = 380, Children = { kind, hint, imapPanel, busy, error } },
        };
        dialog.PrimaryButtonClick += async (d, args) =>
        {
            var deferral = args.GetDeferral();
            busy.Visibility = Visibility.Visible;
            error.IsOpen = false;
            try
            {
                switch (kind.SelectedIndex)
                {
                    case 0: await Core.Run("add_oauth", new { provider = "google" }); break;
                    case 1: await Core.Run("add_oauth", new { provider = "microsoft" }); break;
                    default:
                        await Core.Run("add_password", new { email = email.Text.Trim(), password = password.Password, name = name.Text, imap = imap.Text, smtp = smtp.Text });
                        break;
                }
                await _main.AfterAccountsChangedAsync();
            }
            catch (Exception e)
            {
                error.Message = e.Message;
                error.IsOpen = true;
                args.Cancel = true;
            }
            busy.Visibility = Visibility.Collapsed;
            deferral.Complete();
        };
        await dialog.ShowAsync();
    }
}

public sealed class SettingsDialog
{
    private readonly MainWindow _main;

    public SettingsDialog(MainWindow main) => _main = main;

    public async Task ShowAsync(XamlRoot root)
    {
        Config cfg;
        try { cfg = await Core.Call<Config>("config"); }
        catch { return; }
        var info = await Core.Call<Info>("info");
        var gid = new TextBox { Header = "Google 客户端 ID", Text = cfg.GoogleClientId ?? "" };
        var gsecret = new PasswordBox { Header = "Google 客户端密钥", Password = cfg.GoogleClientSecret ?? "" };
        var mid = new TextBox { Header = "Microsoft 应用程序（客户端）ID", Text = cfg.MicrosoftClientId ?? "" };
        var pass = new PasswordBox { Header = "云同步密码（可选）", Password = cfg.SyncPassphrase ?? "" };
        var interval = new NumberBox { Header = "同步间隔（秒）", Value = cfg.SyncIntervalSecs, Minimum = 30, Maximum = 3600, SpinButtonPlacementMode = NumberBoxSpinButtonPlacementMode.Inline };
        var syncText = info.Sync is { } sync ? $"账号同步：{sync.Title} · {sync.Where}" : "账号同步：未开启";
        var changeSync = new Button { Content = info.Sync == null ? "设置同步…" : "更换同步方式…" };
        var stopSync = new Button { Content = "停止同步", Visibility = info.Sync == null ? Visibility.Collapsed : Visibility.Visible };
        ContentDialog? self = null;
        changeSync.Click += async (_, _) =>
        {
            self?.Hide();
            await new SyncSetupDialog(_main).ShowAsync(root);
        };
        stopSync.Click += async (_, _) =>
        {
            await Core.Run("sync_off");
            stopSync.Visibility = Visibility.Collapsed;
        };
        var syncPanel = new StackPanel
        {
            Spacing = 8,
            Children = { new TextBlock { Text = syncText, TextWrapping = TextWrapping.Wrap }, new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8, Children = { changeSync, stopSync } } },
        };
        var note = new TextBlock
        {
            Text = $"WebDAV 和同步文件夹用同步密码加密账号列表；每台设备要填相同的密码。\n数据目录：{info.DataDir}",
            TextWrapping = TextWrapping.Wrap,
            Opacity = 0.7,
            IsTextSelectionEnabled = true,
        };
        var dialog = new ContentDialog
        {
            XamlRoot = root,
            Title = "设置",
            PrimaryButtonText = "保存",
            CloseButtonText = "取消",
            DefaultButton = ContentDialogButton.Primary,
            Content = new ScrollViewer { Content = new StackPanel { Spacing = 12, MinWidth = 420, Children = { syncPanel, gid, gsecret, mid, interval, pass, note } } },
        };
        self = dialog;
        if (await dialog.ShowAsync() != ContentDialogResult.Primary) return;
        static string? Opt(string s) => string.IsNullOrWhiteSpace(s) ? null : s.Trim();
        cfg.GoogleClientId = Opt(gid.Text);
        cfg.GoogleClientSecret = Opt(gsecret.Password);
        cfg.MicrosoftClientId = Opt(mid.Text);
        cfg.SyncPassphrase = Opt(pass.Password);
        cfg.SyncIntervalSecs = (long)interval.Value;
        await Core.Run("set_config", cfg);
        await _main.AfterAccountsChangedAsync();
    }
}

/// Account-list sync without Google: self-hosted `apark server`, WebDAV, or a synced folder.
public sealed class SyncSetupDialog
{
    private readonly MainWindow _main;

    public SyncSetupDialog(MainWindow main) => _main = main;

    public async Task ShowAsync(XamlRoot root)
    {
        var kinds = new[] { "server", "webdav", "file" };
        var kind = new RadioButtons { Header = "方式", SelectedIndex = 0, MaxColumns = 3 };
        kind.Items.Add("自建服务器");
        kind.Items.Add("WebDAV");
        kind.Items.Add("同步文件夹");
        var url = new TextBox { Header = "服务器地址", PlaceholderText = "https://sync.example.com" };
        var user = new TextBox { Header = "用户名" };
        var password = new PasswordBox { Header = "密码（同时用于加密）" };
        var token = new TextBox { Header = "访问令牌（可选）" };
        var path = new TextBox { Header = "文件路径", PlaceholderText = @"C:\Users\me\Dropbox\Apark\accounts.json" };
        var passphrase = new PasswordBox { Header = "同步密码（用来加密账号列表）" };
        var hint = new TextBlock { TextWrapping = TextWrapping.Wrap, Opacity = 0.7 };
        var error = new InfoBar { Severity = InfoBarSeverity.Error };
        var busy = new ProgressBar { IsIndeterminate = true, Visibility = Visibility.Collapsed };
        void Update()
        {
            var k = kinds[Math.Max(0, kind.SelectedIndex)];
            url.Visibility = user.Visibility = password.Visibility = k == "file" ? Visibility.Collapsed : Visibility.Visible;
            url.Header = k == "webdav" ? "文件地址" : "服务器地址";
            url.PlaceholderText = k == "webdav" ? "https://dav.example.com/apark/accounts.json" : "https://sync.example.com";
            password.Header = k == "server" ? "密码（同时用于加密）" : "WebDAV 密码";
            token.Visibility = k == "server" ? Visibility.Visible : Visibility.Collapsed;
            path.Visibility = k == "file" ? Visibility.Visible : Visibility.Collapsed;
            passphrase.Visibility = k == "server" ? Visibility.Collapsed : Visibility.Visible;
            hint.Text = k switch
            {
                "server" => "在任意服务器上运行 apark server --token 你的令牌，建议放在 HTTPS 反向代理后面。服务器只保存密文。",
                "webdav" => "坚果云、Nextcloud、群晖等都支持 WebDAV。",
                _ => "放在 OneDrive、Dropbox、Syncthing 等会自动同步的文件夹里。",
            };
        }
        kind.SelectionChanged += (_, _) => Update();
        Update();

        var dialog = new ContentDialog
        {
            XamlRoot = root,
            Title = "账号同步",
            PrimaryButtonText = "开始同步",
            CloseButtonText = "取消",
            DefaultButton = ContentDialogButton.Primary,
            Content = new ScrollViewer
            {
                Content = new StackPanel { Spacing = 12, MinWidth = 420, Children = { kind, url, user, password, token, path, passphrase, hint, busy, error } },
            },
        };
        dialog.PrimaryButtonClick += async (_, args) =>
        {
            var deferral = args.GetDeferral();
            busy.Visibility = Visibility.Visible;
            error.IsOpen = false;
            try
            {
                await Core.Run("sync_setup", new
                {
                    type = kinds[kind.SelectedIndex],
                    url = url.Text.Trim(),
                    user = user.Text.Trim(),
                    password = password.Password,
                    token = token.Text.Trim(),
                    path = path.Text.Trim(),
                    passphrase = passphrase.Password,
                });
                await _main.AfterAccountsChangedAsync();
            }
            catch (Exception e)
            {
                error.Message = e.Message;
                error.IsOpen = true;
                args.Cancel = true;
            }
            busy.Visibility = Visibility.Collapsed;
            deferral.Complete();
        };
        await dialog.ShowAsync();
    }
}
