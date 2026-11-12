using System.Runtime.InteropServices;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using TieZ.Core;
using Windows.Graphics;

namespace TieZ.Native;

/// Parks a visible window on the top, left or right screen edge. Popup and management each report;
/// quick paste is edge-hidden when either one is parked.
internal sealed class EdgeDockController : IDisposable
{
    private static readonly object Gate = new();
    private static readonly Dictionary<string, bool> Parked = [];
    private static bool lastCombined;
    private readonly Window window;
    private readonly NativeCore core;
    private readonly Func<bool> blocked;
    private readonly string id;
    private readonly DispatcherQueueTimer timer;
    private bool disposed, hidden, reporting;
    private DockEdge edge = DockEdge.None;

    internal EdgeDockController(Window window, NativeCore core, Func<bool> blocked, string id)
    {
        this.window = window;
        this.core = core;
        this.blocked = blocked;
        this.id = id;
        timer = window.DispatcherQueue.CreateTimer();
        timer.Interval = TimeSpan.FromMilliseconds(150);
        timer.Tick += Tick;
        timer.Start();
    }

    internal static bool Combined(bool popup, bool management) => popup || management;

    private void Tick(DispatcherQueueTimer sender, object args)
    {
        if (disposed || reporting || blocked()) return;
        bool enabled = core.Snapshot.Values.TryGetValue("app.edge_docking", out string? value) && value == "true";
        if (!window.AppWindow.IsVisible)
        {
            if (hidden) Publish(false, DockEdge.None);
            return;
        }
        if (!enabled && !hidden) return;
        var position = window.AppWindow.Position;
        var size = window.AppWindow.Size;
        var windowRect = new DockRect(position.X, position.Y, position.X + size.Width, position.Y + size.Height);
        if (!GetCursorPos(out Point cursor)) return;
        var area = DisplayArea.GetFromPoint(new PointInt32(cursor.X, cursor.Y), DisplayAreaFallback.Nearest);
        if (area is null) return;
        var bounds = area.OuterBounds;
        var screen = new DockRect(bounds.X, bounds.Y, bounds.X + bounds.Width, bounds.Y + bounds.Height);
        bool mouseDown = (GetAsyncKeyState(1) & 0x8000) != 0;
        if (EdgeDock.Next(windowRect, screen, cursor.X, cursor.Y, hidden, edge, mouseDown, enabled) is not { } next) return;
        if (next.Moved) window.AppWindow.Move(new PointInt32(next.Window.Left, next.Window.Top));
        Publish(next.Hidden, next.Edge);
    }

    private void Publish(bool value, DockEdge nextEdge)
    {
        hidden = value;
        edge = nextEdge;
        bool combined;
        lock (Gate)
        {
            Parked[id] = value;
            combined = Parked.Values.Any(parked => parked);
            if (combined == lastCombined) return;
            lastCombined = combined;
        }
        reporting = true;
        _ = ReportAsync(combined);
    }

    private async Task ReportAsync(bool value)
    {
        try { await core.SetEdgeHiddenAsync(value); }
        catch (Exception) { lock (Gate) lastCombined = !value; }
        finally { reporting = false; }
    }

    public void Dispose()
    {
        if (disposed) return;
        disposed = true;
        timer.Stop();
        timer.Tick -= Tick;
        Publish(false, DockEdge.None);
    }

    [DllImport("user32.dll")]
    private static extern bool GetCursorPos(out Point point);
    [DllImport("user32.dll")]
    private static extern short GetAsyncKeyState(int key);
    [StructLayout(LayoutKind.Sequential)]
    private struct Point { public int X; public int Y; }
}
