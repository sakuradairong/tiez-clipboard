using TieZ.Core;

internal static class LayoutPreferenceSmoke
{
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    private static void Expect(string code, Action action)
    {
        try { action(); } catch (CoreException error) when (error.Code == code) { return; }
        throw new InvalidOperationException($"Expected {code}");
    }
    internal static void Run(string database)
    {
        using (var core = new NativeCore(database))
        {
            int events = 0; core.Changed += _ => events++;
            Check(core.Refresh().Revision == 0 && core.Snapshot.Values["app.history_view_mode"] == "list"
                && core.Snapshot.Values["app.tag_history_view_mode"] == "grid" && events == 0, "Layout defaults wrote/notified");
            Check(!core.Update(new Dictionary<string,string> { ["app.history_view_mode"] = "list" }, 0) && events == 0, "Layout no-op saved");
            Expect("invalid_value", () => core.Update(new Dictionary<string,string> { ["app.tag_history_view_mode"] = "card" }, 0));
            Check(core.Update(new Dictionary<string,string> { ["app.history_view_mode"] = "grid" }, 0), "Ordinary layout save");
            Expect("revision_conflict", () => core.Update(new Dictionary<string,string> { ["app.tag_history_view_mode"] = "list" }, 0));
            Check(core.Snapshot.Revision == 1 && core.Snapshot.Values["app.tag_history_view_mode"] == "grid" && events == 1, "Layout conflict changed state");
            Check(core.Update(new Dictionary<string,string> { ["app.tag_history_view_mode"] = "list" }, 1) && events == 2, "Tag layout save");
            Check(core.Snapshot.Values["app.compact_mode"] == "false", "Layout changed compact preference");
        }
        using (var reopened = new NativeCore(database))
        {
            Check(reopened.Snapshot.Revision == 2 && reopened.Snapshot.Values["app.history_view_mode"] == "grid"
                && reopened.Snapshot.Values["app.tag_history_view_mode"] == "list", "Layout reopen persistence");
            Check(reopened.Refresh().Revision == 2, "Layout reread wrote");
        }
        Console.WriteLine("PASS: real DLL independent history/tag layouts, readonly defaults, validation, no-op/conflict notifications and reopen persistence");
    }
}
