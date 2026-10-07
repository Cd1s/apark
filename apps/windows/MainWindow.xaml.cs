using System.Collections.ObjectModel;
using System.Text.Json;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Windows.System;

namespace Apark;

public sealed partial class MainWindow : Window
{
    private readonly ObservableCollection<Message> _messages = new();
    private List<Account> _accounts = new();
    private Info? _info;
    private object _nav = "inbox";
    private MailBody? _body;
    private bool _syncing;
    private DispatcherTimer? _searchTimer;
    private readonly bool _noSync = Environment.GetEnvironmentVariable("APARK_NO_SYNC") != null;

    public MainWindow()
    {
        InitializeComponent();
        SystemBackdrop = new MicaBackdrop();
        ExtendsContentIntoTitleBar = true;
        SetTitleBar(TitleBar);
        AppWindow.SetIcon(Path.Combine(AppContext.BaseDirectory, "Assets", "icon.ico"));
        AppWindow.Resize(new Windows.Graphics.SizeInt32(1280, 820));
        List.ItemsSource = _messages;
        AddShortcuts();
        Core.Listen();
        Core.Event += e => DispatcherQueue.TryEnqueue(() => OnCoreEvent(e));
        _ = BootstrapAsync();
    }

    private Message? Selected => List.SelectedItem as Message;

    // ---- lifecycle -------------------------------------------------------

    private async Task BootstrapAsync()
    {
        try { _info = await Core.Call<Info>("info"); }
        catch (Exception e) { ShowError(e.Message); }
        await RefreshAccountsAsync();
        await ReloadAsync();
        if (_accounts.Count > 0 && !_noSync)
        {
            SetSyncing(true);
            _ = Core.Run("start_auto_sync");
        }
        if (Environment.GetEnvironmentVariable("APARK_SNAPSHOT") is { } dir) _ = Snapshot.RunAsync(this, dir);
    }

    private void OnCoreEvent(JsonElement e)
    {
        switch (e.GetProperty("type").GetString())
        {
            case "synced":
                SetSyncing(false);
                SyncStatus.Text = $"已同步 {DateTime.Now:HH:mm}";
                if (e.TryGetProperty("results", out var results))
                    foreach (var r in results.EnumerateArray())
                        if (!r.GetProperty("ok").GetBoolean())
                        {
                            ShowError($"{r.GetProperty("account").GetString()}：{r.GetProperty("error").GetString()}");
                            break;
                        }
                _ = RefreshAccountsAsync().ContinueWith(_ => DispatcherQueue.TryEnqueue(() => _ = ReloadAsync()));
                break;
            case "login_url":
                if (e.TryGetProperty("url", out var url) && Uri.TryCreate(url.GetString(), UriKind.Absolute, out var uri))
                {
                    LoginLink.NavigateUri = uri;
                    LoginLink.Visibility = Visibility.Visible;
                }
                break;
        }
    }

    private void SetSyncing(bool on)
    {
        _syncing = on;
        SyncButton.IsEnabled = !on;
        if (on) SyncStatus.Text = "正在同步…";
    }

    private void ShowError(string message)
    {
        Toast.Message = message;
        Toast.IsOpen = true;
    }

    // ---- accounts & navigation -------------------------------------------

    private async Task RefreshAccountsAsync()
    {
        try
        {
            _info = await Core.Call<Info>("info");
            _accounts = await Core.Call<List<Account>>("accounts");
        }
        catch (Exception e) { ShowError(e.Message); }
        var empty = _accounts.Count == 0;
        WelcomeView.Visibility = empty ? Visibility.Visible : Visibility.Collapsed;
        Nav.Visibility = empty ? Visibility.Collapsed : Visibility.Visible;
        ClientPanel.Visibility = _info?.GoogleReady == true ? Visibility.Collapsed : Visibility.Visible;
        GoogleButton.IsEnabled = _info?.GoogleReady == true;
        await BuildNavAsync();
    }

    private static NavigationViewItem Item(string text, string glyph, object tag, int badge = 0)
    {
        var item = new NavigationViewItem { Content = text, Icon = new FontIcon { Glyph = glyph }, Tag = tag };
        if (badge > 0) item.InfoBadge = new InfoBadge { Value = badge };
        return item;
    }

