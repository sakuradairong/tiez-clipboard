namespace TieZ.Core;

public sealed record ShellStatus(bool Running, ulong Starts, bool TrayVisible, string? LastError);
public sealed record ShellEventBatch(ShellStatus Status, IReadOnlyList<string> Actions);

public sealed partial class NativeCore
{
    public Task<ShellStatus> StartShellAsync(bool showTray = true, CancellationToken cancellationToken = default) =>
        HistoryRequestAsync<ShellStatus>(new { op = "shell_start", handle, show_tray = showTray }, cancellationToken);
    public Task<ShellEventBatch> PollShellAsync(CancellationToken cancellationToken = default) =>
        HistoryRequestAsync<ShellEventBatch>(new { op = "shell_poll", handle }, cancellationToken);
    public Task<ShellStatus> StopShellAsync(CancellationToken cancellationToken = default) =>
        HistoryRequestAsync<ShellStatus>(new { op = "shell_stop", handle }, cancellationToken);
}
