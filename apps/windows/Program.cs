using System.Diagnostics;
using System.Runtime.InteropServices;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;

namespace Apark;

public static class Program
{
    [DllImport("kernel32.dll")]
    private static extern bool AttachConsole(int pid);

    [STAThread]
    private static int Main(string[] args)
    {
        // With arguments the app behaves as the `apark` CLI (bundled as apark-cli.exe).
        if (args.Length > 0) return ForwardToCli(args);

        AppDomain.CurrentDomain.UnhandledException += (_, e) => CrashLog(e.ExceptionObject);
        try
        {
            WinRT.ComWrappersSupport.InitializeComWrappers();
            Application.Start(_ =>
            {
                SynchronizationContext.SetSynchronizationContext(
                    new DispatcherQueueSynchronizationContext(DispatcherQueue.GetForCurrentThread()));
                new App();
            });
            return 0;
        }
        catch (Exception e)
        {
            CrashLog(e);
            return 1;
        }
    }

    /// Last-resort diagnostics: %TEMP%\apark-crash.log
    public static void CrashLog(object error)
    {
        try { File.AppendAllText(Path.Combine(Path.GetTempPath(), "apark-crash.log"), $"{DateTime.Now:O} {error}\n\n"); }
        catch { }
    }

    private static int ForwardToCli(string[] args)
    {
        AttachConsole(-1);
        var psi = new ProcessStartInfo(Path.Combine(AppContext.BaseDirectory, "apark-cli.exe")) { UseShellExecute = false };
        foreach (var a in args) psi.ArgumentList.Add(a);
        using var p = Process.Start(psi);
        if (p == null) return 1;
        p.WaitForExit();
        return p.ExitCode;
    }
}
