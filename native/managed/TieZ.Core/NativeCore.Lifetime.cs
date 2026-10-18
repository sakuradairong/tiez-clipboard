namespace TieZ.Core;

public sealed partial class NativeCore
{
    private Task? pendingClose;

    // Must be initiated by the owner. All waits/native stop/join run without its dispatcher.
    // Duplicate requests borrow the same task; a failed close retains ownership for retry.
    public ValueTask DisposeAsync()
    {
        VerifyThread();
        if (handle == 0) return ValueTask.CompletedTask;
        if (pendingClose is { IsCompleted: false }) return new ValueTask(pendingClose);
        shutdown.Cancel();
        pendingClose = CloseAsync();
        return new ValueTask(pendingClose);
    }

    private async Task CloseAsync()
    {
        await appsGate.WaitAsync().ConfigureAwait(false);
        try
        {
            await openGate.WaitAsync().ConfigureAwait(false);
            try
            {
                await requestGate.WaitAsync().ConfigureAwait(false);
                try
                {
                    // Shutdown cancellation is for queued application work, not the close itself.
                    await Task.Run(() => Request(new { op = "close", handle })).ConfigureAwait(false);
                    handle = 0;
                    Changed = null;
                    GC.SuppressFinalize(this);
                }
                finally { requestGate.Release(); }
            }
            finally { openGate.Release(); }
        }
        finally { appsGate.Release(); }
    }
}
