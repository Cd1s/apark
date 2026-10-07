using Microsoft.UI.Composition.SystemBackdrops;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Windows.ApplicationModel.DataTransfer;
using Windows.Storage;
using Windows.Storage.Pickers;

namespace Apark;

/// Compose window, built in code: From / To / Cc / Subject, body, attachments (picker or drag-drop).
public sealed class ComposeWindow : Window
{
    private readonly ComboBox _from = new() { HorizontalAlignment = HorizontalAlignment.Stretch };
    private readonly TextBox _to = new() { PlaceholderText = "收件人（多个用逗号分隔）" };
    private readonly TextBox _cc = new() { PlaceholderText = "抄送" };
    private readonly TextBox _subject = new() { PlaceholderText = "主题" };
    private readonly TextBox _body = new() { AcceptsReturn = true, TextWrapping = TextWrapping.Wrap, VerticalAlignment = VerticalAlignment.Stretch };
    private readonly StackPanel _files = new() { Orientation = Orientation.Horizontal, Spacing = 8 };
    private readonly InfoBar _error = new() { Severity = InfoBarSeverity.Error };
    private readonly Button _send = new() { Content = "发送", Style = (Style)Application.Current.Resources["AccentButtonStyle"], MinWidth = 96 };
    private readonly List<string> _attachments;
    private readonly Draft _draft;

    public ComposeWindow(List<string> accounts, string from, Draft draft)
    {
        _draft = draft;
        _attachments = new List<string>(draft.Attachments);
        Title = string.IsNullOrEmpty(draft.Subject) ? "新邮件" : draft.Subject;
        SystemBackdrop = new MicaBackdrop();
        AppWindow.Resize(new Windows.Graphics.SizeInt32(720, 640));
        AppWindow.SetIcon(Path.Combine(AppContext.BaseDirectory, "Assets", "icon.ico"));

        foreach (var a in accounts) _from.Items.Add(a);
        _from.SelectedItem = from;
        _to.Text = string.Join(", ", draft.To);
        _cc.Text = string.Join(", ", draft.Cc);
        _subject.Text = draft.Subject;
        _body.Text = draft.Body;
        _subject.TextChanged += (_, _) => Title = string.IsNullOrEmpty(_subject.Text) ? "新邮件" : _subject.Text;

        var attach = new Button { Content = new SymbolIcon(Symbol.Attach) };
        ToolTipService.SetToolTip(attach, "添加附件（也可以直接拖进来）");
        attach.Click += async (_, _) => await PickFilesAsync();
        _send.Click += async (_, _) => await SendAsync();

        var fields = new Grid { ColumnSpacing = 12, RowSpacing = 8 };
        fields.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        fields.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        var labels = new[] { "发件人", "收件人", "抄送", "主题" };
        var inputs = new FrameworkElement[] { _from, _to, _cc, _subject };
        for (var i = 0; i < labels.Length; i++)
        {
            fields.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
            var label = new TextBlock { Text = labels[i], VerticalAlignment = VerticalAlignment.Center, Opacity = 0.7 };
            Grid.SetRow(label, i);
            Grid.SetRow(inputs[i], i);
            Grid.SetColumn(inputs[i], 1);
            fields.Children.Add(label);
            fields.Children.Add(inputs[i]);
        }

        var toolbar = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8, HorizontalAlignment = HorizontalAlignment.Right };
        toolbar.Children.Add(attach);
        toolbar.Children.Add(_send);

        var root = new Grid { Padding = new Thickness(20, 40, 20, 20), RowSpacing = 12, AllowDrop = true };
        root.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        root.RowDefinitions.Add(new RowDefinition { Height = new GridLength(1, GridUnitType.Star) });
        root.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        root.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        root.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        Grid.SetRow(_body, 1);
        Grid.SetRow(_files, 2);
        Grid.SetRow(_error, 3);
        Grid.SetRow(toolbar, 4);
        root.Children.Add(fields);
        root.Children.Add(_body);
        root.Children.Add(_files);
        root.Children.Add(_error);
        root.Children.Add(toolbar);
        root.DragOver += (_, e) => e.AcceptedOperation = DataPackageOperation.Copy;
        root.Drop += async (_, e) =>
        {
            if (!e.DataView.Contains(StandardDataFormats.StorageItems)) return;
            var deferral = e.GetDeferral();
            foreach (var item in await e.DataView.GetStorageItemsAsync())
                if (item is StorageFile f) _attachments.Add(f.Path);
            deferral.Complete();
            ShowFiles();
        };
        Content = root;
        ShowFiles();
        if (Environment.GetEnvironmentVariable("APARK_SNAPSHOT") is { } dir)
            _ = Snapshot.CaptureLaterAsync(this, root, Path.Combine(dir, "compose.png"));
    }

    private void ShowFiles()
    {
        _files.Children.Clear();
        foreach (var path in _attachments.ToList())
        {
            var chip = new Button { Content = $"📎 {Path.GetFileName(path)}  ✕" };
            chip.Click += (_, _) => { _attachments.Remove(path); ShowFiles(); };
            _files.Children.Add(chip);
        }
    }

    private async Task PickFilesAsync()
    {
        var picker = new FileOpenPicker();
        WinRT.Interop.InitializeWithWindow.Initialize(picker, WinRT.Interop.WindowNative.GetWindowHandle(this));
        picker.FileTypeFilter.Add("*");
        foreach (var f in await picker.PickMultipleFilesAsync()) _attachments.Add(f.Path);
        ShowFiles();
    }

    private static List<string> Split(string s) =>
        s.Split(',', ';').Select(x => x.Trim()).Where(x => x.Length > 0).ToList();

    private async Task SendAsync()
    {
        var to = Split(_to.Text);
        var cc = Split(_cc.Text);
        if (to.Count + cc.Count == 0)
        {
            _error.Message = "请填写收件人";
            _error.IsOpen = true;
            return;
        }
        _send.IsEnabled = false;
        _send.Content = "发送中…";
        try
        {
            await Core.Run("send", new
            {
                from = _from.SelectedItem as string,
                to,
                cc,
                subject = _subject.Text,
                body = _body.Text,
                in_reply_to = _draft.InReplyTo,
                references = _draft.References,
                attachments = _attachments,
            });
            Close();
        }
        catch (Exception e)
        {
            _error.Message = e.Message;
            _error.IsOpen = true;
            _send.IsEnabled = true;
            _send.Content = "发送";
        }
    }
}
