using System.Runtime.InteropServices;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using TieZ.Core;
using Windows.Graphics;

namespace TieZ.Native;

internal sealed record CompactPreviewRequest(HistorySummary Summary, ulong Revision, bool AllowSensitive,
    FrameworkElement Anchor, Func<bool> IsCurrent);

internal sealed class CompactPreviewController : IDisposable
{
    private readonly MainWindow owner;
    private readonly NativeCore core;
    private readonly DispatcherQueueTimer delay;
    private CompactPreviewWindow? window;
    private CompactPreviewRequest? pending;
    private CancellationTokenSource? cancellation;
    private int generation;
    private bool disposed, enabled;
    private ElementTheme theme;
    internal int WindowsCreated { get; private set; }
    internal nint WindowHandle => window is null ? 0 : WinRT.Interop.WindowNative.GetWindowHandle(window);
    internal bool Visible => window?.AppWindow.IsVisible == true;
    internal bool HasPending => pending is not null;
    internal long? DisplayedId => window?.DisplayedId;
    internal bool HasImage => window?.HasImage == true;
    internal bool HasDocument => window?.HasDocument == true;
    internal bool ShowPreservedForeground => window?.ShowPreservedForeground == true;
    internal string FocusDiagnostic => window is null ? "no window" : $"before={window.FocusBeforeShow:X}, after={window.FocusAfterShow:X}";
    internal bool IsNonActivating => window?.IsNonActivating == true;
    internal string Text => window?.Text ?? "";
    internal ElementTheme CurrentTheme => window?.CurrentTheme ?? theme;
    internal RectInt32? Bounds => window is null ? null : new RectInt32(window.AppWindow.Position.X,
        window.AppWindow.Position.Y, window.AppWindow.Size.Width, window.AppWindow.Size.Height);

    internal CompactPreviewController(MainWindow owner, NativeCore core)
    {
        this.owner = owner; this.core = core;
        delay = owner.DispatcherQueue.CreateTimer(); delay.IsRepeating = false;
        delay.Interval = TimeSpan.FromSeconds(1); delay.Tick += OnDelay;
        owner.AppWindow.Changed += OnOwnerChanged;
    }
    internal void Apply(bool compactMode, ElementTheme colorMode)
    {
        enabled = compactMode; theme = colorMode;
        if (!enabled) Cancel();
        if (window is not null) window.CurrentTheme = theme;
    }
    internal void Request(CompactPreviewRequest request)
    {
        Cancel();
        if (disposed || !enabled || !owner.AppWindow.IsVisible || !request.IsCurrent()
            || request.Summary.ContentType == "file" || (request.Summary.IsSensitive && !request.AllowSensitive)) return;
        pending = request; cancellation = new(); delay.Start();
    }
    internal void Cancel()
    {
        generation++; delay.Stop(); pending = null;
        cancellation?.Cancel(); cancellation?.Dispose(); cancellation = null;
        window?.HideAndClear();
    }
    private bool Current(int current, CompactPreviewRequest request, CancellationToken token) =>
        !disposed && enabled && !token.IsCancellationRequested && generation == current
        && owner.AppWindow.IsVisible && request.IsCurrent();
    private async void OnDelay(DispatcherQueueTimer sender, object args)
    {
        if (pending is not { } request || cancellation is null) return;
        int current = generation; var token = cancellation.Token;
        try
        {
            if (!Current(current, request, token)) { Cancel(); return; }
            var entry = await core.GetPreviewEntryAsync(request.Summary.Id, request.Revision, request.AllowSensitive, token);
            if (!Current(current, request, token)) { if (current == generation) Cancel(); return; }
            if (window is null)
            {
                var created = new CompactPreviewWindow(owner); window = created; WindowsCreated++;
                created.Closed += (_, _) => { if (ReferenceEquals(window, created)) { window = null; Cancel(); } };
            }
            var target = window;
            target.CurrentTheme = theme;
            await target.PrepareAsync(entry, core, request.Revision, request.AllowSensitive, token);
            if (!Current(current, request, token)) { if (current == generation) Cancel(); return; }
            var anchor = AnchorBounds(request.Anchor);
            var work = DisplayArea.GetFromRect(anchor, DisplayAreaFallback.Nearest).WorkArea;
            var ownerBounds = new RectInt32(owner.AppWindow.Position.X, owner.AppWindow.Position.Y,
                owner.AppWindow.Size.Width, owner.AppWindow.Size.Height);
            target.ShowAt(CompactPreviewPlacement.Calculate(ownerBounds, anchor, work, request.Anchor.XamlRoot.RasterizationScale));
            pending = null;
        }
        catch (OperationCanceledException) { }
        catch (ObjectDisposedException) when (disposed) { }
        catch (CoreException error) when (error.Code is "history_conflict" or "sensitive_confirmation" or "not_found")
        { if (current == generation) Cancel(); }
        catch (Exception error)
        {
            if (current != generation || disposed) return;
            Cancel(); owner.History.SetPreviewError($"紧凑预览不可用：{error.Message}");
        }
    }
    private RectInt32 AnchorBounds(FrameworkElement anchor)
    {
        if (!anchor.IsLoaded || anchor.XamlRoot is null) throw new InvalidOperationException("Preview anchor is unloaded.");
        var bounds = anchor.TransformToVisual(null).TransformBounds(new Windows.Foundation.Rect(0, 0, anchor.ActualWidth, anchor.ActualHeight));
        double scale = anchor.XamlRoot.RasterizationScale;
        var origin = new Point();
        if (!ClientToScreen(WinRT.Interop.WindowNative.GetWindowHandle(owner), ref origin))
            throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
        return new RectInt32(origin.X + (int)Math.Round(bounds.X * scale), origin.Y + (int)Math.Round(bounds.Y * scale),
            Math.Max(1, (int)Math.Round(bounds.Width * scale)), Math.Max(1, (int)Math.Round(bounds.Height * scale)));
    }
    private void OnOwnerChanged(AppWindow sender, AppWindowChangedEventArgs args)
    { if (!sender.IsVisible || args.DidPositionChange || args.DidSizeChange) Cancel(); }
    public void Dispose()
    {
        if (disposed) return;
        disposed = true; Cancel(); delay.Tick -= OnDelay;
        owner.AppWindow.Changed -= OnOwnerChanged;
        window?.Close(); window = null;
    }
    [StructLayout(LayoutKind.Sequential)] private struct Point { public int X, Y; }
    [DllImport("user32.dll", SetLastError = true)] [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool ClientToScreen(nint window, ref Point point);
}
