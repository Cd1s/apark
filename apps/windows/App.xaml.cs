using Microsoft.UI.Xaml;

namespace Apark;

public partial class App : Application
{
    public static MainWindow? Main { get; private set; }

    public App() => InitializeComponent();

    protected override void OnLaunched(LaunchActivatedEventArgs args)
    {
        Main = new MainWindow();
        Main.Activate();
    }
}
