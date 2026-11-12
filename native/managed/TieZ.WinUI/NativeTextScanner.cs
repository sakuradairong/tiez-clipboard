using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Media;

namespace TieZ.Native;

/// <summary>
/// Collects every string a user can read in one live native window, including its open dialogs, so a check can
/// prove that no surface was left in the previous language. User payload (typed text, history bodies, tag names)
/// is deliberately excluded; callers keep their fixtures ASCII so any CJK finds a leftover chrome string.
/// </summary>
internal static class NativeTextScanner
{
    /// <summary>Automation id of the interface-language choice, whose items are localized autonyms by design.</summary>
    internal const string AutonymChoiceId = "InterfaceLanguageAutonyms";

    internal static IReadOnlyList<string> Strings(Window window)
    {
        var found = new List<string>();
        if (window.Content is FrameworkElement root) Walk(root, found);
        XamlRoot? xamlRoot = window.Content?.XamlRoot;
        if (xamlRoot is not null)
        {
            foreach (var popup in VisualTreeHelper.GetOpenPopupsForXamlRoot(xamlRoot))
                if (popup.Child is FrameworkElement child) Walk(child, found);
        }
        return found;
    }

    internal static IReadOnlyList<string> Strings(FrameworkElement root)
    {
        var found = new List<string>();
        Walk(root, found);
        return found;
    }

    /// <summary>Describes where each matching string was rendered, for diagnostics and for framework-owned chrome.</summary>
    internal static IReadOnlyList<string> Described(Window window, Func<string, bool> match)
    {
        var found = new List<string>();
        if (window.Content is FrameworkElement root) WalkDescribed(root, match, found);
        XamlRoot? xamlRoot = window.Content?.XamlRoot;
        if (xamlRoot is not null)
        {
            foreach (var popup in VisualTreeHelper.GetOpenPopupsForXamlRoot(xamlRoot))
                if (popup.Child is FrameworkElement child) WalkDescribed(child, match, found);
        }
        return found;
    }

    private static string Detail(DependencyObject node)
    {
        if (node is not FrameworkElement element) return "";
        string name = AutomationProperties.GetName(node) ?? "";
        object? tip = ToolTipService.GetToolTip(element);
        string tooltip = tip switch { null => "<none>", string text => text, ToolTip object1 => object1.Content as string ?? "<object>", _ => "<object>" };
        return $" [name='{name}' tooltip='{tooltip}']";
    }

    private static string Name(DependencyObject node) =>        $"{node.GetType().Name}{(node is FrameworkElement element && element.Name.Length > 0 ? "#" + element.Name : "")}";

    private static string Path(DependencyObject node)
    {
        var steps = new List<string>();
        for (var parent = VisualTreeHelper.GetParent(node); parent is not null && steps.Count < 3; parent = VisualTreeHelper.GetParent(parent))
            steps.Add(Name(parent));
        return steps.Count == 0 ? "" : " in " + string.Join(" < ", steps);
    }

    private static void WalkDescribed(DependencyObject node, Func<string, bool> match, List<string> found)
    {
        // Collapsed subtrees are neither rendered nor reachable through the automation tree, so framework parts
        // that stay collapsed (the top navigation overflow affordance, for example) are not user-visible chrome.
        if (node is UIElement collapsed && collapsed.Visibility == Visibility.Collapsed) return;
        if (IsSystemOwned(node)) return;
        var here = new List<string>();
        Collect(node, here);
        if (here.Any(match)) found.Add($"{string.Join(", ", here.Where(match))} <= {Name(node)}{Path(node)}{Detail(node)}");
        int count = VisualTreeHelper.GetChildrenCount(node);
        for (int index = 0; index < count; index++) WalkDescribed(VisualTreeHelper.GetChild(node, index), match, found);
    }

