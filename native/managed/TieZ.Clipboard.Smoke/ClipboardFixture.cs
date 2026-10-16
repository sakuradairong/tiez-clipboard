using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Text;

// A process-private window station has a different OS clipboard. Never switch the
// displayed desktop, send input, or inspect the user's interactive clipboard.
internal sealed class ClipboardIsolationUnavailable(int nativeError) : Exception("Private window station unavailable")
{
    internal int NativeError { get; } = nativeError;
}

internal sealed class IsolatedDesktop : IDisposable
{
    private readonly nint previousStation = Api.GetProcessWindowStation();
    private readonly nint previousDesktop = Api.GetThreadDesktop(Api.GetCurrentThreadId());
    private nint station, desktop;
    internal IsolatedDesktop()
    {
        try
        {
            string name = $"TieZClipboardFixture-{Environment.ProcessId}-{Guid.NewGuid():N}";
            station = Api.CreateWindowStationW(name, 0, 0x0000003F, 0);
            if (station == 0) throw new ClipboardIsolationUnavailable(Marshal.GetLastWin32Error());
            Api.Require(Api.SetProcessWindowStation(station), "SetProcessWindowStation");
            var actual = new StringBuilder(256);
            Api.Require(Api.GetUserObjectInformationW(Api.GetProcessWindowStation(), 2, actual, 512, out _), "Read fixture station name");
            if (actual.ToString() != name) throw new InvalidOperationException("Clipboard isolation could not be verified.");
            desktop = Api.CreateDesktopW("Default", null, 0, 0, 0x000000C3, 0);
            Api.Require(desktop != 0, "CreateDesktop");
            Api.Require(Api.SetThreadDesktop(desktop), "SetThreadDesktop");
        }
        catch { Dispose(); throw; }
    }
    public void Dispose()
    {
        bool restoredStation = station == 0 || Api.SetProcessWindowStation(previousStation);
        bool restoredDesktop = desktop == 0 || Api.SetThreadDesktop(previousDesktop);
        bool closedDesktop = desktop == 0 || Api.CloseDesktop(desktop);
        bool closedStation = station == 0 || Api.CloseWindowStation(station);
        desktop = station = 0;
        if (!restoredDesktop || !restoredStation || !closedDesktop || !closedStation)
            throw new InvalidOperationException("Fixture window-station cleanup failed.");
    }
}
internal sealed class ClipboardFixture : IDisposable
{
    private readonly nint window;
    internal ClipboardFixture()
    {
        window = Api.CreateWindowExW(0, "STATIC", "TieZ isolated clipboard fixture", 0, 0, 0, 0, 0, new nint(-3), 0, 0, 0);
        Api.Require(window != 0, "Create clipboard owner");
    }
    internal static uint Format(string name)
    {
        uint id = Api.RegisterClipboardFormatW(name);
        Api.Require(id != 0, "Register clipboard format");
        return id;
    }
    internal void Write(params (uint Format, byte[] Bytes)[] values)
    {
        Open();
        try
        {
            Api.Require(Api.EmptyClipboard(), "Empty fixture clipboard");
            foreach (var (format, bytes) in values)
            {
                nint memory = Api.GlobalAlloc(0x42, new nuint((uint)bytes.Length));
                Api.Require(memory != 0, "Allocate clipboard payload");
                try
                {
                    nint data = Api.GlobalLock(memory);
                    Api.Require(data != 0, "Lock clipboard payload");
                    try { Marshal.Copy(bytes, 0, data, bytes.Length); }
                    finally { Api.GlobalUnlock(memory); }
                    Api.Require(Api.SetClipboardData(format, memory) != 0, "Set fixture format");
                    memory = 0; // OS owns this allocation after success.
                }
                finally { if (memory != 0) Api.GlobalFree(memory); }
            }
        }
        finally { Api.CloseClipboard(); }
    }
    private void Open()
    {
        for (int attempt = 0; attempt < 50; attempt++)
        {
            if (Api.OpenClipboard(window)) return;
            Thread.Sleep(10);
        }
        Api.Require(false, "Open fixture clipboard");
    }
    internal void Text(string value) => Write((13, Encoding.Unicode.GetBytes(value + "\0")));
    internal byte[]? Read(uint format)
    {
        Open();
        try
        {
            if (!Api.IsClipboardFormatAvailable(format)) return null;
            nint memory = Api.GetClipboardData(format);
            Api.Require(memory != 0, "Get fixture format");
            ulong length = Api.GlobalSize(memory).ToUInt64();
            if (length == 0 || length > 32 * 1024 * 1024) throw new InvalidOperationException("Invalid fixture allocation size.");
            nint data = Api.GlobalLock(memory);
            Api.Require(data != 0, "Lock fixture format");
            try { byte[] result = new byte[(int)length]; Marshal.Copy(data, result, 0, result.Length); return result; }
            finally { Api.GlobalUnlock(memory); }
        }
        finally { Api.CloseClipboard(); }
    }
    internal string ReadText() => Encoding.Unicode.GetString(Read(13) ?? throw new InvalidOperationException("Missing Unicode text")).TrimEnd('\0');
    internal static byte[] DropFiles(IEnumerable<string> paths)
    {
        byte[] data = Encoding.Unicode.GetBytes(string.Join('\0', paths) + "\0\0");
        byte[] result = new byte[20 + data.Length];
        BitConverter.GetBytes(20).CopyTo(result, 0);
        BitConverter.GetBytes(1).CopyTo(result, 16);
        data.CopyTo(result, 20); return result;
    }
    internal string[] ReadFiles()
    {
        byte[] bytes = Read(15) ?? throw new InvalidOperationException("Missing CF_HDROP");
        if (bytes.Length < 20 || BitConverter.ToUInt32(bytes, 16) != 1) throw new InvalidOperationException("Invalid wide CF_HDROP");
        return Encoding.Unicode.GetString(bytes, checked((int)BitConverter.ToUInt32(bytes, 0)), bytes.Length - checked((int)BitConverter.ToUInt32(bytes, 0)))
            .Split('\0', StringSplitOptions.RemoveEmptyEntries);
    }
    internal static byte[] Dib()
    {
        byte[] bytes = new byte[48];
        BitConverter.GetBytes(40).CopyTo(bytes, 0); BitConverter.GetBytes(2).CopyTo(bytes, 4);
        BitConverter.GetBytes(1).CopyTo(bytes, 8); BitConverter.GetBytes((short)1).CopyTo(bytes, 12);
        BitConverter.GetBytes((short)32).CopyTo(bytes, 14); BitConverter.GetBytes(8).CopyTo(bytes, 20);
        new byte[] { 11, 22, 33, 255, 44, 55, 66, 255 }.CopyTo(bytes, 40); return bytes;
    }
    public void Dispose() => Api.Require(Api.DestroyWindow(window), "Destroy fixture owner");
}
internal static class Api
{
    internal static void Require(bool value, string operation)
    {
        if (!value) throw new Win32Exception(Marshal.GetLastWin32Error(), operation);
    }
    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)] internal static extern nint CreateWindowStationW(string name, uint flags, uint access, nint security);
    [DllImport("user32.dll", SetLastError = true)] internal static extern nint GetProcessWindowStation();
    [DllImport("user32.dll", SetLastError = true)] internal static extern bool SetProcessWindowStation(nint station);
    [DllImport("user32.dll", SetLastError = true)] internal static extern bool CloseWindowStation(nint station);
    [DllImport("kernel32.dll")] internal static extern uint GetCurrentThreadId();
    [DllImport("user32.dll")] internal static extern nint GetThreadDesktop(uint thread);
    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)] internal static extern nint CreateDesktopW(string name, string? device, nint mode, uint flags, uint access, nint security);
    [DllImport("user32.dll", SetLastError = true)] internal static extern bool SetThreadDesktop(nint desktop);
    [DllImport("user32.dll", SetLastError = true)] internal static extern bool CloseDesktop(nint desktop);
    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)] internal static extern bool GetUserObjectInformationW(nint handle, int index, StringBuilder information, int length, out int needed);
    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)] internal static extern nint CreateWindowExW(uint ex, string cls, string name, uint style, int x, int y, int width, int height, nint parent, nint menu, nint instance, nint param);
    [DllImport("user32.dll", SetLastError = true)] internal static extern bool DestroyWindow(nint window);
    [DllImport("user32.dll", SetLastError = true)] internal static extern bool OpenClipboard(nint owner);
    [DllImport("user32.dll", SetLastError = true)] internal static extern bool CloseClipboard();
    [DllImport("user32.dll", SetLastError = true)] internal static extern bool EmptyClipboard();
    [DllImport("user32.dll", SetLastError = true)] internal static extern nint SetClipboardData(uint format, nint memory);
    [DllImport("user32.dll", SetLastError = true)] internal static extern nint GetClipboardData(uint format);
    [DllImport("user32.dll", SetLastError = true)] internal static extern bool IsClipboardFormatAvailable(uint format);
    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)] internal static extern uint RegisterClipboardFormatW(string name);
    [DllImport("kernel32.dll", SetLastError = true)] internal static extern nint GlobalAlloc(uint flags, nuint size);
    [DllImport("kernel32.dll", SetLastError = true)] internal static extern nint GlobalLock(nint memory);
    [DllImport("kernel32.dll")] internal static extern bool GlobalUnlock(nint memory);
    [DllImport("kernel32.dll")] internal static extern nint GlobalFree(nint memory);
    [DllImport("kernel32.dll")] internal static extern nuint GlobalSize(nint memory);
}
