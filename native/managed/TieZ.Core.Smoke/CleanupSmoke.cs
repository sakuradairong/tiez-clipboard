using TieZ.Core;

internal static class CleanupSmoke
{
    internal static void Run(NativeCore core)
    {
        ulong revision = core.Snapshot.Revision;
        int changed = 0;
        void Changed(SettingsSnapshot _) => changed++;
        core.Changed += Changed;
        try
        {
            var result = core.PreviewCleanupAsync("(?P<word>foo) => ${word}bar", "  foo\r\n ").GetAwaiter().GetResult();
            if (result.Text != "  foobar\n " || result.Ignored || result.Rules != 1 || result.Matches != 1
                || result.ToString().Contains("foobar", StringComparison.Ordinal)) throw new Exception("Cleanup preview semantics");
            var ignored = core.PreviewCleanupAsync("foo => __IGNORE_CAPTURE__", "foo").GetAwaiter().GetResult();
            if (!ignored.Ignored) throw new Exception("Cleanup preview ignore result");
            try { core.PreviewCleanupAsync("[ => broken", "private sample").GetAwaiter().GetResult(); throw new Exception("Invalid rule accepted"); }
            catch (CoreException e) when (e.Code == "invalid_rule")
            { if (e.Message.Contains("private", StringComparison.Ordinal)) throw new Exception("Sample in error"); }
            try { core.PreviewCleanupAsync("x => y", new string('x', 17000)).GetAwaiter().GetResult(); throw new Exception("Unbounded sample accepted"); }
            catch (CoreException e) when (e.Code == "preview_limit") { }
            using var cancelled = new CancellationTokenSource(); cancelled.Cancel();
            try { core.PreviewCleanupAsync("", "", cancelled.Token).GetAwaiter().GetResult(); throw new Exception("Cancelled preview accepted"); }
            catch (OperationCanceledException) { }
            if (core.Refresh().Revision != revision || changed != 0) throw new Exception("Preview wrote settings");
        }
        finally { core.Changed -= Changed; }
    }
}
