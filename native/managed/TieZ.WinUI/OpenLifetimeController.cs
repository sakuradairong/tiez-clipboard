using Microsoft.UI.Dispatching;
using TieZ.Core;

namespace TieZ.Native;

// One application-owner timer; it continues while hidden and never belongs to settings windows.
internal sealed class OpenLifetimeController : IDisposable
{
    private readonly MainWindow main;
    private readonly NativeCore core;
    private readonly DispatcherQueueTimer timer;
    private readonly CancellationTokenSource lifetime = new();
    private Task? polling;
    private bool disposed;
    internal bool IsStopped => disposed;
    internal int CompletedPolls { get; private set; }
    internal OpenContentStatus? Status { get; private set; }
    internal OpenLifetimeController(MainWindow main, NativeCore core)
    {
        this.main = main; this.core = core;
        timer = main.DispatcherQueue.CreateTimer(); timer.Interval = TimeSpan.FromSeconds(30);
        timer.Tick += Tick; timer.Start();
    }
    private async void Tick(DispatcherQueueTimer sender, object args) => await PollNowAsync();
    internal Task PollNowAsync()
    {
        if (disposed) return Task.CompletedTask;
        if (polling is { IsCompleted: false }) return polling;
        polling = PollAsync(); return polling;
    }
    private async Task PollAsync()
    {
        try
        {
            var status = await core.PollOpenContentAsync(lifetime.Token);
            if (disposed) return;
            Status = status; CompletedPolls++;
            main.SetOpenCleanupWarning(status.CleanupError?.Code);
        }
        catch (OperationCanceledException) { }
        catch (ObjectDisposedException) when (disposed || core.IsClosing) { }
        catch (Exception) { if (!disposed) main.SetOpenCleanupWarning("open_poll"); }
    }
    public void Dispose()
    {
        if (disposed) return;
        disposed = true; timer.Stop(); timer.Tick -= Tick; lifetime.Cancel(); lifetime.Dispose(); Status = null;
    }
}
