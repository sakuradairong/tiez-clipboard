using TieZ.Core;

internal static class TargetSmoke
{
    private static T Read<T>(Task<T> task) => task.GetAwaiter().GetResult();
    private static void Check(bool value, string message)
    {
        if (!value) throw new InvalidOperationException(message);
    }
    internal static void Run(NativeCore core)
    {
        var initial = Read(core.GetTargetStatusAsync());
        Check(!initial.Running && initial.Starts == 0, "Target tracking must be explicit");
        var started = Read(core.StartTargetTrackingAsync());
        Check(started.Running && started.Starts == 1, "Target hook start");
        Check(Read(core.StartTargetTrackingAsync()).Starts == 1, "Idempotent target start");
        try { Read(core.RestoreTargetFocusAsync(ulong.MaxValue)); throw new InvalidOperationException("Stale focus request accepted"); }
        catch (CoreException error) when (error.Code == "target_changed") { }
        try { Read(core.PasteEntryAsync(99999, ulong.MaxValue)); throw new InvalidOperationException("Stale paste request accepted"); }
        catch (CoreException error) when (error.Code == "target_changed") { }
        var stopped = Read(core.StopTargetTrackingAsync());
        Check(!stopped.Running && !stopped.HasTarget && stopped.Generation > started.Generation, "Target stop/join and ticket invalidation");
        try { Read(core.PasteEntryAsync(99999, stopped.Generation)); throw new InvalidOperationException("Stopped tracker accepted paste"); }
        catch (CoreException error) when (error.Code == "target_tracker_stopped") { }
        Check(!Read(core.StopTargetTrackingAsync()).Running, "Idempotent target stop");
        Check(Read(core.StartTargetTrackingAsync()).Starts == 2, "Target hook restart");
    }
    internal static void DisposeActive(string database)
    {
        using (var active = new NativeCore(database)) { Read(active.StartTargetTrackingAsync()); }
        using var reopened = new NativeCore(database);
        Check(Read(reopened.GetTargetStatusAsync()).Starts == 0, "Fresh tracker after core disposal");
        Check(Read(reopened.StartTargetTrackingAsync()).Running, "Target hook reusable after disposal");
    }
}
