using System.Threading;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;

namespace EfAssistant.App;

/// <summary>
/// Explicit entry point. The XAML compiler would otherwise generate one into
/// App.g.i.cs; DISABLE_XAML_GENERATED_MAIN in the project file suppresses that so
/// startup stays under our control for single-instance handling and tray launch.
/// </summary>
public static class Program
{
    [STAThread]
    public static void Main(string[] args)
    {
        WinRT.ComWrappersSupport.InitializeComWrappers();

        Application.Start(_ignored =>
        {
            var dispatcherQueue = DispatcherQueue.GetForCurrentThread();
            SynchronizationContext.SetSynchronizationContext(
                new DispatcherQueueSynchronizationContext(dispatcherQueue));

            _ = new App();
        });
    }
}