    private async Task BuildNavAsync()
    {
        Dictionary<string, int> unread;
        try { unread = await Core.Call<Dictionary<string, int>>("unread_counts"); }
        catch { unread = new(); }
        int Count(string k) => unread.TryGetValue(k, out var n) ? n : 0;

        var items = new List<object>
        {
            new NavigationViewItemHeader { Content = "智能收件箱" },
            Item("收件箱", "", "inbox", unread.Values.Sum()),
            Item("个人", "", "people", Count("people")),
            Item("通知", "", "notification", Count("notification")),
            Item("订阅", "", "newsletter", Count("newsletter")),
            new NavigationViewItemHeader { Content = "快速筛选" },
            Item("未读", "", "unread"),
            Item("星标", "", "flagged"),
        };
        foreach (var a in _accounts)
        {
            items.Add(new NavigationViewItemSeparator());
            var parent = new NavigationViewItem
            {
                Content = a.Email,
                Icon = new FontIcon { Glyph = "", Foreground = Palette.Brush(a.Email) },
                SelectsOnInvoked = false,
            };
            List<Folder> folders;
            try { folders = await Core.Call<List<Folder>>("folders", new { email = a.Email }); }
            catch { folders = new(); }
            foreach (var f in folders) parent.MenuItems.Add(Item(f.Title, f.Glyph, (a.Email, f.Name)));
            var menu = new MenuFlyout();
            var remove = new MenuFlyoutItem { Text = "删除账号", Icon = new FontIcon { Glyph = "" } };
            var email = a.Email;
            remove.Click += async (_, _) =>
            {
                try { await Core.Run("remove_account", new { email }); } catch (Exception e) { ShowError(e.Message); }
                await RefreshAccountsAsync();
                await ReloadAsync();
            };
            menu.Items.Add(remove);
            parent.ContextFlyout = menu;
            items.Add(parent);
        }
        Nav.MenuItemsSource = items;
        var current = items.OfType<NavigationViewItem>().FirstOrDefault(i => Equals(i.Tag, _nav));
        if (current != null) Nav.SelectedItem = current;
    }

    private async void Nav_SelectionChanged(NavigationView sender, NavigationViewSelectionChangedEventArgs args)
    {
        if (args.IsSettingsSelected)
        {
            await new SettingsDialog(this).ShowAsync(Root.XamlRoot);
            return;
        }
        if (args.SelectedItem is not NavigationViewItem { Tag: { } tag } || Equals(tag, _nav)) return;
        _nav = tag;
        if (tag is ValueTuple<string, string> folder && !folder.Item2.Equals("INBOX", StringComparison.OrdinalIgnoreCase))
        {
            _ = Core.Run("sync_account", new { email = folder.Item1, folders = new[] { folder.Item2 } })
                .ContinueWith(_ => DispatcherQueue.TryEnqueue(() => _ = ReloadAsync()));
        }
        await ReloadAsync();
    }

    // ---- list -----------------------------------------------------------

    private object Query()
    {
        var search = string.IsNullOrWhiteSpace(Search.Text) ? null : Search.Text.Trim();
        return _nav switch
        {
            "people" or "notification" or "newsletter" => new { category = (string)_nav, search, limit = 5000 },
            "unread" => new { unread = true, folder = "INBOX", search, limit = 5000 },
            "flagged" => new { flagged = true, folder = "INBOX", search, limit = 5000 },
            ValueTuple<string, string> f => new { account = f.Item1, folder = f.Item2, search, limit = 5000 } as object,
            _ => new { search, limit = 5000 },
        };
    }

    private string Title() => _nav switch
    {
        "people" => "个人",
        "notification" => "通知",
        "newsletter" => "订阅",
        "unread" => "未读",
        "flagged" => "星标",
        ValueTuple<string, string> f => new Folder(f.Item2, "").Title,
        _ => "收件箱",
    };

