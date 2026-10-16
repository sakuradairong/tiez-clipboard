using System.ComponentModel;
using System.Diagnostics;
using System.Globalization;
using System.Runtime.InteropServices;

// Controlled windows only. No clipboard APIs, injected keys, or user window text.
internal sealed class FixtureForegroundUnavailable : Exception { }

internal sealed class FixtureWindow : IDisposable
{
    internal nint Handle { get; }
    internal FixtureWindow(string name)
    {
        Handle = FocusApi.CreateWindowExW(0, "STATIC", name, 0x00CF0000, 80, 80, 300, 140, 0, 0, 0, 0);
        FocusApi.Require(Handle != 0, "Create fixture window");
        FocusApi.ShowWindow(Handle, 4); // SW_SHOWNOACTIVATE
    }
    internal static void Pump()
    {
        while (FocusApi.PeekMessageW(out var message, 0, 0, 0, 1)) FocusApi.DispatchMessageW(in message);
    }
    internal static void Activate(nint window)
    {
        FocusApi.ShowWindow(window, 9); // Restore controlled window if minimized.
        FocusApi.SetForegroundWindow(window);
        var deadline = DateTime.UtcNow.AddSeconds(2);
        while (FocusApi.GetForegroundWindow() != window)
        {
            if (DateTime.UtcNow >= deadline) throw new FixtureForegroundUnavailable();
            Pump(); Thread.Sleep(10);
        }
    }
    public void Dispose() { if (FocusApi.IsWindow(Handle)) FocusApi.Require(FocusApi.DestroyWindow(Handle), "Destroy fixture window"); }
}
internal sealed class FixtureChild : IDisposable
{
    private readonly Process process;
    internal nint[] Windows { get; }
    internal FixtureChild()
    {
        string executable = Environment.ProcessPath ?? throw new InvalidOperationException("Missing apphost path");
        if (!Path.GetFileNameWithoutExtension(executable).Equals("TieZ.Focus.Smoke", StringComparison.OrdinalIgnoreCase))
            throw new InvalidOperationException("Run the built apphost executable or use dotnet run.");
        process = Process.Start(new ProcessStartInfo(executable)
        {
            ArgumentList = { "--fixture-target" }, UseShellExecute = false, CreateNoWindow = true,
            WindowStyle = ProcessWindowStyle.Hidden, RedirectStandardInput = true,
            RedirectStandardOutput = true, RedirectStandardError = true,
        }) ?? throw new InvalidOperationException("Target fixture did not start");
        try
        {
            var ready = process.StandardOutput.ReadLineAsync();
            if (!ready.Wait(TimeSpan.FromSeconds(10))) throw new InvalidOperationException("Target fixture startup timed out");
            Windows = (ready.Result ?? "").Split(' ', StringSplitOptions.RemoveEmptyEntries)
                .Select(value => new nint(long.Parse(value, CultureInfo.InvariantCulture))).ToArray();
            if (Windows.Length != 2 || Windows.Any(window => !FocusApi.IsWindow(window))) throw new InvalidOperationException("Invalid target fixture handles");
        }
        catch { Dispose(); throw; }
    }
    internal void Close(nint window) => FocusApi.Require(FocusApi.PostMessageW(window, 0x10, 0, 0), "Close owned target fixture");
    public void Dispose()
    {
        if (!process.HasExited)
        {
            try { process.StandardInput.WriteLine("quit"); process.StandardInput.Flush(); }
            catch (IOException) { }
            var until = DateTime.UtcNow.AddSeconds(3);
            while (!process.HasExited && DateTime.UtcNow < until) { FixtureWindow.Pump(); Thread.Sleep(10); }
            if (!process.HasExited)
            {
                process.Kill();
                if (!process.WaitForExit(3000)) throw new InvalidOperationException("Owned fixture process termination unconfirmed");
            }
        }
        process.Dispose();
    }
    internal static void RunTarget()
    {
        using var first = new FixtureWindow("TieZ focus fixture A");
        using var second = new FixtureWindow("TieZ focus fixture B");
        Console.WriteLine($"{first.Handle.ToInt64()} {second.Handle.ToInt64()}");
        _ = Task.Run(() => { if (Console.ReadLine() is not null) { FocusApi.PostMessageW(first.Handle, 0x10, 0, 0); FocusApi.PostMessageW(second.Handle, 0x10, 0, 0); } });
        while (FocusApi.IsWindow(first.Handle) || FocusApi.IsWindow(second.Handle)) { FixtureWindow.Pump(); Thread.Sleep(10); }
    }
}
internal static class FocusApi
{
    [StructLayout(LayoutKind.Sequential)] internal struct Message
    {
        internal nint Window; internal uint Id; internal nuint WParam; internal nint LParam;
        internal uint Time; internal int X, Y; internal uint Private;
    }
    internal static void Require(bool value, string operation) { if (!value) throw new Win32Exception(Marshal.GetLastWin32Error(), operation); }
    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)] internal static extern nint CreateWindowExW(uint ex, string cls, string name, uint style, int x, int y, int width, int height, nint parent, nint menu, nint instance, nint param);
    [DllImport("user32.dll", SetLastError = true)] internal static extern bool DestroyWindow(nint window);
    [DllImport("user32.dll")] internal static extern bool IsWindow(nint window);
    [DllImport("user32.dll")] internal static extern bool ShowWindow(nint window, int command);
    [DllImport("user32.dll")] internal static extern nint GetForegroundWindow();
    [DllImport("user32.dll")] internal static extern bool SetForegroundWindow(nint window);
    [DllImport("user32.dll")] internal static extern uint GetWindowThreadProcessId(nint window, out uint process);
    [DllImport("user32.dll")] internal static extern bool PeekMessageW(out Message message, nint window, uint min, uint max, uint remove);
    [DllImport("user32.dll")] internal static extern nint DispatchMessageW(in Message message);
    [DllImport("user32.dll", SetLastError = true)] internal static extern bool PostMessageW(nint window, uint message, nuint wParam, nint lParam);
}
