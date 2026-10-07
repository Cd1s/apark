using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Media.Imaging;
using Windows.Graphics.Imaging;
using System.Runtime.InteropServices.WindowsRuntime;

namespace Apark;

/// CI/dev helper: with APARK_SNAPSHOT=<dir>, render windows to PNG and quit.
public static class Snapshot
{
    public static async Task RunAsync(MainWindow main, string dir)
    {
        Directory.CreateDirectory(dir);
        await Task.Delay(2500);
        main.SelectFirst();
        await Task.Delay(2000);
        await CaptureAsync((FrameworkElement)main.Content, Path.Combine(dir, "main.png"));
        main.Compose(null, new Draft { To = ["sarah@example.org"], Subject = "周末计划", Body = "嗨 Sarah，\n\n周六一起去看展吗？\n" });
        await Task.Delay(4000);
        Application.Current.Exit();
    }

    public static async Task CaptureLaterAsync(Window w, FrameworkElement root, string path)
    {
        await Task.Delay(2000);
        await CaptureAsync(root, path);
    }

    public static async Task CaptureAsync(FrameworkElement element, string path)
    {
        var rtb = new RenderTargetBitmap();
        await rtb.RenderAsync(element);
        var pixels = await rtb.GetPixelsAsync();
        using var file = File.Create(path);
        var encoder = await BitmapEncoder.CreateAsync(BitmapEncoder.PngEncoderId, file.AsRandomAccessStream());
        encoder.SetPixelData(BitmapPixelFormat.Bgra8, BitmapAlphaMode.Premultiplied,
            (uint)rtb.PixelWidth, (uint)rtb.PixelHeight, 96, 96, pixels.ToArray());
        await encoder.FlushAsync();
    }
}
