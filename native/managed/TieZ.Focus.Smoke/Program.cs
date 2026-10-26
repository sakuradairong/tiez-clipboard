using TieZ.Core;

if (args.Contains("--cleanup-only")) { FixturePaths.CheckRules(); return; }

if (args.Contains("--fixture-target")) { FixtureChild.RunTarget(); return; }
static T Read<T>(Task<T> request)
{
    // Keep this owned UI thread pumping while the native worker restores focus.
    while (!request.IsCompleted) { FixtureWindow.Pump(); Thread.Sleep(5); }
    return request.GetAwaiter().GetResult();
}
static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
static TargetTrackingStatus Wait(NativeCore core, Func<TargetTrackingStatus, bool> ready)
{
    var deadline = DateTime.UtcNow.AddSeconds(5);
    while (DateTime.UtcNow < deadline)
    {
        var status = Read(core.GetTargetStatusAsync());
        if (ready(status)) return status;
        if (status.LastError is not null) throw new InvalidOperationException(status.LastError);
        FixtureWindow.Pump(); Thread.Sleep(20);
    }
    throw new InvalidOperationException("Native target tracking did not settle");
}
string temporaryRoot = new DirectoryInfo(Path.GetTempPath()).FullName;
string directory = Path.Combine(temporaryRoot, $"TieZ-focus-{Guid.NewGuid():N}");
Directory.CreateDirectory(directory);
nint original = FocusApi.GetForegroundWindow();
FocusApi.GetWindowThreadProcessId(original, out uint originalProcess);
try
{
    using var own = new FixtureWindow("TieZ focus fixture controller");
    using var child = new FixtureChild();
    using var core = new NativeCore(Path.Combine(directory, "fixture.db"));
    try
    {
        FixtureWindow.Activate(own.Handle);
        Check(Read(core.StartTargetTrackingAsync()).Running, "Start target tracker");
        FixtureWindow.Activate(child.Windows[0]);
        var first = Wait(core, status => status.HasTarget && status.Application == "TieZ.Focus.Smoke.exe");
        FixtureWindow.Activate(own.Handle);
        Thread.Sleep(150);
        var ownExcluded = Read(core.GetTargetStatusAsync());
        Check(ownExcluded.Generation == first.Generation, "Own window replaced external target");
        var restored = Read(core.RestoreTargetFocusAsync(first.Generation));
        Check(restored.Restored && FocusApi.GetForegroundWindow() == child.Windows[0], "Native restore missed controlled target A");
        Check(!Read(core.RestoreTargetFocusAsync(first.Generation)).Restored, "Already-focused target was needlessly focused again");
        FixtureWindow.Activate(child.Windows[1]);
        var second = Wait(core, status => status.Generation > first.Generation);
        FixtureWindow.Activate(own.Handle);
        try { Read(core.RestoreTargetFocusAsync(first.Generation)); throw new InvalidOperationException("Stale target accepted"); }
        catch (CoreException error) when (error.Code == "target_changed") { }
        Check(FocusApi.GetForegroundWindow() == own.Handle, "Stale request moved foreground");
        Read(core.RestoreTargetFocusAsync(second.Generation));
        Check(FocusApi.GetForegroundWindow() == child.Windows[1], "Native restore missed controlled target B");
        FixtureWindow.Activate(own.Handle);
        child.Close(child.Windows[1]);
        Wait(core, status => !status.HasTarget);
        try { Read(core.RestoreTargetFocusAsync(second.Generation)); throw new InvalidOperationException("Closed target accepted"); }
        catch (CoreException error) when (error.Code is "target_unavailable" or "target_changed" or "target_missing") { }
        Check(FocusApi.GetForegroundWindow() == own.Handle, "Closed target changed foreground");
        Check(!Read(core.StopTargetTrackingAsync()).Running, "Stop target hook");
        Check(Read(core.StartTargetTrackingAsync()).Starts == 2, "Restart target hook");
        Console.WriteLine("PASS: real foreground hook, own-process exclusion, native cross-process focus restore, no-op focus, stale/closed-target rejection, stop/restart; no clipboard or keyboard injection");
    }
    finally
    {
        Read(core.StopTargetTrackingAsync());
        // Restore prior foreground only if the test still owns foreground and the
        // original window still belongs to its original process. Never steal it
        // back if the user has moved to another application during this test.
        nint current = FocusApi.GetForegroundWindow();
        FocusApi.GetWindowThreadProcessId(original, out uint process);
        if ((current == own.Handle || child.Windows.Contains(current)) && FocusApi.IsWindow(original) && process == originalProcess)
            FocusApi.SetForegroundWindow(original);
    }
}
catch (FixtureForegroundUnavailable)
{
    Console.WriteLine("UNAVAILABLE: Windows refused controlled foreground activation; native cross-process focus acceptance is incomplete. No clipboard or keyboard injection was used.");
    Environment.ExitCode = 77;
}
finally
{
    string resolved = Path.GetFullPath(directory);
    if (!FixturePaths.CanDelete(resolved, temporaryRoot))
        throw new InvalidOperationException("Fixture cleanup target escaped its temporary root");
    Directory.Delete(resolved, recursive: true);
}
