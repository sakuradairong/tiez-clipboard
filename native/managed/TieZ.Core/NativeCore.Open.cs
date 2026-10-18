using System.Text.Json;

namespace TieZ.Core;

public sealed record OpenContentResult(long Id, ulong HistoryRevision, string Route, bool Exported,
    bool EditWatchRequired, bool EditWritebackAvailable, bool UsageUpdated);
public sealed record OpenCleanupFailure(string Code, string Message);
public sealed record OpenContentStatus(int ActiveExports, long ExportBytes, bool Closed, OpenCleanupFailure? CleanupError);

public sealed partial class NativeCore
{
    // Native open releases the registry/history locks. Keep its potentially slow Windows
    // handler dispatch separate from the ordinary history/settings queue as well.
    private readonly SemaphoreSlim openGate = new(1, 1);
    public bool IsClosing => handle != 0 && shutdown.IsCancellationRequested;

    public Task<OpenContentResult> OpenContentAsync(long id, ulong expectedRevision, bool allowSensitive = false,
        CancellationToken cancellationToken = default) =>
        OpenRequestAsync<OpenContentResult>(new { op = "content_open", handle, id,
            expected_revision = expectedRevision, allow_sensitive = allowSensitive }, cancellationToken);

    // Poll performs cooperative export expiry/failed-close cleanup. It never saves settings.
    public Task<OpenContentStatus> PollOpenContentAsync(CancellationToken cancellationToken = default) =>
        OpenRequestAsync<OpenContentStatus>(new { op = "content_open_poll", handle }, cancellationToken);

    private async Task<T> OpenRequestAsync<T>(object request, CancellationToken cancellationToken)
    {
        VerifyAccess();
        byte[] bytes = SerializeRequest(request);
        using var cancellation = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken, shutdown.Token);
        await openGate.WaitAsync(cancellation.Token).ConfigureAwait(false);
        try
        {
            ObjectDisposedException.ThrowIf(handle == 0, this);
            return await Task.Run(() =>
            {
                cancellation.Token.ThrowIfCancellationRequested();
                // Once submitted, preserve Rust's actual outcome even if cancellation follows.
                return RequestBytes(bytes).Deserialize<T>(WireJson)
                    ?? throw new CoreException("invalid_response", "Native core returned an empty result.");
            }).ConfigureAwait(false);
        }
        finally { openGate.Release(); }
    }
}
