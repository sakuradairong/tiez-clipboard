using System.Runtime.InteropServices;
using TieZ.Core;

internal static class QueueHotkeySmoke
{
    [DllImport("user32.dll", SetLastError=true)] private static extern bool RegisterHotKey(nint window, int id, uint modifiers, uint key);
    [DllImport("user32.dll", SetLastError=true)] private static extern bool UnregisterHotKey(nint window, int id);
    [DllImport("user32.dll", SetLastError=true)] private static extern nint SendMessageTimeoutW(nint window, uint message, nuint wparam, nint lparam, uint flags, uint timeout, out nuint result);
    internal static void Run(string database)
    {
        int checks = 0;
        void Check(bool value, string message) { checks++; if (!value) throw new Exception(message); }
        using var core = new NativeCore(database);
        core.StartShellAsync(false, false).GetAwaiter().GetResult();
        Check(core.Update(new Dictionary<string,string> { ["app.sequential_hotkey"] = "Ctrl+Alt+Shift+F22" }, core.Snapshot.Revision), "Save disabled chord");
        ulong revision = core.Snapshot.Revision;
        Check(RegisterHotKey(0, 120, 7 | 0x4000, 0x85), "Rare-key conflict fixture unavailable");
        try {
            try { core.Update(new Dictionary<string,string> { ["app.sequential_mode"] = "true" }, revision); throw new Exception("Conflicting queue chord accepted"); }
            catch (CoreException error) when (error.Code == "hotkey_conflict") { checks++; }
            Check(core.Refresh().Revision == revision && core.Snapshot.Values["app.sequential_mode"] == "false", "Conflict changed mode or revision");
        } finally { Check(UnregisterHotKey(0, 120), "Conflict fixture not released"); }
        core.Update(new Dictionary<string,string> { ["app.sequential_mode"] = "true" }, revision);
        nint window = ShellSmoke.FindOwnedWindow();
        Check(SendMessageTimeoutW(window, 0x312, 0x7400, (0x85 << 16) | 7, 3, 3000, out _) != 0, "Synthetic hotkey message failed");
        Check(core.PollShellAsync().GetAwaiter().GetResult().Actions.SequenceEqual(new[] { "paste_next" }), "Sequential action not delivered");
        core.Update(new Dictionary<string,string> { ["app.sequential_mode"] = "false" }, core.Snapshot.Revision);
        Check(RegisterHotKey(0, 120, 7 | 0x4000, 0x85), "Disabled queue still owns chord");
        Check(UnregisterHotKey(0, 120), "Release probe failed");
        Check(SendMessageTimeoutW(window, 0x312, 0x7400, (0x85 << 16) | 7, 3, 3000, out _) != 0, "Stale message fixture failed");
        Check(core.PollShellAsync().GetAwaiter().GetResult().Actions.Count == 0, "Disabled queue accepted stale hotkey");
        core.StopShellAsync().GetAwaiter().GetResult();
        Console.WriteLine($"PASS: queue shortcut {checks} real-DLL registration/conflict/dispatch/release checks; no keyboard injection");
    }
}
