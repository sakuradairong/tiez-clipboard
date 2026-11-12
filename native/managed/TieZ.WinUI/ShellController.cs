using Microsoft.UI.Dispatching;
using TieZ.Core;

namespace TieZ.Native;

// UI dispatch is independent from history loading, selection and dialogs.
internal sealed class ShellController : IDisposable
{
    private readonly MainWindow main;
    private readonly NativeCore core;
    private readonly ClipboardPopupWindow? popup;
    private readonly DispatcherQueueTimer timer;
    private readonly CancellationTokenSource lifetime = new();
    private bool disposed, polling, stopped;
    internal ShellStatus? Status { get; private set; }

    internal ShellController(MainWindow main, NativeCore core, ClipboardPopupWindow? popup = null)
    {
        this.main = main; this.core = core; this.popup = popup;
        main.ShutdownStarted += Dispose;
        LanguageService.Changed += ApplyLanguage;
        timer = main.DispatcherQueue.CreateTimer();
        timer.Interval = TimeSpan.FromMilliseconds(200);
        timer.Tick += Poll;
    }
    internal async Task StartAsync(bool bindMainHotkey = true)
    {
        try
        {
            var status = await core.StartShellAsync(bindMainHotkey: bindMainHotkey, cancellationToken: lifetime.Token);
            if (disposed) return;
            Apply(status); timer.Start();
        }
        catch (OperationCanceledException) { }
        catch (Exception error) { if (!disposed) { stopped = true; main.SetShellUnavailable(error.Message); } }
    }
    private void Apply(ShellStatus status)
    {
        // If Explorer recovery failed while hidden, reveal the still-running app.
        if (Status?.TrayVisible == true && !status.TrayVisible) main.ShowMain();
        Status = status;
        main.SetResidentMode(status.Running && status.TrayVisible);
        Publish();
    }
    private string? Warning(ShellStatus status)
    {
        var text = ShellStrings.Current;
        if (status.LastError is not null) return status.LastError;
        if (status.HotkeyError is null) return null;
        return status.HotkeyError == "hotkey_conflict"
            ? status.MainHotkeyActive ? text.HotkeyPending : text.HotkeyOccupied
            : string.Format(text.HotkeyFailureFormat, status.HotkeyError);
    }
    private void Publish()
    {
        if (disposed || stopped || Status is not { } status) return;
        string? message = Warning(status);
        if (message is not null) main.SetShellUnavailable(message); else main.ClearShellWarning();
        popup?.SetShellMessage(message);
    }
    // The accepted language changed; re-derive only the shell warning already on screen.
    private void ApplyLanguage() => Publish();
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
                    case "show_main": if (popup is not null) await popup.ShowAsync(); else main.ShowMain(); break;
                    case "toggle_main": if (popup is not null) await popup.ToggleAsync(); else main.ToggleMain(); break;
                    case "open_settings": main.OpenSettings(); break;
                    case "quit": _ = main.Quit(); return;
                    default:
                        if (action.StartsWith("quick_paste:", StringComparison.Ordinal)
                            && byte.TryParse(action.AsSpan("quick_paste:".Length), out byte index))
                        {
                            await PastePinnedAsync(index);
                        }
                        break;
                }
            }
        }
        catch (OperationCanceledException) { }
        catch (ObjectDisposedException) when (disposed) { }
        catch (Exception error)
        {
            if (!disposed)
            {
                stopped = true;
                main.ShowMain(); main.SetShellUnavailable(error.Message);
                main.SetResidentMode(false); timer.Stop();
            }
        }
        finally { polling = false; }
    }
    private async Task PastePinnedAsync(byte index)
    {
        try
        {
            var result = await core.QuickPasteAsync(index, lifetime.Token);
            if (disposed || result.Pasted || popup is null) return;
            popup.SetMessage(result.Reason == "not_pinned" ? PopupStrings.NotPinned : PopupStrings.Failed);
        }
        catch (OperationCanceledException) { }
        catch (ObjectDisposedException) when (disposed) { }
        catch (CoreException error)
        {
            if (disposed) return;
            string message = error.Code == "no_target" ? PopupStrings.NoTarget : PopupStrings.Failed;
            if (popup is not null) popup.SetMessage(message);
            else main.SetShellUnavailable(message);
        }
    }
    public void Dispose()
    {
        if (disposed) return;
        disposed = true; main.ShutdownStarted -= Dispose; LanguageService.Changed -= ApplyLanguage;
        timer.Stop(); timer.Tick -= Poll;
        lifetime.Cancel(); lifetime.Dispose();
        // Core.Dispose owns native stop/join, including a menu that is open.
    }
}
