using System.Runtime.InteropServices;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Windowing;
using TieZ.Core;
using Windows.Graphics;

namespace TieZ.Native;

/// Moves the quick-paste window to a screen-edge sliver. The management window is not docked.
internal sealed class EdgeDockController : IDisposable
{
    private readonly ClipboardPopupWindow window;
    private readonly NativeCore core;
    private readonly DispatcherQueueTimer timer;
    private bool disposed, hidden, reporting;
    private DockEdge edge = DockEdge.None;

    internal EdgeDockController(ClipboardPopupWindow window, NativeCore core)
    {
        this.window = window;
        this.core = core;
        timer = window.DispatcherQueue.CreateTimer();
        timer.Interval = TimeSpan.FromMilliseconds(150);
        timer.Tick += Tick;
        timer.Start();
    }

    private void Tick(DispatcherQueueTimer sender, object args)
    {
        if (disposed || reporting || window.HasDialog) return;
        bool enabled = core.Snapshot.Values.TryGetValue("app.edge_docking", out string? value) && value == "true";
        if (!enabled && !hidden) return;
        if (!window.AppWindow.IsVisible) return;
        var position = window.AppWindow.Position;
        var size = window.AppWindow.Size;
        var windowRect = new DockRect(position.X, position.Y, position.X + size.Width, position.Y + size.Height);
        if (!GetCursorPos(out Point cursor)) return;
        var area = DisplayArea.GetFromPoint(new PointInt32(cursor.X, cursor.Y), DisplayAreaFallback.Nearest);
        if (area is null) return;
        var bounds = area.OuterBounds;
        var screen = new DockRect(bounds.X, bounds.Y, bounds.X + bounds.Width, bounds.Y + bounds.Height);
        bool mouseDown = (GetAsyncKeyState(1) & 0x8000) != 0;
        EdgeDock.Step? step = EdgeDock.Next(windowRect, screen, cursor.X, cursor.Y, hidden, edge, mouseDown, enabled);
        if (step is not { } next) return;
        if (next.Moved)
            window.AppWindow.Move(new PointInt32(next.Window.Left, next.Window.Top));
        hidden = next.Hidden;
        edge = next.Edge;
        reporting = true;
        _ = ReportAsync(next.Hidden);
    }

    private async Task ReportAsync(bool value)
    {
        try { await core.SetEdgeHiddenAsync(value); }
        catch (Exception) { /* The next tick reports the same flag again. */ }
        finally { reporting = false; }
    }

    public void Dispose()
    {
        if (disposed) return;
        disposed = true;
        timer.Stop();
        timer.Tick -= Tick;
    }

    [DllImport("user32.dll")]
    private static extern bool GetCursorPos(out Point point);
    [DllImport("user32.dll")]
    private static extern short GetAsyncKeyState(int key);
    [StructLayout(LayoutKind.Sequential)]
    private struct Point { public int X; public int Y; }
}
