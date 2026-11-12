using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Automation.Provider;
using Microsoft.UI.Xaml.Controls;
using TieZ.Core;

namespace TieZ.Native;

// A draft-only sample, never a history/clipboard read. Native work cannot publish after invalidation.
internal sealed class CleanupPreviewView : UserControl, IDisposable
{
    private readonly Func<string> getRules;
    private readonly Expander expander = new() { HorizontalAlignment = HorizontalAlignment.Stretch };
    internal bool IsContentReady => sample.IsLoaded;
    internal void Expand() => expander.IsExpanded = true;
    private readonly TextBlock intro = new() { TextWrapping = TextWrapping.Wrap };
    private readonly TextBox sample = new() { MaxLength = 4096,
        AcceptsReturn = true, TextWrapping = TextWrapping.Wrap, MinHeight = 70 };
    private readonly TextBox output = new() { IsReadOnly = true, AcceptsReturn = true,
        TextWrapping = TextWrapping.Wrap, MinHeight = 70, MaxHeight = 220, Visibility = Visibility.Collapsed };
    private readonly Button run = new() { IsEnabled = false };
    private Func<AdvancedStrings, string>? statusText;
    private readonly InfoBar status = new() { IsOpen = false, IsClosable = true };
    private NativeCore? core;
    private CancellationTokenSource? pending;
    private int generation;
    private bool disposed;
    internal bool IsBusy => pending is not null;
    internal bool HasResult => output.Visibility == Visibility.Visible;
    internal bool HasError => status.IsOpen && status.Severity == InfoBarSeverity.Error;
    internal bool Ignored { get; private set; }
    internal string Output => output.Text;
    internal string Sample { get => sample.Text; set => sample.Text = value; }
    internal bool HasRetainedData => sample.Text.Length > 0 || output.Text.Length > 0 || pending is not null;

    internal CleanupPreviewView(Func<string> getRules)
    {
        this.getRules = getRules;
        var panel = new StackPanel { Spacing = 10 };
        panel.Children.Add(intro);
        panel.Children.Add(sample); panel.Children.Add(run); panel.Children.Add(output); panel.Children.Add(status);
        expander.Content = panel; Content = expander;
        sample.TextChanging += (_, _) => Invalidate();
        run.Click += async (_, _) => await RunAsync();
        ApplyLanguage();
    }
    internal void Initialize(NativeCore sharedCore)
    { if (!disposed) { core = sharedCore; run.IsEnabled = true; } }
    internal void Invalidate()
    {
        generation++;
        pending?.Cancel(); pending?.Dispose(); pending = null;
        output.Text = ""; output.Visibility = Visibility.Collapsed;
        status.IsOpen = false; Ignored = false;
        run.IsEnabled = !disposed && core is not null;
    }
    internal void Reset() { Invalidate(); sample.Text = ""; }
    private async Task RunAsync()
    {
        if (disposed || core is null) return;
        Invalidate();
        int current = generation;
        var cancellation = new CancellationTokenSource(); pending = cancellation;
        run.IsEnabled = false;
        try
        {
            var result = await core.PreviewCleanupAsync(getRules(), sample.Text, cancellation.Token);
            if (disposed || current != generation || cancellation.IsCancellationRequested) return;
            output.Text = result.Text; output.Visibility = Visibility.Visible; Ignored = result.Ignored;
            ShowStatus(result.Ignored ? InfoBarSeverity.Warning : InfoBarSeverity.Success, text => string.Format(text.RunResultFormat, result.Rules, result.Matches)
                + (result.Ignored ? text.IgnoredSuffix : text.DoneSuffix));
        }
        catch (OperationCanceledException) { }
        catch (Exception error)
        {
            if (disposed || current != generation) return;
            if (error is CoreException { Code: "invalid_rule" }) ShowStatus(InfoBarSeverity.Error, text => text.InvalidRule);
            else if (error is CoreException { Code: "preview_limit" }) ShowStatus(InfoBarSeverity.Error, text => text.LimitError);
            else ShowStatus(InfoBarSeverity.Error, text => string.Format(text.FailureFormat, error.Message));
        }
        finally
        {
            if (ReferenceEquals(pending, cancellation)) { pending = null; cancellation.Dispose(); }
            if (!disposed && current == generation) run.IsEnabled = core is not null;
        }
    }
    private void ShowStatus(InfoBarSeverity severity, Func<AdvancedStrings, string> message)
    {
        statusText = message; status.Severity = severity;
        status.Message = message(AdvancedStrings.Current); status.IsOpen = true;
    }
    // Only chrome is re-derived; the sample draft, the result text and any pending request stay untouched.
    internal void ApplyLanguage()
    {
        if (disposed) return;
        var text = AdvancedStrings.Current;
        expander.Header = text.PreviewHeader;
        intro.Text = text.PreviewIntro;
        sample.Header = text.SampleHeader;
        output.Header = text.OutputHeader;
        run.Content = text.Run;
        if (status.IsOpen && statusText is { } message) status.Message = message(text);
    }
    internal void InvokeRun()
    {
        if (!run.IsEnabled) throw new InvalidOperationException("Cleanup test action is disabled.");
        ((IInvokeProvider)new ButtonAutomationPeer(run).GetPattern(PatternInterface.Invoke)).Invoke();
    }
    public void Dispose() { disposed = true; Reset(); core = null; }
}
