using Microsoft.UI.Dispatching;
using TieZ.Core;

namespace TieZ.Native;

// UI dispatch is independent from history loading, selection and dialogs.
internal sealed class ShellController : IDisposable
{
    private readonly MainWindow main;
    private readonly NativeCore core;
    private readonly DispatcherQueueTimer timer;
    private readonly CancellationTokenSource lifetime = new();
    private bool disposed, polling;
    internal ShellStatus? Status { get; private set; }

    internal ShellController(MainWindow main, NativeCore core)
    {
        this.main = main; this.core = core;
        timer = main.DispatcherQueue.CreateTimer();
        timer.Interval = TimeSpan.FromMilliseconds(200);
        timer.Tick += Poll;
    }
    internal async Task StartAsync()
    {
        try
        {
            var status = await core.StartShellAsync(cancellationToken: lifetime.Token);
            if (disposed) return;
            Apply(status); timer.Start();
        }
        catch (OperationCanceledException) { }
        catch (Exception error) { if (!disposed) main.SetShellUnavailable(error.Message); }
    }
    private void Apply(ShellStatus status)
    {
        // If Explorer recovery failed while hidden, reveal the still-running app.
        if (Status?.TrayVisible == true && !status.TrayVisible) main.ShowMain();
        Status = status;
        main.SetResidentMode(status.Running && status.TrayVisible);
        if (status.LastError is not null) main.SetShellUnavailable(status.LastError);
    }
    private async void Poll(DispatcherQueueTimer sender, object args)
    {
        if (disposed || polling) return;
        polling = true;
        try
        {
            var batch = await core.PollShellAsync(lifetime.Token);
            if (disposed) return;
            Apply(batch.Status);
            foreach (string action in batch.Actions)
            {
                if (disposed) break;
                switch (action)
                {
                    case "show_main": main.ShowMain(); break;
                    case "open_settings": main.OpenSettings(); break;
                    case "quit": main.Quit(); return;
                }
            }
        }
        catch (OperationCanceledException) { }
        catch (ObjectDisposedException) when (disposed) { }
        catch (Exception error)
        {
            if (!disposed)
            {
                main.ShowMain(); main.SetShellUnavailable(error.Message);
                main.SetResidentMode(false); timer.Stop();
            }
        }
        finally { polling = false; }
    }
    public void Dispose()
    {
        if (disposed) return;
        disposed = true; timer.Stop(); timer.Tick -= Poll;
        lifetime.Cancel(); lifetime.Dispose();
        // Core.Dispose owns native stop/join, including a menu that is open.
    }
}
