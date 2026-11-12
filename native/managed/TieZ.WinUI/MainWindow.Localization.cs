using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;

namespace TieZ.Native;

// Localized shell chrome, re-derived in place after every settings snapshot so a language change needs no restart.
public sealed partial class MainWindow
{
    internal void ApplyLanguage()
    {
        var text = ShellStrings.Current;
        Title = text.WindowTitle;
        HistoryNav.Content = text.NavigationHistory;
        AutomationProperties.SetName(HistoryNav, text.NavigationHistory);
        TagsNav.Content = text.NavigationTags;
        AutomationProperties.SetName(TagsNav, text.NavigationTags);
        // The navigation view template renders its own settings affordance for this app, so it follows the
        // accepted language too instead of the Windows display language.
        if (Navigation.SettingsItem is NavigationViewItem settingsItem)
        {
            settingsItem.Content = text.NavigationSettings;
            AutomationProperties.SetName(settingsItem, text.NavigationSettings);
        }
        HideLabel.Text = text.HideToTray;
        AutomationProperties.SetName(HideButton, text.HideToTray);
        ToolTipService.SetToolTip(HideButton, text.HideToTrayHint);
        QuitLabel.Text = text.Quit;
        AutomationProperties.SetName(QuitButton, text.Quit);
        ToolTipService.SetToolTip(QuitButton, text.QuitHint);
        AutomationProperties.SetName(ExitProgress, text.ExitProgressName);
        MigrationNote.Text = text.MigrationNote;
        AutomationProperties.SetName(MigrationNote, text.MigrationNoteName);
        ToolTipService.SetToolTip(MigrationNote, text.MigrationNoteHint);
        ShellError.Title = text.ShellStatusTitle;
        OpenCleanupError.Title = text.OpenCleanupTitle;
        RetryExitButton.Content = text.ExitRetry;
        AutomationProperties.SetName(RetryExitButton, text.ExitRetry);
        FrameworkChrome.ApplyDeferred(this);
    }
}