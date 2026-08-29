using CommunityToolkit.Mvvm.ComponentModel;
using CommunityToolkit.Mvvm.Input;
using Microsoft.Windows.ApplicationModel.Resources;

namespace EfAssistant.App.ViewModels;

/// <summary>
/// Sample ViewModel using CommunityToolkit.Mvvm partial property syntax.
/// Uses <see cref="ObservableProperty"/> for change notification and
/// <see cref="RelayCommand"/> for command binding.
/// </summary>
public partial class MainPageViewModel : ObservableObject
{
    [ObservableProperty]
    public partial string Greeting { get; set; }

    [ObservableProperty]
    public partial int Counter { get; set; }

    public MainPageViewModel()
    {
        // User-facing copy comes from the localization catalog, never from a literal
        // (CONTEXT section 7). Resource lookup must not be able to take the app down,
        // so a failure degrades to an empty string rather than throwing.
        string greeting;
        try
        {
            greeting = new ResourceLoader().GetString("Greeting");
        }
        catch (Exception)
        {
            greeting = string.Empty;
        }

        Greeting = greeting;
    }

    [RelayCommand]
    private void Increment()
    {
        Counter++;
    }

    [RelayCommand]
    private void Decrement()
    {
        Counter--;
    }
}
