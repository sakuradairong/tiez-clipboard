using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Text;
using TieZ.Core;

internal static class ShellSmoke
{
    private static T Read<T>(Task<T> task) => task.GetAwaiter().GetResult();
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    internal static void Run(NativeCore core)
    {
        var initial = Read(core.PollShellAsync());
        Check(!initial.Status.Running && initial.Status.Starts == 0 && initial.Actions.Count == 0, "Explicit shell startup");
        var started = Read(core.StartShellAsync(showTray: false, bindMainHotkey: false));
        Check(started.Running && !started.TrayVisible && started.Starts == 1, "Headless shell message thread start");
        Check(Read(core.StartShellAsync(false, false)).Starts == 1, "Idempotent shell start");
        try { Read(core.StartShellAsync(true)); throw new InvalidOperationException("Changed live shell options accepted without restart"); }
        catch (CoreException error) when (error.Code == "shell_restart_required") { }
        nint window = FindOwnedWindow();
        Send(window, 0x201); // Mouse-down must not trigger showing.
        Check(Read(core.PollShellAsync()).Actions.Count == 0, "Mouse-down triggered tray show");
        for (int index = 0; index < 256; index++) Send(window, 0x400); // NIN_SELECT
        Send(window, 0x401); // Keyboard select uses same show action.
        Check(Read(core.PollShellAsync()).Actions.SequenceEqual(new[] { "show_main" }), "Tray actions were lost or unbounded");
        Check(Read(core.PollShellAsync()).Actions.Count == 0, "Tray action drained twice");
        var stopped = Read(core.StopShellAsync());
        Check(!stopped.Running && !stopped.TrayVisible && !IsWindow(window), "Shell stop did not remove native window");
        Check(!Read(core.StopShellAsync()).Running, "Idempotent shell stop");
        Check(Read(core.StartShellAsync(false, false)).Starts == 2, "Shell message thread restart");
        Console.WriteLine("PASS: real native shell window, bounded release/keyboard-select events, ignored mouse-down, explicit lifecycle and restart; no popup, clipboard or injected input");
    }
    internal static void DisposeActive(string database)
    {
        nint window;
        using (var core = new NativeCore(database)) { Read(core.StartShellAsync(false, false)); window = FindOwnedWindow(); }
        Check(!IsWindow(window), "Active core disposal left shell HWND alive");
        using var reopened = new NativeCore(database);
        Check(Read(reopened.StartShellAsync(false, false)).Starts == 1, "Shell reopen after active disposal");
    }
    internal static nint FindOwnedWindow()
    {
        nint found = 0;
        bool duplicate = false;
        EnumWindows((window, _) =>
        {
            GetWindowThreadProcessId(window, out uint process);
            if (process != (uint)Environment.ProcessId) return true;
            var name = new StringBuilder(256);
            GetClassNameW(window, name, name.Capacity);
            if (!name.ToString().StartsWith($"TieZNativeShell-{Environment.ProcessId}-", StringComparison.Ordinal)) return true;
            duplicate |= found != 0; found = window; return true;
        }, 0);
        Check(!duplicate, "Duplicate native shell window");
        Check(found != 0, "Owned shell window missing"); return found;
    }
    private static void Send(nint window, uint notification)
    {
        // Synchronous bounded message delivery to only our own Rust HWND. No real tray click.
        if (SendMessageTimeoutW(window, 0x8000 + 41, 0, (nint)notification, 3, 3000, out _) == 0)
            throw new Win32Exception(Marshal.GetLastWin32Error(), "Owned shell message timeout");
    }
    private delegate bool EnumWindowProc(nint window, nint data);
    [DllImport("user32.dll")] private static extern bool EnumWindows(EnumWindowProc callback, nint data);
    [DllImport("user32.dll")] private static extern bool IsWindow(nint window);
    [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(nint window, out uint process);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] private static extern int GetClassNameW(nint window, StringBuilder name, int maximum);
    [DllImport("user32.dll", SetLastError = true)] private static extern nint SendMessageTimeoutW(nint window, uint message, nuint wparam, nint lparam, uint flags, uint timeout, out nuint result);
}
