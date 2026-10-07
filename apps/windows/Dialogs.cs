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
        var note = new TextBlock
        {
            Text = $"账号列表保存在总账号 Google Drive 的隐藏目录；设置同步密码后先加密再上传。\n数据目录：{info.DataDir}",
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
            Content = new ScrollViewer { Content = new StackPanel { Spacing = 12, MinWidth = 420, Children = { gid, gsecret, mid, interval, pass, note } } },
        };
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
