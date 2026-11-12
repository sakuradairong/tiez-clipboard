using System.Diagnostics;
using System.Reflection;
using System.Text.Json;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using TieZ.Core;

namespace TieZ.Native;

// Real native windows, one shared core DLL and an isolated database. Fixtures are ASCII on purpose: any CJK glyph
// found in the English pass is chrome that the language switch forgot, not user payload.
internal sealed class LocalizationWindowSmoke(MainWindow main, NativeCore core, string directory, string report)
{
    private const string Fixture = "localized fixture text";
    private const string Tagged = "localized tagged fixture";
    private const string Secret = "localized secret fixture";
    private const string DraftHotkey = "Ctrl+Alt+F8";
    // Characters that only exist in the simplified form; a Traditional table must have used the other glyph.
    private const string SimplifiedOnly = "设关删标签贴复载览确认选择应该记录询验读轻类项预图发过页内历缓执冲净击稳获营额层丝备历";
    private DispatcherQueueTimer? timer;
    private readonly Stopwatch elapsed = Stopwatch.StartNew();
    private bool ticking;
    private bool seeded;
    private int previewWaits;
    private int phase, tables, properties, differing;
    private long entryId;
    private ulong settingsRevision, historyRevision, captureStarts;
    private IReadOnlyList<string> zhMain = [], enMain = [], twMain = [];
    private string offenders = "";
    private string tagNames = "";
    private HashSet<string> data = [];

    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }

    internal void Start()
    {
        timer = main.DispatcherQueue.CreateTimer();
        timer.Interval = TimeSpan.FromMilliseconds(400);
        timer.Tick += Tick;
        timer.Start();
    }

    private static string Describe(IEnumerable<string> values) => string.Join(" | ", values.Take(40));

    // Failure text that names the element type and x:Name, so a leftover string can be traced to its owner.
    private static string Located(Window window) => string.Join(" | ", NativeTextScanner.Described(window, NativeTextScanner.HasCjk).Take(40));

    // Tag names and stored content are user data, never chrome: the core seeds sensitive tags such as 密码, and a
    // translated interface must show them exactly as stored.
    private IReadOnlyList<string> SimplifiedIn(IEnumerable<string> values) =>
        values.Where(value => value.Any(SimplifiedOnly.Contains) && !data.Contains(value)).ToList();

    private IReadOnlyList<string> Cjk(IEnumerable<string> values) =>
        values.Where(value => NativeTextScanner.HasCjk(value) && !data.Contains(value)).ToList();

    private async void Tick(DispatcherQueueTimer sender, object args)
    {
        if (ticking) return;
        ticking = true;
        try
        {
            Check(elapsed.Elapsed < TimeSpan.FromSeconds(90), $"Timed out in localization phase {phase}.");
            var view = main.History;
            if (!view.IsLoaded || view.IsBusy || (view.HasDialog && !view.IsDialogReady)) return;
            switch (phase)
            {
                case 0:
                    Check(LanguageService.Current == Language.SimplifiedChinese, "The default interface language is not Simplified Chinese.");
                    if (!seeded)
                    {
                        entryId = (await core.AddManualAsync(Fixture)).Id;
                        await core.AddManualAsync(Tagged, tags: ["work"]);
                        await core.AddManualAsync(Secret, tags: ["password"]);
                        await view.ReloadAsync(entryId);
                        historyRevision = (await core.GetHistoryAsync(new())).Revision;
                        captureStarts = (await core.GetCaptureStatusAsync()).Starts;
                        // Compact mode is what enables the hover/explicit preview window this check scans.
                        core.Update(new Dictionary<string, string> { ["app.compact_mode"] = "true" }, core.Snapshot.Revision);
                        settingsRevision = core.Snapshot.Revision;
                        tagNames = string.Join(" | ", (await core.GetTagsAsync(new())).Items.Select(tag => tag.Name));
                        // Tag names are stored data and the picker quotes them; both spellings are exempt.
                        data = [Fixture, Tagged, Secret, .. tagNames.Split(" | ", StringSplitOptions.RemoveEmptyEntries)
                            .SelectMany(name => new[] { name, '"' + name + '"' })];
                        seeded = true;
                    }
                    // The first row is realized a layout pass after the reload; the baseline scan must see it.
                    zhMain = NativeTextScanner.Strings(main);
                    if (!zhMain.Contains(Fixture)) return;
                    offenders = Describe(zhMain);
                    Check(zhMain.Contains("剪贴板历史") && zhMain.Contains("开始采集"), $"Simplified chrome is missing pinned literals: {offenders}");
                    Check(zhMain.Contains(Fixture), $"The fixture preview was not rendered before the first scan. CJK chrome: {string.Join(" | ", NativeTextScanner.Described(main, NativeTextScanner.HasCjk))}");
                    Check(view.SelectedHistoryIds.Contains(entryId) && view.Detail == Fixture, "Initial selection/detail not bound.");
                    break;
                case 1:
                    core.Update(new Dictionary<string, string> { ["app.language"] = "en" }, settingsRevision);
                    break;
                case 2:
                    Check(LanguageService.Current == Language.English, "Accepted English value did not reach the native views.");
                    Check(core.Snapshot.Values["app.language"] == "en" && core.Snapshot.Revision == settingsRevision + 1,
                        "Language save did not persist exactly one accepted revision.");
                    enMain = NativeTextScanner.Strings(main);
                    offenders = Located(main);
                    Check(Cjk(enMain).Count == 0, $"English main window still shows CJK chrome: {offenders}");
                    Check(main.Title == "TieZ - native migration", $"English window title is not the pinned translation: {main.Title}");
                    Check(enMain.Contains("Clipboard") && enMain.Contains("Tags") && enMain.Contains("Hide to tray")
                        && enMain.Contains("Exit TieZ") && enMain.Contains("Settings") && enMain.Contains("Clipboard history")
                        && enMain.Contains("Start capture"), $"English shell chrome is incomplete: {Describe(enMain)}");
                    Check(enMain.Intersect(zhMain).Count() < zhMain.Count, "Language switch did not change the rendered chrome.");
                    Check(view.SelectedHistoryIds.Contains(entryId) && view.Detail == Fixture, "Language switch lost selection or detail.");
                    Check((await core.GetHistoryAsync(new())).Revision == historyRevision, "Language switch wrote history.");
                    Check((await core.GetCaptureStatusAsync()).Starts == captureStarts, "Language switch restarted clipboard capture.");
                    main.OpenSettings();
                    break;
                case 3:
                    var settings = main.Settings ?? throw new InvalidOperationException("No native settings window.");
                    settings.SetSmokeHotkey(DraftHotkey);
                    var settingsChrome = NativeTextScanner.Strings(settings);
                    Check(settingsChrome.Contains("Settings") && settingsChrome.Contains("Save settings")
                        && settingsChrome.Contains("Interface language"), $"English settings chrome is incomplete: {Describe(settingsChrome)}");
                    offenders = Located(settings);
                    Check(Cjk(NativeTextScanner.Strings(settings)).Count == 0, $"English settings window still shows CJK chrome: {offenders}");
                    // The interface-language choice is exempted from the scan; check its autonyms and label here.
                    var choice = NativeTextScanner.FindByAutomationId((DependencyObject)settings.Content, NativeTextScanner.AutonymChoiceId)
                        as Microsoft.UI.Xaml.Controls.ComboBox ?? throw new InvalidOperationException("The interface-language choice is missing or not exempted.");
                    Check(choice.Items.Count == 3 && choice.Items[0] as string == "简体中文" && choice.Items[1] as string == "English"
                        && choice.Items[2] as string == "繁體中文", "Language autonyms are not the stable Simplified/English/Traditional order.");
                    string choiceLabel = Microsoft.UI.Xaml.Automation.AutomationProperties.GetName(choice);
                    Check(choiceLabel.Length > 0 && !NativeTextScanner.HasCjk(choiceLabel), $"Exempted language choice kept a CJK label: {choiceLabel}");
                    break;
                case 4:
                    Check(main.Settings!.DraftHotkey == DraftHotkey, "Settings draft was lost before the language changed.");
                    core.Update(new Dictionary<string, string> { ["app.language"] = "tw" }, core.Snapshot.Revision);
                    break;
                case 5:
                    Check(LanguageService.Current == Language.TraditionalChinese, "Accepted Traditional Chinese value did not reach the native views.");
                    twMain = NativeTextScanner.Strings(main);
                    offenders = Describe(SimplifiedIn(twMain));
                    Check(SimplifiedIn(twMain).Count == 0, $"Traditional main window still contains simplified-only glyphs: {offenders}");
                    Check(twMain.Any(value => value.Contains("剪貼簿", StringComparison.Ordinal)) && !twMain.Any(value => value.Contains("剪贴板", StringComparison.Ordinal)),
                        "Traditional chrome is not actually Traditional.");
                    var twSettings = NativeTextScanner.Strings(main.Settings!);
                    Check(SimplifiedIn(twSettings).Count == 0, $"Traditional settings window still contains simplified-only glyphs: {Describe(SimplifiedIn(twSettings))}");
                    Check(main.Settings!.DraftHotkey == DraftHotkey, "Language switch overwrote the settings draft.");
                    Check(core.Snapshot.Revision == settingsRevision + 2, "Traditional Chinese save did not persist exactly one revision.");
                    core.Update(new Dictionary<string, string> { ["app.language"] = "en" }, core.Snapshot.Revision);
                    break;
                case 6:
                    Check(LanguageService.Current == Language.English, "Returning to English did not apply.");
                    main.OpenAdvancedSettings();
                    break;
                case 7:
                    var advanced = main.AdvancedSettings ?? throw new InvalidOperationException("No native advanced settings window.");
                    offenders = Located(advanced);
                    Check(Cjk(NativeTextScanner.Strings(advanced)).Count == 0, $"English advanced window still shows CJK chrome: {offenders}");
                    Check(Cjk(NativeTextScanner.Strings(main.Settings!)).Count == 0, "Live settings window reverted its language.");
                    main.CloseAdvancedSettings();
                    break;
                case 8:
                    Check(main.AdvancedSettings is null, "Advanced settings window was not released.");
                    view.Select(entryId);
                    view.Invoke("tags");
                    break;
                case 9:
                    Check(view.HasDialog && view.TagPicker is not null, "Tag picker did not open for the language check.");
                    var dialogChrome = Cjk(NativeTextScanner.Strings(main));
                    offenders = Describe(dialogChrome) + " || " + Located(main);
                    Check(dialogChrome.Count == 0, $"Open tag dialog still shows CJK chrome: {Describe(dialogChrome)}");
                    view.CancelDialog();
                    break;
                case 10:
                    if (view.HasDialog) return;
                    main.NavigateTags();
                    break;
                case 11:
                    offenders = Located(main);
                    Check(Cjk(NativeTextScanner.Strings(main)).Count == 0, $"English tag directory still shows CJK chrome: {offenders}");
                    main.NavigateHistory();
                    break;
                case 12:
                    view.PreviewRow(entryId);
                    break;
                case 13:
                    if (main.CompactPreview.PreviewWindow is null)
                    {
                        // An explicit preview request is served on the next layout pass.
                        Check(++previewWaits < 25, "No compact preview window appeared for the explicit preview request.");
                        return;
                    }
                    var preview = main.CompactPreview.PreviewWindow;
                    offenders = Located(preview);
                    Check(Cjk(NativeTextScanner.Strings(preview)).Count == 0, $"English compact preview still shows CJK chrome: {offenders}");
                    main.CompactPreview.Cancel();
                    break;
                case 14:
                    view.Invoke("add");
                    break;
                case 15:
                    Check(view.HasDialog, "Add dialog did not open for the live dialog check.");
                    core.Update(new Dictionary<string, string> { ["app.language"] = "zh" }, core.Snapshot.Revision);
                    break;
                case 16:
                    Check(LanguageService.Current == Language.SimplifiedChinese, "Returning to Simplified Chinese did not apply.");
                    var live = NativeTextScanner.Strings(main);
                    offenders = Describe(Cjk(live));
                    Check(live.Contains("新增条目"), $"Open dialog did not follow the live language change: {offenders}");
                    Check(Cjk(live).Count > 0, "Simplified dialog text disappeared.");
                    view.CancelDialog();
                    break;
                case 17:
                    if (view.HasDialog) return;
                    // The target tracker reports live status (and the last input window name), which changes for
                    // reasons unrelated to the language, so those captions are compared by the earlier phases only.
                    string[] volatileChrome = ["目标跟踪", "上次输入窗口"];
                    bool Stable(string value) => !volatileChrome.Any(value.Contains);
                    var restored = NativeTextScanner.Strings(main).Where(NativeTextScanner.HasCjk).Where(Stable)
                        .OrderBy(value => value, StringComparer.Ordinal).ToList();
                    var original = zhMain.Where(NativeTextScanner.HasCjk).Where(Stable).OrderBy(value => value, StringComparer.Ordinal).ToList();
                    Check(restored.SequenceEqual(original),
                        $"Restoring Simplified Chinese did not reproduce the original chrome. Missing: {Describe(original.Except(restored))} Extra: {Describe(restored.Except(original))}");
                    Check(view.SelectedHistoryIds.Contains(entryId) && view.Detail == Fixture, "Round-trip language switching lost the selected record.");
                    Check(core.Snapshot.Revision == settingsRevision + 4, "Round-trip language switching wrote unexpected revisions.");
                    break;
                case 18:
                    main.CloseSettings();
                    break;
                case 19:
                    Check(main.Settings is null, "Settings window was not released.");
                    core.Update(new Dictionary<string, string> { ["app.language"] = "en" }, core.Snapshot.Revision);
                    break;
                case 20:
                    Check(LanguageService.Current == Language.English, "English did not survive the settings window closing.");
                    main.OpenSettings();
                    break;
                case 21:
                    Check(core.Snapshot.Revision == settingsRevision + 5, "Reopening settings for the language check wrote configuration.");
                    offenders = Located(main.Settings!);
                    Check(Cjk(NativeTextScanner.Strings(main.Settings!)).Count == 0, $"Reopened English settings window shows CJK chrome: {offenders}");
                    main.CloseSettings();
                    break;
                case 22:
                    CheckTables();
                    break;
                case 23:
                    bool rejected = false;
                    try { core.Update(new Dictionary<string, string> { ["app.language"] = "en-US" }, core.Snapshot.Revision); }
                    catch (CoreException error) when (error.Code is "invalid_value" or "invalid_settings" or "settings_invalid") { rejected = true; }
                    Check(rejected, "An unsupported language value was accepted.");
                    Check(LanguageService.Current == Language.English && core.Snapshot.Revision == settingsRevision + 5,
                        "Rejected language value changed accepted state.");
                    Check((await core.GetHistoryAsync(new())).Revision == historyRevision, "Localization checks wrote history.");
                    Finish(true, null);
                    return;
            }
            phase++;
        }
        catch (Exception error) { Finish(false, $"Localization phase {phase}: {error}"); }
        finally { ticking = false; }
    }

    // Static table checks: every language filled, English printable ASCII, Traditional free of simplified glyphs.
    private void CheckTables()
    {
        const BindingFlags statics = BindingFlags.Static | BindingFlags.Public | BindingFlags.NonPublic;
        const BindingFlags instances = BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic;
        var types = typeof(LanguageService).Assembly.GetTypes()
            .Where(type => type.Namespace == "TieZ.Native" && type.Name.EndsWith("Strings", StringComparison.Ordinal)
                && type.GetProperty("Zh", statics) is not null && type.GetProperty("Tw", statics) is not null && type.GetProperty("En", statics) is not null)
            .OrderBy(type => type.Name, StringComparer.Ordinal).ToList();
        Check(types.Count >= 4, $"Too few localization tables were implemented: {string.Join(", ", types.Select(type => type.Name))}");
        foreach (var type in types)
        {
            var zh = (object)type.GetProperty("Zh", statics)!.GetValue(null)!;
            var tw = (object)type.GetProperty("Tw", statics)!.GetValue(null)!;
            var en = (object)type.GetProperty("En", statics)!.GetValue(null)!;
            tables++;
            foreach (var property in type.GetProperties(instances).Where(property => property.PropertyType == typeof(string)))
            {
                properties++;
                string simplified = (string?)property.GetValue(zh) ?? "";
                string traditional = (string?)property.GetValue(tw) ?? "";
                string english = (string?)property.GetValue(en) ?? "";
                Check(simplified.Length > 0 && traditional.Length > 0 && english.Length > 0,
                    $"{type.Name}.{property.Name} is empty in at least one language.");
                // Line breaks are the only non-printable character an English value may carry.
                Check(english.All(character => character is >= '\u0020' and <= '\u007E' or '\n'),
                    $"{type.Name}.{property.Name} English value is not printable ASCII: {english}");
                Check(!traditional.Any(SimplifiedOnly.Contains), $"{type.Name}.{property.Name} Traditional value uses a simplified glyph: {traditional}");
                if (simplified != traditional) differing++;
            }
        }
        Check(differing > properties / 2, $"Only {differing} of {properties} translated values differ between Simplified and Traditional Chinese.");
    }

    private async void Finish(bool passed, string? error)
    {
        timer!.Stop();
        timer.Tick -= Tick;
        ulong revision = core.Snapshot.Revision;
        bool webView2ModuleLoaded = Process.GetCurrentProcess().Modules.Cast<ProcessModule>().Select(module => module.ModuleName)
            .Concat(AppDomain.CurrentDomain.GetAssemblies().Select(assembly => assembly.GetName().Name ?? ""))
            .Any(name => name.Contains("WebView2", StringComparison.OrdinalIgnoreCase));
        try
        {
            Check(!webView2ModuleLoaded, "WebView2 loaded in the native localization check.");
            if (!await main.Quit()) throw new Exception("Native shutdown did not release the core.");
            Directory.Delete(directory, recursive: true);
        }
        catch (Exception failure) { passed = false; error = $"{error}\n{failure}"; }
        File.WriteAllText(report, JsonSerializer.Serialize(new { passed, error, scope = "localization", phases = phase + 1,
            revision, tables, properties, differing, offenders, tagNames, webView2ModuleLoaded, elapsedSeconds = elapsed.Elapsed.TotalSeconds }));
        Environment.ExitCode = passed ? 0 : 1;
    }
}