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
    private readonly Expander expander = new() { Header = "规则测试预览", HorizontalAlignment = HorizontalAlignment.Stretch };
    internal bool IsContentReady => sample.IsLoaded;
    internal void Expand() => expander.IsExpanded = true;
    private readonly TextBox sample = new() { Header = "测试文本（最多 4096 字符）", MaxLength = 4096,
        AcceptsReturn = true, TextWrapping = TextWrapping.Wrap, MinHeight = 70 };
    private readonly TextBox output = new() { Header = "处理结果", IsReadOnly = true, AcceptsReturn = true,
        TextWrapping = TextWrapping.Wrap, MinHeight = 70, MaxHeight = 220, Visibility = Visibility.Collapsed };
    private readonly Button run = new() { Content = "测试当前规则", IsEnabled = false };
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
        panel.Children.Add(new TextBlock { Text = "仅测试这一组文字清理规则，不包含应用匹配、策略忽略动作、其他规则组或敏感检测。测试不会保存配置或操作剪贴板。最多 32 条规则、4096 次匹配、64 KiB 输出。", TextWrapping = TextWrapping.Wrap });
        panel.Children.Add(sample); panel.Children.Add(run); panel.Children.Add(output); panel.Children.Add(status);
        expander.Content = panel; Content = expander;
        sample.TextChanging += (_, _) => Invalidate();
        run.Click += async (_, _) => await RunAsync();
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
            status.Severity = result.Ignored ? InfoBarSeverity.Warning : InfoBarSeverity.Success;
            status.Message = $"已执行 {result.Rules} 条规则，匹配 {result.Matches} 处。"
                + (result.Ignored ? "此规则组会忽略这条采集。" : "测试完成，配置尚未因此保存。");
            status.IsOpen = true;
        }
        catch (OperationCanceledException) { }
        catch (Exception error)
        {
            if (disposed || current != generation) return;
            status.Severity = InfoBarSeverity.Error;
            status.Message = error is CoreException { Code: "invalid_rule" } ? "规则含无效正则或缺少 =>，请修正后重试。"
                : error is CoreException { Code: "preview_limit" } ? "测试超过样本、规则、匹配或输出上限，请缩小样本或规则范围。"
                : $"测试未完成：{error.Message}";
            status.IsOpen = true;
        }
        finally
        {
            if (ReferenceEquals(pending, cancellation)) { pending = null; cancellation.Dispose(); }
            if (!disposed && current == generation) run.IsEnabled = core is not null;
        }
    }
    internal void InvokeRun()
    {
        if (!run.IsEnabled) throw new InvalidOperationException("Cleanup test action is disabled.");
        ((IInvokeProvider)new ButtonAutomationPeer(run).GetPattern(PatternInterface.Invoke)).Invoke();
    }
    public void Dispose() { disposed = true; Reset(); core = null; }
}
