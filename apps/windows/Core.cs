using System.Runtime.InteropServices;
using System.Text;
using System.Text.Json;

namespace Apark;

public sealed class CoreException(string message) : Exception(message);

/// Bridge to the Rust core (apark_ffi.dll): one JSON call, run off the UI thread.
public static class Core
{
    [DllImport("apark_ffi.dll", CallingConvention = CallingConvention.Cdecl)]
    private static extern IntPtr apark_call(byte[] request);

    [DllImport("apark_ffi.dll", CallingConvention = CallingConvention.Cdecl)]
    private static extern void apark_free(IntPtr s);

    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate void EventCallback(IntPtr json);

    [DllImport("apark_ffi.dll", CallingConvention = CallingConvention.Cdecl)]
    private static extern void apark_set_event_callback(EventCallback cb);

    public static readonly JsonSerializerOptions Json = new()
    {
        PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower,
        PropertyNameCaseInsensitive = true,
    };

    /// Raised on a background thread with each core event ({"type": ...}).
    public static event Action<JsonElement>? Event;

    private static EventCallback? _callback; // keep the delegate alive

    public static void Listen()
    {
        _callback = ptr =>
        {
            var text = Marshal.PtrToStringUTF8(ptr);
            if (text == null) return;
            using var doc = JsonDocument.Parse(text);
            Event?.Invoke(doc.RootElement.Clone());
        };
        apark_set_event_callback(_callback);
    }

    private static JsonElement Raw(string method, object? args)
    {
        var request = JsonSerializer.Serialize(new { method, @params = args ?? new { } }, Json);
        var ptr = apark_call(Encoding.UTF8.GetBytes(request + "\0"));
        string reply;
        try { reply = Marshal.PtrToStringUTF8(ptr) ?? "{\"ok\":false,\"error\":\"no reply\"}"; }
        finally { apark_free(ptr); }
        using var doc = JsonDocument.Parse(reply);
        var root = doc.RootElement;
        if (!root.GetProperty("ok").GetBoolean())
            throw new CoreException(root.TryGetProperty("error", out var e) ? e.GetString() ?? "未知错误" : "未知错误");
        return root.TryGetProperty("result", out var r) ? r.Clone() : default;
    }

    public static Task<T> Call<T>(string method, object? args = null) => Task.Run(() =>
    {
        var r = Raw(method, args);
        return r.Deserialize<T>(Json) ?? throw new CoreException("空结果");
    });

    public static Task<T?> CallOptional<T>(string method, object? args = null) where T : class => Task.Run(() =>
    {
        var r = Raw(method, args);
        return r.ValueKind is JsonValueKind.Null or JsonValueKind.Undefined ? null : r.Deserialize<T>(Json);
    });

    public static Task Run(string method, object? args = null) => Task.Run(() => { Raw(method, args); });
}
