using System.Text.Json.Serialization;
using Microsoft.UI;
using Microsoft.UI.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Media;
using Windows.UI.Text;

namespace Apark;

public sealed record Info(string Version, string DataDir, bool HasMaster, bool GoogleReady, bool MicrosoftReady);

public sealed record Account(string Email, string Name, string Provider, bool Master, string Imap, string Smtp);

public sealed record Folder(string Name, string Role)
{
    [JsonIgnore]
    public string Title => Name.Equals("INBOX", StringComparison.OrdinalIgnoreCase)
        ? "收件箱"
        : Name.Split('/', '.').Last();

    [JsonIgnore]
    public string Glyph => Role switch
    {
        "inbox" => "",
        "sent" => "",
        "drafts" => "",
        "trash" => "",
        "junk" => "",
        "archive" or "all" => "",
        "flagged" => "",
        _ => "",
    };
}

public sealed record Message
{
    public long Id { get; init; }
    public string Account { get; init; } = "";
    public string Folder { get; init; } = "";
    public string MessageId { get; init; } = "";
    public string Subject { get; init; } = "";
    public string FromName { get; init; } = "";
    public string FromAddr { get; init; } = "";
    public string To { get; init; } = "";
    public string Cc { get; init; } = "";
    public long Date { get; init; }
    public bool Seen { get; init; }
    public bool Flagged { get; init; }
    public string Category { get; init; } = "";
    public string Snippet { get; init; } = "";

    [JsonIgnore] public string Sender => string.IsNullOrEmpty(FromName) ? FromAddr : FromName;
    [JsonIgnore] public string DisplaySubject => string.IsNullOrEmpty(Subject) ? "（无主题）" : Subject;
    [JsonIgnore] public string When => Fmt.Short(Date);
    [JsonIgnore] public string WhenLong => Fmt.Long(Date);
    [JsonIgnore] public FontWeight SenderWeight => Seen ? FontWeights.Normal : FontWeights.SemiBold;
    [JsonIgnore] public FontWeight SubjectWeight => Seen ? FontWeights.Normal : FontWeights.SemiBold;
    [JsonIgnore] public Visibility UnreadVisibility => Seen ? Visibility.Collapsed : Visibility.Visible;
    [JsonIgnore] public Visibility FlagVisibility => Flagged ? Visibility.Visible : Visibility.Collapsed;
    [JsonIgnore] public Visibility SnippetVisibility => string.IsNullOrEmpty(Snippet) ? Visibility.Collapsed : Visibility.Visible;
    [JsonIgnore] public SolidColorBrush AccountBrush => Palette.Brush(Account);
}

public sealed record Attachment(string Name, long Size);

public sealed record MailBody(string Text, string? Html, List<Attachment> Attachments);

public sealed record Draft
{
    public List<string> To { get; init; } = [];
    public List<string> Cc { get; init; } = [];
    public List<string> Bcc { get; init; } = [];
    public string Subject { get; init; } = "";
    public string Body { get; init; } = "";
    public string? InReplyTo { get; init; }
    public string? References { get; init; }
    public List<string> Attachments { get; init; } = [];
}

public sealed record DraftReply(string From, Draft Draft);

public sealed record Config
{
    public string? GoogleClientId { get; set; }
    public string? GoogleClientSecret { get; set; }
    public string? MicrosoftClientId { get; set; }
    public string? SyncPassphrase { get; set; }
    public long SyncIntervalSecs { get; set; }
    public long InitialLimit { get; set; }
    public long PrefetchKb { get; set; }
}

public static class Fmt
{
    public static string Short(long ts)
    {
        var d = DateTimeOffset.FromUnixTimeSeconds(ts).ToLocalTime();
        var now = DateTimeOffset.Now;
        if (d.Date == now.Date) return d.ToString("HH:mm");
        if (d.Date == now.Date.AddDays(-1)) return "昨天";
        return d.Year == now.Year ? $"{d.Month}月{d.Day}日" : d.ToString("yyyy/M/d");
    }

    public static string Long(long ts) => DateTimeOffset.FromUnixTimeSeconds(ts).ToLocalTime().ToString("yyyy年M月d日 HH:mm");

    public static string Size(long n) => n switch
    {
        >= 1 << 20 => $"{n / 1048576.0:0.0} MB",
        >= 1 << 10 => $"{n / 1024.0:0} KB",
        _ => $"{n} B",
    };
}

public static class Palette
{
    private static readonly Dictionary<string, SolidColorBrush> Cache = new();

    /// FNV-1a hue per address, shared with the other Apark front-ends.
    public static SolidColorBrush Brush(string key)
    {
        if (Cache.TryGetValue(key, out var b)) return b;
        uint h = 0x811C9DC5;
        foreach (var c in System.Text.Encoding.UTF8.GetBytes(key)) h = (h ^ c) * 0x01000193;
        var color = FromHsv(h % 3600 / 3600.0, 0.55, 0.85);
        return Cache[key] = new SolidColorBrush(color);
    }

    private static Windows.UI.Color FromHsv(double h, double s, double v)
    {
        var i = (int)(h * 6) % 6;
        var f = h * 6 - Math.Floor(h * 6);
        double p = v * (1 - s), q = v * (1 - f * s), t = v * (1 - (1 - f) * s);
        var (r, g, b) = i switch { 0 => (v, t, p), 1 => (q, v, p), 2 => (p, v, t), 3 => (p, q, v), 4 => (t, p, v), _ => (v, p, q) };
        return ColorHelper.FromArgb(255, (byte)(r * 255), (byte)(g * 255), (byte)(b * 255));
    }
}