    /// <summary>Finds the first element carrying one automation id, used to check exempted controls explicitly.</summary>
    internal static DependencyObject? FindByAutomationId(DependencyObject root, string automationId)
    {
        if (AutomationProperties.GetAutomationId(root) == automationId) return root;
        int count = VisualTreeHelper.GetChildrenCount(root);
        for (int index = 0; index < count; index++)
        {
            var found = FindByAutomationId(VisualTreeHelper.GetChild(root, index), automationId);
            if (found is not null) return found;
        }
        return null;
    }

    internal static bool HasCjk(string text) => text.Any(IsCjk);

    /// <summary>
    /// Controls that render Windows' own chrome text and expose no way to relabel it (the colour picker's channel
    /// labels, for example) follow the Windows display language, like the system file picker does. They are
    /// skipped so the scan keeps reporting app-owned chrome only.
    /// </summary>
    internal static bool IsSystemOwned(DependencyObject node)
    {
        for (var current = node; current is not null; current = VisualTreeHelper.GetParent(current))
            if (current is ColorPicker) return true;
        return false;
    }

    private static bool IsCjk(char value) =>
        value is >= '\u2E80' and <= '\u9FFF' or >= '\u3000' and <= '\u303F' or >= '\u3400' and <= '\u4DBF'
            or >= '\uF900' and <= '\uFAFF' or >= '\uFE30' and <= '\uFE4F' or >= '\uFF00' and <= '\uFF65';

    private static void Walk(DependencyObject node, List<string> found)
    {
        // Collapsed subtrees are neither rendered nor reachable through the automation tree.
        if (node is UIElement collapsed && collapsed.Visibility == Visibility.Collapsed) return;
        if (IsSystemOwned(node)) return;
        Collect(node, found);
        int count = VisualTreeHelper.GetChildrenCount(node);
        for (int index = 0; index < count; index++) Walk(VisualTreeHelper.GetChild(node, index), found);
    }

    private static void Add(string? text, List<string> found)
    {
        if (string.IsNullOrWhiteSpace(text)) return;
        if (!found.Contains(text)) found.Add(text);
    }

    private static void Collect(DependencyObject node, List<string> found)
    {
        switch (node)
        {
            case ContentDialog dialog:
                Add(dialog.Title as string, found);
                Add(dialog.PrimaryButtonText, found);
                Add(dialog.SecondaryButtonText, found);
                Add(dialog.CloseButtonText, found);
                break;
            case InfoBar bar:
                Add(bar.Title, found);
                Add(bar.Message, found);
                break;
            case TextBlock text:
                Add(text.Text, found);
                break;
            case TextBox box:
                Add(box.PlaceholderText, found);
                Add(box.Header as string, found);
                break;
            case PasswordBox password:
                Add(password.PlaceholderText, found);
                Add(password.Header as string, found);
                break;
            case NumberBox number:
                Add(number.Header as string, found);
                Add(number.PlaceholderText, found);
                break;
            case ComboBox combo:
                Add(combo.Header as string, found);
                Add(combo.PlaceholderText, found);
                // Language autonyms are the one legitimate CJK family in a non-CJK interface; the dialog that owns
                // this control verifies them explicitly instead of scanning them here.
                if (AutomationProperties.GetAutomationId(combo) == AutonymChoiceId) break;
                foreach (object item in combo.Items)
                {
                    switch (item)
                    {
                        case string text: Add(text, found); break;
                        case ComboBoxItem entry: Add(entry.Content as string, found); break;
                    }
                }
                break;
            case ToggleSwitch toggle:
                Add(toggle.Header as string, found);
                Add(toggle.OnContent as string, found);
                Add(toggle.OffContent as string, found);
                break;
            case AppBarButton appBar:
                Add(appBar.Label, found);
                Add(appBar.Content as string, found);
                break;
            case ButtonBase button:
                Add(button.Content as string, found);
                break;
            case Expander expander:
                Add(expander.Header as string, found);
                break;
            case ContentControl content:
                Add(content.Content as string, found);
                break;
        }
        Add(AutomationProperties.GetName(node), found);
        Add(AutomationProperties.GetHelpText(node), found);
        if (node is FrameworkElement element && ToolTipService.GetToolTip(element) is string tooltip) Add(tooltip, found);
    }
}
