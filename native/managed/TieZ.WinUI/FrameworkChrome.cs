using System.Runtime.CompilerServices;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;

namespace TieZ.Native;

/// <summary>
/// Framework template parts that carry text of their own: the navigation view pane toggle, the command bar
/// overflow affordance, the info bar severity icon and close button, and the text box clear button. Those parts
/// are rendered by this app, so their labels and automation names follow the accepted interface language instead
/// of the Windows display language. Only elements that really live inside the matching framework control are
/// touched, so an app button that happens to share a template name (the history "load more" button, for instance)
/// keeps its own labels.
/// </summary>
internal static class FrameworkChrome
{
    private static readonly ConditionalWeakTable<InfoBar, object> watched = [];

    /// <summary>
    /// Applies the labels now, once more on the dispatcher queue and once after the next layout pass: the
    /// navigation view writes its overflow tooltip and help text while it measures its top navigation area, which
    /// happens after a language change has already been applied.
    /// </summary>
    internal static void ApplyDeferred(Window window)
    {
        if (!Apply(window)) return;
        window.DispatcherQueue.TryEnqueue(() => Apply(window));
        if (!TryRoot(window, out var root)) return;
        int remaining = 5;
        EventHandler<object>? settled = null;
        settled = (_, _) =>
        {
            // A window can be closed while the passes are still queued, and a closed window has no content.
            if (!Apply(window)) root.LayoutUpdated -= settled;
            else if (--remaining <= 0) root.LayoutUpdated -= settled;
        };
        root.LayoutUpdated += settled;
    }

    private static bool TryRoot(Window window, out FrameworkElement root)
    {
        root = null!;
        try
        {
            if (window.Content is FrameworkElement content) { root = content; return true; }
        }
        catch (Exception) { }
        return false;
    }

    /// <summary>Returns false when the window is already closed, where touching Content throws.</summary>
    internal static bool Apply(Window window)
    {
        if (!TryRoot(window, out var root)) return false;
        Apply(root);
        XamlRoot? xamlRoot = root.XamlRoot;
        if (xamlRoot is null) return true;
        foreach (var popup in VisualTreeHelper.GetOpenPopupsForXamlRoot(xamlRoot))
            if (popup.Child is FrameworkElement child) Apply(child);
        return true;
    }

    internal static void Apply(DependencyObject root)
    {
        var text = FrameworkStrings.Current;
        var common = CommonStrings.Current;
        foreach (var node in Descendants(root).Prepend(root))
        {
            switch (node)
            {
                case InfoBar bar:
                    // The severity icon and its automation name are rewritten by the control whenever the bar is
                    // opened or its severity changes, so it is labelled again from that moment on.
                    Watch(bar);
                    LabelIcon(bar);
                    break;
                case Button button when button.Name == "DeleteButton" && Inside<TextBox>(button) is not null:
                    Label(button, text.ClearText);
                    break;
                case Button button when Inside<NavigationView>(button) is not null && NavigationLabel(button, text, common) is string navigation:
                    Label(button, navigation);
                    break;
                case Button overflow when overflow.Name == "MoreButton" && Inside<CommandBar>(overflow) is not null:
                    Label(overflow, text.OverflowMore);
                    break;
                case Button close when close.Name == "CloseButton" && Inside<InfoBar>(close) is not null:
                    Label(close, common.Close);
                    break;
            }
        }
    }

    private static void Label(Button button, string text)
    {
        AutomationProperties.SetName(button, text);
        AutomationProperties.SetHelpText(button, text);
        ToolTipService.SetToolTip(button, text);
    }

    private static void Watch(InfoBar bar)
    {
        if (watched.TryGetValue(bar, out _)) return;
        watched.Add(bar, new object());
        bar.RegisterPropertyChangedCallback(InfoBar.IsOpenProperty, (_, _) => LabelIcon(bar));
        bar.RegisterPropertyChangedCallback(InfoBar.SeverityProperty, (_, _) => LabelIcon(bar));
    }

    private static void LabelIcon(InfoBar bar)
    {
        var text = FrameworkStrings.Current;
        string name = bar.Severity switch
        {
            InfoBarSeverity.Error => text.IconError,
            InfoBarSeverity.Warning => text.IconWarning,
            InfoBarSeverity.Success => text.IconSuccess,
            _ => text.IconInformation,
        };
        foreach (var icon in Descendants(bar).OfType<TextBlock>().Where(block => block.Name == "StandardIcon"))
            AutomationProperties.SetName(icon, name);
    }

    private static string? NavigationLabel(Button button, FrameworkStrings text, CommonStrings common) => button.Name switch
    {
        "TogglePaneButton" => text.TogglePane,
        "NavigationViewCloseButton" => text.ClosePane,
        "NavigationViewBackButton" => common.Back,
        "PaneAutoSuggestButton" => common.Search,
        "TopNavOverflowButton" => text.OverflowMore,
        _ => null,
    };

    private static T? Inside<T>(DependencyObject node) where T : DependencyObject
    {
        for (var parent = VisualTreeHelper.GetParent(node); parent is not null; parent = VisualTreeHelper.GetParent(parent))
            if (parent is T match) return match;
        return null;
    }

    private static IEnumerable<DependencyObject> Descendants(DependencyObject root)
    {
        int count = VisualTreeHelper.GetChildrenCount(root);
        for (int index = 0; index < count; index++)
        {
            var child = VisualTreeHelper.GetChild(root, index);
            yield return child;
            foreach (var nested in Descendants(child)) yield return nested;
        }
    }
}