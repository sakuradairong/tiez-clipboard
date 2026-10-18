using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Text;
using System.Text.Json;
using Microsoft.UI.Xaml;
using TieZ.Core;

namespace TieZ.Native;

// Real settings window and DLL; file-selection responses use owned fixtures except native cancellation.
internal sealed class OpenerSettingsWindowSmoke(MainWindow main, NativeCore core, string directory, string report, string fixtureSource)
{
    private readonly Stopwatch elapsed = Stopwatch.StartNew();
    private int checks, pickerResponses, launches;
    private bool finished, nativePickerCancelled;
    private string? exported;
    private static readonly string[] Keys = ["app.text", "app.code", "app.rich_text", "app.image", "app.file", "app.video", "app.url", "app.link"];
    [DllImport("winsqlite3.dll", CallingConvention = CallingConvention.Cdecl)]
    private static extern int sqlite3_open_v2([MarshalAs(UnmanagedType.LPUTF8Str)] string path, out nint database, int flags, nint vfs);
    [DllImport("winsqlite3.dll", CallingConvention = CallingConvention.Cdecl)]
    private static extern int sqlite3_exec(nint database, [MarshalAs(UnmanagedType.LPUTF8Str)] string sql, nint callback, nint argument, out nint error);
    [DllImport("winsqlite3.dll", CallingConvention = CallingConvention.Cdecl)] private static extern void sqlite3_free(nint value);
    [DllImport("winsqlite3.dll", CallingConvention = CallingConvention.Cdecl)] private static extern int sqlite3_close(nint database);
    private void Check(bool condition, string message) { checks++; if (!condition) throw new InvalidOperationException(message); }
    private async Task WaitUntil(Func<bool> ready, string message)
    {
        var watch = Stopwatch.StartNew();
        while (!ready() && watch.Elapsed < TimeSpan.FromSeconds(8)) await Task.Delay(20);
        Check(ready(), message);
    }
    private void Inject(string sql)
    {
        nint database = 0;
        try
        {
            Check(sqlite3_open_v2(Path.Combine(directory, "settings.db"), out database, 2, 0) == 0, "Owned fixture database did not open");
            int result = sqlite3_exec(database, sql, 0, 0, out nint error);
            if (error != 0) sqlite3_free(error);
            Check(result == 0, $"Owned SQL setup failed ({result})");
        }
        finally { if (database != 0) Check(sqlite3_close(database) == 0, "SQL fixture connection remained open"); }
    }
    private async Task<SettingsWindow> ShowSettings()
    {
        main.OpenSettings(); var settings = main.Settings!; settings.ShowOpenerSettings();
        await WaitUntil(() => settings.IsContentReady, "Settings window did not load");
        return settings;
    }
    private async Task Choose(SettingsWindow settings, string key, string? selection)
    {
        var result = new TaskCompletionSource<string?>(TaskCreationOptions.RunContinuationsAsynchronously);
        settings.Openers.PickerReader = _ => result.Task;
        settings.Openers.InvokeBrowse(key);
        await WaitUntil(() => settings.Openers.IsPicking, "Browse control did not start a choice");
        Check(!settings.HasDraft, "Save remained enabled during pending choice");
        result.SetResult(selection);
        await WaitUntil(() => !settings.Openers.IsPicking, "Choice did not complete");
        pickerResponses++; settings.Openers.PickerReader = null;
    }
    internal async void Start()
    {
        try
        {
            await WaitUntil(() => main.ResidentMode && !main.History.IsBusy, "Main window was not ready");
            var captureBefore = await core.GetCaptureStatusAsync();
            var targetBefore = await core.GetTargetStatusAsync();
            var lifetime = main.OpenLifetime;
            ulong revision = core.Snapshot.Revision;
            var settings = await ShowSettings();
            Check(Keys.All(key => settings.Openers.Draft(key) == core.Snapshot.Values[key]), "Initial eight selections did not bind");
            Check(!settings.HasDraft && core.Refresh().Revision == revision, "Initial binding saved settings");
            main.CloseSettings(); Check(!settings.Openers.HasRetainedData && main.Settings is null, "Close retained opener data");
            settings = await ShowSettings();
            Check(core.Refresh().Revision == revision && ReferenceEquals(lifetime, main.OpenLifetime), "Reopen saved or replaced core owner");
            var legacy = new Dictionary<string,string> {
                ["app.text"] = "legacy unsupported\r\nselection", ["app.code"] = "",
                ["app.rich_text"] = @"C:\missing legacy\editor.exe", ["app.image"] = "Family_123!App",
                ["app.file"] = "  ", ["app.video"] = "legacy video selection", ["app.url"] = "Browser_123!App", ["app.link"] = "old link"
            };
            Inject("BEGIN;" + string.Join("", legacy.Select(pair => $"INSERT OR REPLACE INTO settings(key,value) VALUES('{pair.Key}',CAST(X'{Convert.ToHexString(Encoding.UTF8.GetBytes(pair.Value))}' AS TEXT));"))
                + "UPDATE native_settings_revision SET revision=revision+1 WHERE id=1; COMMIT;");
            core.Refresh(); revision = core.Snapshot.Revision;
            Check(Keys.All(key => settings.Openers.Draft(key) == legacy[key]), "Legacy values or exact CRLF were changed by binding");
            Check(!settings.HasDraft, "Legacy invalid selection caused a false draft");
            settings.Openers.SetDraft("app.url", "NextBrowser_123!App"); Check(settings.HasDraft, "Text change did not immediately enable save");
            settings.Openers.SetDraft("app.url", legacy["app.url"]); Check(!settings.HasDraft, "Undo did not immediately clear dirty state");
            settings.SetSmokeLimit("321"); settings.InvokeSettingsSave();
            Check(core.Snapshot.Revision == revision + 1 && core.Snapshot.Values["app.persistent_limit"] == "321"
                && Keys.All(key => core.Snapshot.Values[key] == legacy[key]) && !settings.HasDraft && !settings.HasError,
                "Ordinary save rewrote unchanged legacy selections");
            revision = core.Snapshot.Revision;
            settings.Openers.SetDraft("app.text", "cmd.exe /c calc"); settings.SetSmokeLimit("322"); settings.InvokeSettingsSave();
            Check(settings.HasError && settings.HasDraft && core.Refresh().Revision == revision
                && core.Snapshot.Values["app.persistent_limit"] == "321" && settings.DraftLimit == "322"
                && settings.Openers.Draft("app.text") == "cmd.exe /c calc", "Invalid opener partially saved or lost drafts");
            settings.InvokeSettingsReload(); Check(!settings.HasDraft && !settings.HasError && settings.DraftLimit == "321", "Reload did not discard rejected drafts");
            settings.Openers.SetDraft("app.text", @"C:\owned editor.exe"); settings.SetSmokeLimit("322");
            Inject("CREATE TRIGGER owned_opener_save_failure BEFORE INSERT ON settings WHEN NEW.key='app.text' BEGIN SELECT RAISE(ABORT,'owned opener failure'); END;");
            settings.InvokeSettingsSave();
            Check(settings.HasError && settings.HasDraft && core.Refresh().Revision == revision
                && core.Snapshot.Values["app.persistent_limit"] == "321" && core.Snapshot.Values["app.text"] == legacy["app.text"],
                "SQL failure did not roll back the combined ordinary/opener patch");
            Inject("DROP TRIGGER owned_opener_save_failure;");
            settings.InvokeSettingsSave(); revision++;
            Check(core.Snapshot.Revision == revision && core.Snapshot.Values["app.persistent_limit"] == "322"
                && core.Snapshot.Values["app.text"] == @"C:\owned editor.exe" && !settings.HasError, "Failed save could not be retried once");
            settings.SetSmokeLimit("323"); settings.Openers.SetDraft("app.url", "DraftBrowser_123!App");
            core.Update(new Dictionary<string,string> { ["app.color_mode"] = "dark", ["app.url"] = "AcceptedBrowser_123!App" }, revision);
            revision++;
            Check(settings.HasError && settings.HasDraft && settings.DisplayedRevision == revision-1 && settings.DraftLimit == "323"
                && settings.Openers.Draft("app.url") == "DraftBrowser_123!App" && settings.CurrentTheme == ElementTheme.Dark
                && main.CurrentTheme == ElementTheme.Dark, "External snapshot discarded drafts or did not apply accepted theme");
            settings.InvokeSettingsSave(); Check(settings.HasError && core.Refresh().Revision == revision
                && core.Snapshot.Values["app.url"] == "AcceptedBrowser_123!App", "Conflict overwrote accepted setting");
            settings.InvokeSettingsReload();
            Check(!settings.HasDraft && settings.DisplayedRevision == revision && settings.Openers.Draft("app.url") == "AcceptedBrowser_123!App", "Conflict reload did not follow accepted values");
            foreach (string key in Keys) settings.Openers.InvokeDefault(key);
            Check(Keys.All(key => settings.Openers.Draft(key) == "") && settings.HasDraft, "Default controls did not clear all eight drafts");
            settings.InvokeSettingsSave(); revision++;
            Check(core.Snapshot.Revision == revision && Keys.All(key => core.Snapshot.Values[key] == ""), "Defaults were not saved once");
            await Choose(settings, "app.text", null);
            Check(!settings.HasDraft && core.Refresh().Revision == revision, "Cancelled picker changed draft or database");
            string fixture = Path.Combine(directory, "选择 编辑器 %20 ' &"); Directory.CreateDirectory(fixture);
            foreach (string file in new[] { "TieZ.Core.Smoke.exe", "TieZ.Core.Smoke.dll", "TieZ.Core.Smoke.deps.json", "TieZ.Core.Smoke.runtimeconfig.json", "TieZ.Core.dll" })
                File.Copy(Path.Combine(fixtureSource, file), Path.Combine(fixture, file));
            File.WriteAllText(Path.Combine(fixture, "open-fixture.marker"), "owned child");
            string executable = Path.Combine(fixture, "TieZ.Core.Smoke.exe");
            await Choose(settings, "app.text", executable);
            Check(settings.HasDraft && settings.Openers.Draft("app.text") == executable && core.Refresh().Revision == revision, "File selection saved before explicit save");
            settings.InvokeSettingsSave(); revision++;
            Check(core.Snapshot.Revision == revision && core.Snapshot.Values["app.text"] == executable && core.Snapshot.Values["app.code"] == "", "Selected handler/code fallback did not save");
            const string body = "  设置选择的编辑器\r\n\t  ";
            long id = (await core.AddManualAsync(body, "code")).Id;
            var opened = await core.OpenContentAsync(id, (await core.GetHistoryAsync(new())).Revision);
            string observedPath = Path.Combine(fixture, "open-observed.json");
            await WaitUntil(() => File.Exists(observedPath), "Saved selection did not launch owned editor");
            using (var document = JsonDocument.Parse(File.ReadAllText(observedPath)))
            {
                var root = document.RootElement; exported = root.GetProperty("Path").GetString();
                Check(opened.Route == "desktop" && opened.Exported && Convert.FromBase64String(root.GetProperty("Body").GetString()!).SequenceEqual(Encoding.UTF8.GetBytes(body)),
                    "Code-to-text fallback or exact content bytes changed");
                int pid = root.GetProperty("Pid").GetInt32();
                try { using var child = Process.GetProcessById(pid); await child.WaitForExitAsync().WaitAsync(TimeSpan.FromSeconds(5)); }
                catch (ArgumentException) { }
                launches++;
            }
            for (int scenario = 0; scenario < 3; scenario++)
            {
                var pending = new TaskCompletionSource<string?>(TaskCreationOptions.RunContinuationsAsynchronously);
                settings.Openers.PickerReader = _ => pending.Task;
                settings.Openers.InvokeBrowse("app.text");
                await WaitUntil(() => settings.Openers.IsPicking, "Stale-choice fixture did not begin");
                if (scenario == 0) settings.InvokeSettingsReload();
                if (scenario == 1) settings.Openers.SetDraft("app.text", @"C:\new manual draft.exe");
                if (scenario == 2)
                {
                    settings.SetSmokeLimit("324");
                    core.Update(new Dictionary<string,string> { ["app.color_mode"] = "light" }, revision); revision++;
                }
                pending.SetResult(@"C:\late old choice.exe");
                await WaitUntil(() => !settings.Openers.IsPicking, "Stale-choice fixture did not end"); pickerResponses++;
                Check(settings.HasError && settings.Openers.Draft("app.text") != @"C:\late old choice.exe" && core.Refresh().Revision == revision,
                    "Stale picker result replaced draft or saved settings");
                settings.InvokeSettingsReload(); settings.Openers.PickerReader = null;
            }
            // Invoke the actual HWND-owned Windows picker and cancel; no real user file is selected.
            settings.Openers.InvokeBrowse("app.text");
            await WaitUntil(() => settings.Openers.IsPicking, "Native file picker did not start");
            await Task.Delay(350); settings.Openers.CancelPicker();
            await WaitUntil(() => !settings.Openers.IsPicking, "Native file picker did not cancel");
            Check(!settings.HasError && !settings.HasDraft && core.Refresh().Revision == revision, "Native cancellation changed configuration");
            nativePickerCancelled = true;
            var late = new TaskCompletionSource<string?>(TaskCreationOptions.RunContinuationsAsynchronously);
            CancellationToken pickerToken = default;
            settings.Openers.PickerReader = token => { pickerToken = token; return late.Task; };
            settings.Openers.InvokeBrowse("app.text");
            await WaitUntil(() => settings.Openers.IsPicking, "Close fixture did not begin");
            var closed = settings; main.CloseSettings();
            Check(pickerToken.IsCancellationRequested && !closed.Openers.HasRetainedData, "Close did not cancel/clear picker and drafts");
            settings = await ShowSettings(); late.SetResult(@"C:\closed-window result.exe"); await Task.Delay(80);
            Check(!closed.Openers.IsPicking && !settings.HasDraft && settings.Openers.Draft("app.text") == executable
                && core.Refresh().Revision == revision, "Late result changed a reopened window or configuration");
            Check(ReferenceEquals(lifetime, main.OpenLifetime) && (await core.GetCaptureStatusAsync()).Starts == captureBefore.Starts
                && (await core.GetTargetStatusAsync()).Starts == targetBefore.Starts, "Settings lifetime restarted shared services");
            var exitChoice = new TaskCompletionSource<string?>(TaskCreationOptions.RunContinuationsAsynchronously);
            settings.Openers.PickerReader = _ => exitChoice.Task; settings.Openers.InvokeBrowse("app.text");
            await WaitUntil(() => settings.Openers.IsPicking, "Exit fixture did not begin");
            Check(await main.Quit().WaitAsync(TimeSpan.FromSeconds(8)), "Shared shutdown did not finish");
            exitChoice.SetResult(executable);
            Check(!settings.Openers.HasRetainedData && !settings.Openers.IsPicking && !File.Exists(exported), "Exit retained picker or export");
            Finish(true, null);
        }
        catch (Exception failure) { Finish(false, failure.ToString()); await main.Quit(); }
    }
    private void Finish(bool passed, string? error)
    {
        if (finished) return; finished = true;
        bool webView2ModuleLoaded = Process.GetCurrentProcess().Modules.Cast<ProcessModule>().Select(module => module.ModuleName)
            .Concat(AppDomain.CurrentDomain.GetAssemblies().Select(assembly => assembly.GetName().Name ?? ""))
            .Any(name => name.Contains("WebView2", StringComparison.OrdinalIgnoreCase));
        try
        {
            Check(!webView2ModuleLoaded, "WebView2 loaded during settings smoke");
            if (passed)
            {
                string path = Path.GetFullPath(directory);
                Check(Path.GetDirectoryName(path) == Path.GetFullPath(Path.GetTempPath()).TrimEnd(Path.DirectorySeparatorChar)
                    && Path.GetFileName(path).StartsWith("TieZ-WinUI-原生-", StringComparison.Ordinal)
                    && (File.GetAttributes(path) & FileAttributes.ReparsePoint) == 0, "Cleanup left owned fixture namespace");
                Directory.Delete(path, recursive: true);
            }
        }
        catch (Exception failure) { passed = false; error = $"{error}\n{failure}"; }
        File.WriteAllText(report, JsonSerializer.Serialize(new { passed, error, scope = "opener-settings", checks,
            controlledPickerResponses = pickerResponses, nativePickerCancelled, realNativeEditorLaunches = launches, webView2ModuleLoaded,
            elapsedSeconds = elapsed.Elapsed.TotalSeconds }));
        Environment.ExitCode = passed ? 0 : 1;
    }
}