    private async Task ReloadAsync()
    {
        List<Message> rows;
        try { rows = await Core.Call<List<Message>>("list", Query()); }
        catch (Exception e) { ShowError(e.Message); return; }
        var selectedId = Selected?.Id;
        _messages.Clear();
        foreach (var m in rows) _messages.Add(m);
        ListTitle.Text = Title();
        var unread = rows.Count(m => !m.Seen);
        ListSubtitle.Text = unread > 0 ? $"{unread} 封未读" : "";
        ListEmpty.Visibility = rows.Count == 0 ? Visibility.Visible : Visibility.Collapsed;
        ListEmptyText.Text = string.IsNullOrWhiteSpace(Search.Text) ? "没有邮件" : "没有找到";
        if (selectedId is { } id && _messages.FirstOrDefault(m => m.Id == id) is { } again) List.SelectedItem = again;
    }

    private void Search_TextChanged(AutoSuggestBox sender, AutoSuggestBoxTextChangedEventArgs args)
    {
        _searchTimer ??= new DispatcherTimer { Interval = TimeSpan.FromMilliseconds(150) };
        _searchTimer.Stop();
        _searchTimer.Tick -= SearchTick;
        _searchTimer.Tick += SearchTick;
        _searchTimer.Start();
    }

    private void SearchTick(object? sender, object e)
    {
        _searchTimer?.Stop();
        _ = ReloadAsync();
    }

    private void Replace(Message updated)
    {
        var i = _messages.IndexOf(_messages.First(m => m.Id == updated.Id));
        var wasSelected = List.SelectedIndex == i;
        _messages[i] = updated;
        if (wasSelected) List.SelectedIndex = i;
    }

    // ---- reader ---------------------------------------------------------

    private async void List_SelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        var m = Selected;
        ReaderEmpty.Visibility = m == null ? Visibility.Visible : Visibility.Collapsed;
        Reader.Visibility = m == null ? Visibility.Collapsed : Visibility.Visible;
        if (m == null) return;
        ShowHeader(m);
        if (_body != null && e.RemovedItems.OfType<Message>().Any(r => r.Id == m.Id)) return; // same message re-selected after update
        _body = null;
        ReaderBody.Text = "";
        Html.Visibility = Visibility.Collapsed;
        TextScroll.Visibility = Visibility.Visible;
        AttachmentList.Children.Clear();
        BodyLoading.IsActive = true;
        try
        {
            var body = await Core.CallOptional<MailBody>("cached_body", new { id = m.Id }) ?? await Core.Call<MailBody>("body", new { id = m.Id });
            if (Selected?.Id != m.Id) return;
            ShowBody(m, body);
        }
        catch (Exception ex) { ReaderBody.Text = ex.Message; }
        finally { BodyLoading.IsActive = false; }

