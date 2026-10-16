using System.Collections.ObjectModel;
using System.Runtime.InteropServices;
using System.Text.Json;

namespace TieZ.Core;

public sealed record SettingsSnapshot(ulong Revision, IReadOnlyDictionary<string, string> Values);

public sealed class CoreException(string code, string message) : Exception(message)
{
    public string Code { get; } = code;
}

// Owned by the application's UI thread. Views borrow this client; only the app disposes it.
public sealed class NativeCore : IDisposable
{
    private readonly int ownerThread = Environment.CurrentManagedThreadId;
    private ulong handle;
    public SettingsSnapshot Snapshot { get; private set; }
    public event Action<SettingsSnapshot>? Changed;

    public NativeCore(string databasePath)
    {
        if (!Path.IsPathFullyQualified(databasePath))
            throw new ArgumentException("An absolute isolated database path is required.", nameof(databasePath));
        if (NativeMethods.Version() != 1)
            throw new NotSupportedException("Unsupported native core ABI.");
        var result = Request(new { op = "open", database_path = databasePath });
        handle = result.GetProperty("handle").GetUInt64();
        try { Snapshot = ReadSnapshot(result); }
        catch { Request(new { op = "close", handle }); handle = 0; throw; }
    }

    public bool Update(IReadOnlyDictionary<string, string> patch, ulong expectedRevision)
    {
        VerifyAccess();
        var result = Request(new { op = "update", handle, expected_revision = expectedRevision, patch });
        Snapshot = ReadSnapshot(result);
        bool changed = result.GetProperty("changed").GetBoolean();
        if (changed) Changed?.Invoke(Snapshot);
        return changed;
    }

    public SettingsSnapshot Refresh()
    {
        VerifyAccess();
        var snapshot = ReadSnapshot(Request(new { op = "snapshot", handle }));
        bool changed = snapshot.Revision != Snapshot.Revision;
        Snapshot = snapshot;
        if (changed) Changed?.Invoke(snapshot);
        return snapshot;
    }

    public void Dispose()
    {
        VerifyThread();
        if (handle == 0) return;
        Request(new { op = "close", handle });
        handle = 0;
        Changed = null;
        GC.SuppressFinalize(this);
    }

    private void VerifyThread()
    {
        if (Environment.CurrentManagedThreadId != ownerThread)
            throw new InvalidOperationException("Use the core on its owning UI thread.");
    }

    private void VerifyAccess()
    {
        VerifyThread();
        ObjectDisposedException.ThrowIf(handle == 0, this);
    }

    private static SettingsSnapshot ReadSnapshot(JsonElement result)
    {
        var snapshot = result.GetProperty("snapshot");
        var values = snapshot.GetProperty("values").EnumerateObject()
            .ToDictionary(p => p.Name, p => p.Value.GetString()!);
        return new(snapshot.GetProperty("revision").GetUInt64(),
            new ReadOnlyDictionary<string, string>(values));
    }

    private static JsonElement Request(object request)
    {
        byte[] bytes = JsonSerializer.SerializeToUtf8Bytes(request);
        if (bytes.Length > 65536) throw new ArgumentException("Native request exceeds 64 KiB.");
        nint response = NativeMethods.Request(bytes, (nuint)bytes.Length);
        if (response == 0) throw new CoreException("internal", "Native core returned no response.");
        try
        {
            string json = Marshal.PtrToStringUTF8(response)
                ?? throw new CoreException("internal", "Native response is not UTF-8.");
            using var document = JsonDocument.Parse(json);
            var root = document.RootElement;
            if (!root.GetProperty("ok").GetBoolean())
            {
                var error = root.GetProperty("error");
                throw new CoreException(error.GetProperty("code").GetString()!,
                    error.GetProperty("message").GetString()!);
            }
            return root.GetProperty("result").Clone();
        }
        finally { NativeMethods.Free(response); }
    }

    private static class NativeMethods
    {
        [DllImport("tiez_native_core", EntryPoint = "tiez_core_abi_version", CallingConvention = CallingConvention.Cdecl)]
        internal static extern uint Version();
        [DllImport("tiez_native_core", EntryPoint = "tiez_core_request", CallingConvention = CallingConvention.Cdecl)]
        internal static extern nint Request([In] byte[] bytes, nuint length);
        [DllImport("tiez_native_core", EntryPoint = "tiez_core_free", CallingConvention = CallingConvention.Cdecl)]
        internal static extern void Free(nint response);
    }
}
