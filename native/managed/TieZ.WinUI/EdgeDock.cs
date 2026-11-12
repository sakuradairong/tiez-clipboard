namespace TieZ.Native;

internal enum DockEdge { None, Top, Left, Right }

internal readonly record struct DockRect(int Left, int Top, int Right, int Bottom)
{
    internal int Width => Right - Left;
    internal int Height => Bottom - Top;
}

/// Legacy edge-dock geometry. The window stays shown as a three-pixel sliver; hidden is a logical flag.
internal static class EdgeDock
{
    internal const int Threshold = 5;
    internal const int HideSize = 3;

    internal static DockEdge EdgeOf(DockRect window, DockRect screen)
    {
        if (window.Top <= screen.Top + Threshold) return DockEdge.Top;
        if (window.Left <= screen.Left + Threshold) return DockEdge.Left;
        if (window.Right >= screen.Right - Threshold) return DockEdge.Right;
        return DockEdge.None;
    }

    internal static bool MouseInside(int x, int y, DockRect window) =>
        x >= window.Left && x <= window.Right && y >= window.Top && y <= window.Bottom;

    internal static bool MouseOnSliver(int x, int y, DockRect window, DockRect screen, DockEdge edge) => edge switch
    {
        DockEdge.Top => y <= screen.Top + Threshold && x >= window.Left && x <= window.Right,
        DockEdge.Left => x <= screen.Left + Threshold && y >= window.Top && y <= window.Bottom,
        DockEdge.Right => x >= screen.Right - Threshold && y >= window.Top && y <= window.Bottom,
        _ => false
    };

    internal readonly record struct Step(DockRect Window, bool Hidden, DockEdge Edge, bool Moved);

    /// <summary>One monitor tick. A null result means the window stays where it is.</summary>
    internal static Step? Next(DockRect window, DockRect screen, int mouseX, int mouseY, bool hidden, DockEdge current, bool mouseDown, bool enabled)
    {
        if (!enabled) return hidden ? new Step(window, false, DockEdge.None, false) : null;
        int centerX = window.Left + window.Width / 2;
        int centerY = window.Top + window.Height / 2;
        bool onMonitor = centerX >= screen.Left && centerX < screen.Right && centerY >= screen.Top && centerY < screen.Bottom;
        if (!hidden && !onMonitor) return new Step(window, false, DockEdge.None, false);
        if (hidden)
        {
            if (current == DockEdge.None || !MouseOnSliver(mouseX, mouseY, window, screen, current)) return null;
            int width = window.Width;
            int height = window.Height;
            DockRect revealed = current switch
            {
                DockEdge.Top => new(window.Left, screen.Top, window.Right, screen.Top + height),
                DockEdge.Left => new(screen.Left, window.Top, screen.Left + width, window.Bottom),
                DockEdge.Right => new(screen.Right - width, window.Top, screen.Right, window.Bottom),
                _ => window
            };
            return new Step(revealed, false, DockEdge.None, true);
        }
        DockEdge dock = EdgeOf(window, screen);
        if (dock == DockEdge.None || MouseInside(mouseX, mouseY, window) || mouseDown) return null;
        int w = window.Width;
        int h = window.Height;
        DockRect parked = dock switch
        {
            DockEdge.Top => new(window.Left, screen.Top - h + HideSize, window.Right, screen.Top + HideSize),
            DockEdge.Left => new(screen.Left - w + HideSize, window.Top, screen.Left + HideSize, window.Bottom),
            DockEdge.Right => new(screen.Right - HideSize, window.Top, screen.Right - HideSize + w, window.Bottom),
            _ => window
        };
        return new Step(parked, true, dock, true);
    }

    internal static void AssertFixtures()
    {
        var screen = new DockRect(0, 0, 1920, 1080);
        var top = new DockRect(400, 0, 800, 560);
        Step? parked = Next(top, screen, 10, 10, false, DockEdge.None, false, true);
        if (parked is not { Hidden: true, Edge: DockEdge.Top, Moved: true } result
            || result.Window.Top != 3 - 560 || result.Window.Bottom != 3)
            throw new InvalidOperationException("A window on the top edge should park as a three-pixel sliver.");
        if (Next(top, screen, 500, 20, false, DockEdge.None, false, true) is not null)
            throw new InvalidOperationException("The pointer inside the window must keep it open.");
        if (Next(top, screen, 10, 10, false, DockEdge.None, true, true) is not null)
            throw new InvalidOperationException("A pressed mouse button must not dock a window being dragged.");
        Step? shown = Next(result.Window, screen, 500, 2, true, DockEdge.Top, false, true);
        if (shown is not { Hidden: false, Moved: true } reveal || reveal.Window.Top != 0)
            throw new InvalidOperationException("The pointer on the top sliver should restore the window to the screen edge.");
        if (Next(result.Window, screen, 500, 2, true, DockEdge.Top, false, false) is not { Hidden: false, Moved: false })
            throw new InvalidOperationException("Turning docking off clears the hidden flag without jumping the sliver.");
        var right = new DockRect(1920 - 400, 100, 1920, 660);
        if (Next(right, screen, 0, 0, false, DockEdge.None, false, true) is not { Edge: DockEdge.Right, Hidden: true })
            throw new InvalidOperationException("A window on the right edge should dock right.");
    }
}