        if (!m.Seen)
        {
            Replace(m with { Seen = true });
            Perform("set_seen", new { id = m.Id, seen = true });
        }
    }

    private void ShowHeader(Message m)
    {
        ReaderSubject.Text = m.DisplaySubject;
        ReaderAvatar.DisplayName = m.Sender;
        ReaderSender.Text = m.Sender;
        ReaderAddress.Text = m.FromAddr;
        ReaderTo.Text = string.IsNullOrEmpty(m.Cc) ? $"收件人：{m.To}" : $"收件人：{m.To}    抄送：{m.Cc}";
        ReaderDate.Text = m.WhenLong;
        FlagButton.Label = m.Flagged ? "取消星标" : "星标";
        SeenButton.Label = m.Seen ? "未读" : "已读";
    }

    private void ShowBody(Message m, MailBody body)
    {
        _body = body;
        if (body.Html is { } html)
        {
            TextScroll.Visibility = Visibility.Collapsed;
            Html.Visibility = Visibility.Visible;
            _ = ShowHtmlAsync(html);
        }
        else
        {
            ReaderBody.Text = body.Text.TrimEnd();
        }
        AttachmentList.Children.Clear();
        for (var i = 0; i < body.Attachments.Count; i++)
        {
            var a = body.Attachments[i];
            var index = i;
            var b = new Button
            {
                Content = new StackPanel
                {
                    Orientation = Orientation.Horizontal,
                    Spacing = 6,
                    Children = { new FontIcon { Glyph = "", FontSize = 14 }, new TextBlock { Text = $"{a.Name}  {Fmt.Size(a.Size)}" } },
                },
            };
            ToolTipService.SetToolTip(b, "保存到“下载”");
            b.Click += async (_, _) =>
            {
                try
                {
                    var paths = await Core.Call<List<string>>("save_attachments", new { id = m.Id, index });
                    if (paths.FirstOrDefault() is { } p) System.Diagnostics.Process.Start("explorer.exe", $"/select,\"{p}\"");
                }
                catch (Exception ex) { ShowError(ex.Message); }
            };
            AttachmentList.Children.Add(b);
        }
    }

    private async Task ShowHtmlAsync(string html)
    {
        await Html.EnsureCoreWebView2Async();
        Html.CoreWebView2.Settings.IsScriptEnabled = false;
        Html.CoreWebView2.NewWindowRequested -= OpenExternally;
        Html.CoreWebView2.NewWindowRequested += OpenExternally;
        Html.CoreWebView2.NavigationStarting -= BlockNavigation;
        Html.CoreWebView2.NavigationStarting += BlockNavigation;
        Html.NavigateToString($"<!doctype html><meta charset=\"utf-8\"><style>body{{font:14.5px 'Segoe UI','Microsoft YaHei',sans-serif;margin:20px 28px}}img{{max-width:100%;height:auto}}</style>{html}");
    }

    private static void OpenExternally(Microsoft.Web.WebView2.Core.CoreWebView2 s, Microsoft.Web.WebView2.Core.CoreWebView2NewWindowRequestedEventArgs e)
    {
        e.Handled = true;
        _ = Launcher.LaunchUriAsync(new Uri(e.Uri));
    }

    private static void BlockNavigation(Microsoft.Web.WebView2.Core.CoreWebView2 s, Microsoft.Web.WebView2.Core.CoreWebView2NavigationStartingEventArgs e)
    {
        if (e.Uri.StartsWith("http", StringComparison.OrdinalIgnoreCase) || e.Uri.StartsWith("mailto:", StringComparison.OrdinalIgnoreCase))
        {
            e.Cancel = true;
            _ = Launcher.LaunchUriAsync(new Uri(e.Uri));
        }
    }

    // ---- actions --------------------------------------------------------

    private void Perform(string method, object args, bool reload = false)
    {
        _ = Core.Run(method, args).ContinueWith(t => DispatcherQueue.TryEnqueue(() =>
        {
            if (t.Exception?.InnerException is { } ex) ShowError(ex.Message);
            if (reload || t.IsFaulted) _ = ReloadAsync();
            _ = BuildNavAsync();
        }));
    }

    private void TakeOut(string method)
    {
        if (Selected is not { } m) return;
        var i = _messages.IndexOf(m);
        _messages.RemoveAt(i);
        if (_messages.Count > 0) List.SelectedIndex = Math.Min(i, _messages.Count - 1);
        Perform(method, new { id = m.Id });
    }

    private void Archive_Click(object sender, RoutedEventArgs e) => TakeOut("archive");
    private void Trash_Click(object sender, RoutedEventArgs e) => TakeOut("trash");

    private void Flag_Click(object sender, RoutedEventArgs e)
    {
        if (Selected is not { } m) return;
        Replace(m with { Flagged = !m.Flagged });
        ShowHeader(m with { Flagged = !m.Flagged });
        Perform("set_flagged", new { id = m.Id, flagged = !m.Flagged });
    }

    private void Seen_Click(object sender, RoutedEventArgs e)
    {
        if (Selected is not { } m) return;
        Replace(m with { Seen = !m.Seen });
        ShowHeader(m with { Seen = !m.Seen });
        Perform("set_seen", new { id = m.Id, seen = !m.Seen });
    }

    private void Sync_Click(object sender, RoutedEventArgs e) => SyncNow();

    private void SyncNow()
    {
        if (_syncing || _accounts.Count == 0 || _noSync) return;
        SetSyncing(true);
        _ = Core.Run("sync_all");
    }

    private void Compose_Click(object sender, RoutedEventArgs e) => Compose(null, new Draft());

    public void Compose(string? from, Draft draft)
    {
        if (_accounts.Count == 0) return;
        new ComposeWindow(_accounts.Select(a => a.Email).ToList(), from ?? _accounts[0].Email, draft).Activate();
    }

    private async void Reply(bool all)
    {
        if (Selected is not { } m) return;
        try
        {
            var r = await Core.Call<DraftReply>("reply_draft", new { id = m.Id, all });
            Compose(r.From, r.Draft);
        }
        catch (Exception e) { ShowError(e.Message); }
    }

    private void Reply_Click(object sender, RoutedEventArgs e) => Reply(false);
    private void ReplyAll_Click(object sender, RoutedEventArgs e) => Reply(true);

    private async void Forward_Click(object sender, RoutedEventArgs e)
    {
        if (Selected is not { } m) return;
        try
        {
            var r = await Core.Call<DraftReply>("forward_draft", new { id = m.Id });
            Compose(r.From, r.Draft);
        }
        catch (Exception ex) { ShowError(ex.Message); }
    }

    private void AddShortcuts()
    {
        void Add(VirtualKey key, VirtualKeyModifiers mods, Action action)
        {
            var acc = new KeyboardAccelerator { Key = key, Modifiers = mods };
            acc.Invoked += (_, a) => { a.Handled = true; action(); };
            Root.KeyboardAccelerators.Add(acc);
        }
        Add(VirtualKey.N, VirtualKeyModifiers.Control, () => Compose(null, new Draft()));
        Add(VirtualKey.R, VirtualKeyModifiers.Control, () => Reply(false));
        Add(VirtualKey.R, VirtualKeyModifiers.Control | VirtualKeyModifiers.Shift, () => Reply(true));
        Add(VirtualKey.E, VirtualKeyModifiers.Control, () => TakeOut("archive"));
        Add(VirtualKey.F5, VirtualKeyModifiers.None, SyncNow);
        Add(VirtualKey.F, VirtualKeyModifiers.Control, () => Search.Focus(FocusState.Keyboard));
        List.KeyDown += (_, e) =>
        {
            if (e.Key == VirtualKey.Delete) { TakeOut("trash"); e.Handled = true; }
        };
    }

    // ---- accounts ---------------------------------------------------------

    private async void Login_Click(object sender, RoutedEventArgs e)
    {
        BusyPanel.Visibility = Visibility.Visible;
        BusyText.Text = "请在浏览器中完成 Google 授权…";
        WelcomeError.Text = "";
        GoogleButton.IsEnabled = false;
        try
        {
            await Core.Run("login_master");
            await AfterAccountsChangedAsync();
        }
        catch (Exception ex) { WelcomeError.Text = ex.Message; }
        BusyPanel.Visibility = Visibility.Collapsed;
        LoginLink.Visibility = Visibility.Collapsed;
        GoogleButton.IsEnabled = _info?.GoogleReady == true;
    }

    public async Task AfterAccountsChangedAsync()
    {
        await RefreshAccountsAsync();
        await ReloadAsync();
        if (!_noSync)
        {
            _ = Core.Run("start_auto_sync");
            SyncNow();
        }
    }

    private async void SyncSetup_Click(object sender, RoutedEventArgs e) =>
        await new SyncSetupDialog(this).ShowAsync(Root.XamlRoot);

    private async void AddAccount_Click(object sender, RoutedEventArgs e) =>
        await new AddAccountDialog(this, _info).ShowAsync(Root.XamlRoot);

    private async void SaveClient_Click(object sender, RoutedEventArgs e)
    {
        try
        {
            var cfg = await Core.Call<Config>("config");
            cfg.GoogleClientId = ClientIdBox.Text.Trim();
            cfg.GoogleClientSecret = string.IsNullOrEmpty(ClientSecretBox.Password) ? null : ClientSecretBox.Password;
            await Core.Run("set_config", cfg);
            await RefreshAccountsAsync();
        }
        catch (Exception ex) { WelcomeError.Text = ex.Message; }
    }

    public void SelectFirst()
    {
        if (_messages.Count > 0) List.SelectedIndex = 0;
    }
}
