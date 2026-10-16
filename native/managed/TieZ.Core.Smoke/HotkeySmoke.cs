using System.ComponentModel;
using System.Runtime.InteropServices;
using TieZ.Core;

internal static class HotkeySmoke
{
    private static T Read<T>(Task<T> task) => task.GetAwaiter().GetResult();
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    internal static void Run(NativeCore core)
    {
        const string first = "Ctrl+Alt+Shift+F24", second = "Ctrl+Alt+Shift+F23";
        var original = core.Refresh();
        using (var reserved = new Probe(0x87))
        {
            Reject(core, first, "hotkey_conflict");
            Check(core.Refresh().Revision == original.Revision && core.Snapshot.Values["app.hotkey"] == original.Values["app.hotkey"], "Initial registration conflict changed settings");
        }
        Check(core.Update(new Dictionary<string, string> { ["app.hotkey"] = first }, original.Revision), "Register first real shortcut");
        Check(Read(core.PollShellAsync()).Status.MainHotkeyActive, "Registered shortcut not active");
        nint window = ShellSmoke.FindOwnedWindow();
        Signal(window, 0x4100, 0x87, 7);
        Check(Read(core.PollShellAsync()).Actions.SequenceEqual(new[] { "toggle_main" }), "Native hotkey event not delivered");
        using (var reserved = new Probe(0x86))
        {
            ulong revision = core.Snapshot.Revision;
            Reject(core, second, "hotkey_conflict");
            Check(core.Refresh().Revision == revision && core.Snapshot.Values["app.hotkey"] == first, "Replacement conflict lost old settings");
            Check(Read(core.PollShellAsync()).Status.MainHotkeyActive, "Replacement conflict lost active binding");
            Signal(window, 0x4100, 0x87, 7);
            Check(Read(core.PollShellAsync()).Actions.SequenceEqual(new[] { "toggle_main" }), "Old binding stopped dispatching after conflict");
        }
        Check(core.Update(new Dictionary<string, string> { ["app.hotkey"] = "Control+Alt+Shift+F24" }, core.Snapshot.Revision), "Shortcut alias save");
        Signal(window, 0x4100, 0x87, 3);
        Check(Read(core.PollShellAsync()).Actions.Count == 0, "Mismatched modifier message dispatched");
        ulong savedRevision = core.Snapshot.Revision;
        Read(core.StopShellAsync());
        var restarted = Read(core.StartShellAsync(showTray: false, bindMainHotkey: true));
        Check(restarted.MainHotkeyActive && restarted.HotkeyError is null, "Saved shortcut not restored on shell restart");
        Check(core.Refresh().Revision == savedRevision, "Shell restart rewrote settings");
        window = ShellSmoke.FindOwnedWindow();
        Signal(window, 0x4100, 0x87, 7);
        Check(Read(core.PollShellAsync()).Actions.SequenceEqual(new[] { "toggle_main" }), "Restored shortcut did not dispatch");
        core.Update(new Dictionary<string, string> { ["app.hotkey"] = "" }, core.Snapshot.Revision);
        Check(!Read(core.PollShellAsync()).Status.MainHotkeyActive, "Disable did not release binding");
        using (var released = new Probe(0x87)) { }
        Signal(window, 0x4100, 0x87, 7);
        Check(Read(core.PollShellAsync()).Actions.Count == 0, "Disabled binding dispatched stale notification");
        Reject(core, "MouseMiddle", "hotkey_mouse_pending");
        Check(core.LastSideEffectError is null, "Unexpected finalization failure");
        Console.WriteLine("PASS: real OS hotkey conflicts, preserved binding/settings, owner-thread dispatch, alias identity, saved-binding restart without writes, disable/release and stale-message rejection; no keyboard injection");
    }
    private static void Reject(NativeCore core, string value, string code)
    {
        try { core.Update(new Dictionary<string, string> { ["app.hotkey"] = value }, core.Snapshot.Revision); throw new InvalidOperationException("Rejected hotkey accepted"); }
        catch (CoreException error) when (error.Code == code) { }
    }
    private static void Signal(nint window, nuint id, uint key, uint modifiers)
    {
        if (SendMessageTimeoutW(window, 0x312, id, (nint)((key << 16) | modifiers), 3, 3000, out _) == 0)
            throw new Win32Exception(Marshal.GetLastWin32Error());
    }
    private sealed class Probe : IDisposable
    {
        internal Probe(uint key) { if (!RegisterHotKey(0, 0x6400, 7 | 0x4000, key)) throw new Win32Exception(Marshal.GetLastWin32Error(), "Owned rare-chord conflict fixture unavailable"); }
        public void Dispose() { if (!UnregisterHotKey(0, 0x6400)) throw new Win32Exception(Marshal.GetLastWin32Error()); }
    }
    [DllImport("user32.dll", SetLastError = true)] private static extern bool RegisterHotKey(nint window, int id, uint modifiers, uint key);
    [DllImport("user32.dll", SetLastError = true)] private static extern bool UnregisterHotKey(nint window, int id);
    [DllImport("user32.dll", SetLastError = true)] private static extern nint SendMessageTimeoutW(nint window, uint message, nuint wparam, nint lparam, uint flags, uint timeout, out nuint result);
}
