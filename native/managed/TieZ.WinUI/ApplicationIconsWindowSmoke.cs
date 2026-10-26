using System.Diagnostics;
using System.Reflection;
using System.Text.Json;
using Microsoft.UI.Xaml;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Automation.Provider;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using TieZ.Core;
using Windows.Graphics.Imaging;
using Windows.Storage;
using Windows.Storage.Streams;

namespace TieZ.Native;

// Real controls/real DLL first; explicitly controlled rasters then test cancellation and recycling.
internal sealed class ApplicationIconsWindowSmoke(MainWindow main, NativeCore core, string directory, string report)
{
    private sealed record Pending(ulong Catalog, InstalledApplication App, uint Edge, CancellationToken Token)
    {
        internal readonly TaskCompletionSource<ApplicationIconRaster> Completion = new(TaskCreationOptions.RunContinuationsAsynchronously);
    }
    private readonly List<Pending> pending = [];
    private readonly Stopwatch elapsed = Stopwatch.StartNew();
    private int checks, realReads, realImages, outstanding, peak, heartbeats;
    private ulong generation = 100;
    private bool delayed = true;
    private string? screenshot, diagnostics, scrollingTarget;
    private ApplicationPickerView? diagnosticView;
    private Microsoft.UI.Dispatching.DispatcherQueueTimer? heartbeat;
    private void Check(bool value, string message) { checks++; if (!value) throw new InvalidOperationException(message); }
    private async Task Wait(Func<bool> condition, string message)
    {
        var watch = Stopwatch.StartNew();
        while (!condition() && watch.Elapsed < TimeSpan.FromSeconds(10)) await Task.Delay(15);
        Check(condition(), message);
    }
    private static IEnumerable<DependencyObject> Descendants(DependencyObject parent)
    {
        yield return parent;
        for (int i = 0; i < VisualTreeHelper.GetChildrenCount(parent); i++)
            foreach (var child in Descendants(VisualTreeHelper.GetChild(parent, i))) yield return child;
    }
    private async Task<(ContentDialog Dialog, ApplicationPickerView View)> Open(OpenerSettingsEditor editor)
    {
        editor.InvokeApplications("app.text");
        await Wait(() => editor.ActiveApplications is { IsLoaded: true, IsLoading: false } && editor.ApplicationDialog?.IsLoaded == true,
            "Actual application dialog did not load");
        return (editor.ApplicationDialog!, editor.ActiveApplications!);
    }
    private async Task Cancel(OpenerSettingsEditor editor, ContentDialog dialog, ApplicationPickerView view)
    {
        Button? button = null;
        await Wait(() => (button = Descendants(dialog).OfType<Button>().SingleOrDefault(b => b.Name == "CloseButton" && b.IsLoaded
            && b.IsEnabled && b.ActualWidth > 0 && b.ActualHeight > 0)) is not null, "Actual dialog close button missing");
        int clicks = 0;
        dialog.CloseButtonClick += (_, _) => clicks++;
        ((IInvokeProvider)new ButtonAutomationPeer(button!).GetPattern(PatternInterface.Invoke)).Invoke();
        await Wait(() => editor.ActiveApplications is null && view.IconRequests == 0, "Cancelled picker did not drain");
        Check(clicks == 1 && view.IsDisposed && !view.HasRetainedData, "Cancellation retained picker resources");
    }
    private ApplicationCatalogResult Catalog()
    {
        var apps = Enumerable.Range(0, 80).Select(i => new InstalledApplication($"App{i:000}", $"Opaque.{i}", $"C:/Fixture/App{i}.exe", "desktop", null)).ToArray();
        var recommendation = new InstalledApplication("Recommended only", "Opaque.Recommendation", "C:/Fixture/Recommendation.exe", "desktop", null);
        var groups = new Dictionary<string, ApplicationRecommendations> { ["text"] = new([new(recommendation, "association")], null, 0, false, null) };
        return new(++generation, false, new(apps, apps.Length, 0, 0, false, groups));
    }
    private static ApplicationIconRaster Raster(Pending request, bool wrongIdentity = false)
    {
        var app = request.App;
        var identity = new ApplicationIconIdentity(wrongIdentity ? "Wrong.Row" : app.SourceId, app.Handler, app.Kind, app.Unavailable);
        var json = JsonSerializer.SerializeToElement(new { job_id = 1UL, catalog_job_id = request.Catalog, edge = request.Edge,
            state = "ready", identity = new { source_id = identity.SourceId, handler = identity.Handler, kind = identity.Kind, unavailable = identity.Unavailable },
            width = 2, height = 1, pixels = new int[] { 200, 50, 10, 128, 3, 15, 250, 255 } });
        var parser = typeof(NativeCore).GetMethod("ParseIconPoll", BindingFlags.Static | BindingFlags.NonPublic)!;
        return (ApplicationIconRaster)parser.Invoke(null, [json, 1UL, request.Catalog, request.Edge, identity])!;
    }
    private Task<ApplicationIconRaster> Read(ulong catalog, InstalledApplication app, uint edge, CancellationToken token)
    {
        Check(main.DispatcherQueue.HasThreadAccess, "Icon reader was invoked outside its owner dispatcher");
        peak = Math.Max(peak, Interlocked.Increment(ref outstanding));
        var request = new Pending(catalog, app, edge, token);
        if (delayed) pending.Add(request); else request.Completion.SetResult(Raster(request));
        return Observe(request.Completion.Task);
    }
    private async Task<ApplicationIconRaster> Observe(Task<ApplicationIconRaster> task)
    {
        try { return await task.ConfigureAwait(false); }
        finally { Interlocked.Decrement(ref outstanding); }
    }
    private void Pixels()
    {
        byte[] converted = NativeIconBitmap.ToPremultipliedBgra([200, 50, 10, 128, 99, 33, 250, 0], 2, 1, 24);
        Check(converted.SequenceEqual(new byte[] { 5, 25, 100, 128, 0, 0, 0, 0 }), "RGBA channels/alpha conversion changed");
        Check(NativeIconBitmap.ToPremultipliedBgra(new byte[65536], 128, 128, 128).Length == 65536, "Maximum raster rejected");
        foreach (var bad in new (int Width, int Height, uint Edge, int Bytes)[] { (0, 1, 24, 0), (2, 1, 1, 8),
            (1, 1, 129, 4), (2, 1, 24, 4), (int.MaxValue, 2, 128, 4) })
        {
            try { NativeIconBitmap.ToPremultipliedBgra(new byte[bad.Bytes], bad.Width, bad.Height, bad.Edge); throw new Exception("Invalid raster accepted"); }
            catch (InvalidDataException) { checks++; }
        }
    }
    private async Task Capture(ApplicationPickerView view)
    {
        var bitmap = new RenderTargetBitmap();
        await bitmap.RenderAsync(view);
        Check(bitmap.PixelWidth > 0 && bitmap.PixelHeight > 0, "Native picker screenshot was empty");
        var buffer = await bitmap.GetPixelsAsync();
        using var reader = DataReader.FromBuffer(buffer);
        byte[] bytes = new byte[checked((int)buffer.Length)]; reader.ReadBytes(bytes);
        var folder = await StorageFolder.GetFolderFromPathAsync(Path.GetDirectoryName(Path.GetFullPath(report))!);
        var file = await folder.CreateFileAsync(Path.GetFileName(report) + ".png", CreationCollisionOption.GenerateUniqueName);
        using var stream = await file.OpenAsync(FileAccessMode.ReadWrite);
        var encoder = await BitmapEncoder.CreateAsync(BitmapEncoder.PngEncoderId, stream);
        double dpi = 96 * (view.XamlRoot?.RasterizationScale ?? 1);
        encoder.SetPixelData(BitmapPixelFormat.Bgra8, BitmapAlphaMode.Premultiplied,
            (uint)bitmap.PixelWidth, (uint)bitmap.PixelHeight, dpi, dpi, bytes);
        await encoder.FlushAsync(); screenshot = file.Path;
    }
    internal async void Start()
    {
        string? error = null;
        bool passed = false, webView2ModuleLoaded = false;
        try
        {
            await Wait(() => main.ResidentMode && !main.History.IsBusy, "Native main window did not become ready");
            Pixels();
            heartbeat = main.DispatcherQueue.CreateTimer(); heartbeat.Interval = TimeSpan.FromMilliseconds(15);
            heartbeat.Tick += (_, _) => heartbeats++; heartbeat.Start();
            ulong revision = core.Snapshot.Revision;
            var capture = await core.GetCaptureStatusAsync(); var target = await core.GetTargetStatusAsync();
            main.OpenSettings();
            var settings = main.Settings!; settings.ShowOpenerSettings();
            await Wait(() => settings.IsContentReady, "Settings content not loaded");
            var editor = settings.Openers;
            ulong actualCatalog = 0;
            editor.CatalogReader = async (refresh, token) =>
            {
                var catalog = await core.GetApplicationCatalogAsync(refresh, token); actualCatalog = catalog.JobId; return catalog;
            };
            editor.IconReader = (catalog, app, edge, token) => { realReads++; return core.GetApplicationIconAsync(catalog, app, edge, token); };
            var real = await Open(editor);
            await Wait(() => real.View.IconResources > 0 && real.View.IconRequests == 0, "Real core icons were not displayed in the actual dialog");
            realImages = real.View.IconResources;
            Check(realReads > 0 && real.View.IconPeakRequests <= 2, "Real reads were missing or unbounded");
            foreach (var view in real.View.IconViews.Where(view => view.HasImage))
            {
                var app = view.Choice!.Application!;
                Check(view.RenderedImage.Source is SoftwareBitmapSource && view.DisplayedCatalog == actualCatalog
                    && view.DisplayedIdentity == new ApplicationIconIdentity(app.SourceId, app.Handler, app.Kind, app.Unavailable),
                    "Real Image.Source was attached to another row/catalog");
            }
            await Capture(real.View);
            await Cancel(editor, real.Dialog, real.View);
            editor.CatalogReader = (_, _) => Task.FromResult(Catalog()); editor.IconReader = Read;
            var controlled = await Open(editor); var picker = controlled.View; diagnosticView = picker;
            await Wait(() => pending.Count == 2, "Controlled reads did not fill exactly two slots");
            Check(pending.Any(item => item.App.SourceId == "Opaque.Recommendation"), "Recommendation-only exact row was lost");
            picker.SearchDraft = "App079";
            await Wait(() => pending.All(item => item.Token.IsCancellationRequested), "Filter did not cancel offscreen reads");
            picker.InvokeRefresh(); await Wait(() => !picker.IsLoading, "Refresh did not finish");
            picker.InvokeRefresh(); await Wait(() => !picker.IsLoading, "Repeated refresh did not finish");
            int before = heartbeats;
            await Wait(() => heartbeats >= before + 3, "Blocked icon reads stopped the dispatcher");
            Check(pending.Count == 2 && picker.IconRequests == 2 && outstanding == 2 && peak == 2,
                "Cancellation/refresh prematurely returned an actual outstanding slot");
            foreach (var item in pending.ToArray()) item.Completion.SetResult(Raster(item));
            await Wait(() => pending.Count == 3, "New generation did not resume after actual old completion");
            var current = pending[2];
            Check(current.App.Name == "App079" && current.Catalog == generation, "New read used an obsolete catalog or row");
            current.Completion.SetResult(Raster(current));
            await Wait(() => picker.IconResources == 1 && picker.IconRequests == 0, "Filtered current raster was not displayed");
            var shown = picker.IconViews.Single(view => view.HasImage);
            Check(shown.RenderedImage.Source is SoftwareBitmapSource && shown.Choice!.Name == "App079"
                && shown.DisplayedCatalog == generation, "Late raster replaced the current Image.Source");
            picker.SearchDraft = "App078";
            await Wait(() => pending.Count == 4 && pending[3].App.Name == "App078" && !pending[3].Token.IsCancellationRequested,
                "Exact per-row failure fixture did not start");
            pending[3].Completion.SetException(new CoreException("icons_backend", "controlled per-row failure"));
            await Wait(() => picker.IconRequests == 0 && picker.IconViews.Any(view => view.Choice?.Name == "App078"
                && view.Unavailable && !view.HasImage), "The exact failed row did not show its fallback");
            picker.Select(pending[3].App.Handler!);
            Check(picker.CanChoose && controlled.Dialog.IsPrimaryButtonEnabled, "An unavailable icon blocked selecting its valid application");
            picker.SearchDraft = "App077";
            await Wait(() => pending.Count == 5 && pending[4].App.Name == "App077" && !pending[4].Token.IsCancellationRequested,
                "Exact wrong-identity fixture did not start");
            pending[4].Completion.SetResult(Raster(pending[4], wrongIdentity: true));
            await Wait(() => picker.IconRequests == 0 && picker.IconViews.Any(view => view.Choice?.Name == "App077"
                && view.Unavailable && !view.HasImage), "The exact wrong-identity row adopted an invalid raster");
            delayed = false; picker.SearchDraft = "App";
            await Wait(() => picker.IconResources > 0 && picker.IconRequests == 0, "Automatic controlled rasters did not display");
            var list = (ListView)picker.FindName("Choices");
            foreach (int index in new[] { 0, 20, 40, 60, 79 })
            {
                string name = $"App{index:000}"; scrollingTarget = name;
                var choice = picker.VisibleChoices.Single(choice => choice.Name == name);
                list.ScrollIntoView(choice, ScrollIntoViewAlignment.Leading);
                await Wait(() => list.ContainerFromItem(choice) is ListViewItem { IsLoaded: true },
                    $"Scrolled row {name} was not realized");
                // Virtualized extent estimates can change after ScrollIntoView. Refine the real
                // viewport offset, rather than requiring the loader to fetch an offscreen icon.
                var scroll = Descendants(list).OfType<ScrollViewer>().First();
                ApplicationIconView? targetIcon = null;
                await Wait(() =>
                {
                    if (list.ContainerFromItem(choice) is not ListViewItem container) return false;
                    targetIcon = Descendants(container).OfType<ApplicationIconView>().SingleOrDefault();
                    if (targetIcon is not { IsLoaded: true, ActualHeight: > 0 }) return false;
                    double y = targetIcon.TransformToVisual(list).TransformPoint(default).Y;
                    if (y >= 0 && y + targetIcon.ActualHeight <= list.ActualHeight) return true;
                    scroll.ChangeView(null, Math.Clamp(scroll.VerticalOffset + y - 4, 0, scroll.ScrollableHeight), null, true);
                    return false;
                }, $"Scrolled row {name} icon never entered the actual viewport");
                await Wait(() => targetIcon!.Choice == choice && targetIcon.HasImage && picker.IconRequests == 0,
                    $"Scrolled visible row {name} did not display its own icon");
                Check(picker.IconResources == picker.IconViews.Count(view => view.HasImage) && picker.IconResources <= 10,
                    "Offscreen images were retained outside the realized viewport");
            }
            foreach (var theme in new[] { ElementTheme.Light, ElementTheme.Dark, ElementTheme.Default })
            {
                picker.RequestedTheme = theme;
                await Task.Delay(40);
                Check(picker.IconViews.Any(view => view.RenderedImage.Source is SoftwareBitmapSource), "Theme change lost the native image");
            }
            settings.AppWindow.Hide();
            await Wait(() => picker.IconResources == 0 && picker.IconRequests == 0, "Hidden settings retained icon work or bitmaps");
            settings.AppWindow.Show();
            await Wait(() => picker.IconResources > 0 && picker.IconRequests == 0, "Showing settings did not recover icons");
            var presenter = (OverlappedPresenter)settings.AppWindow.Presenter;
            presenter.Minimize();
            await Wait(() => picker.IconResources == 0 && picker.IconRequests == 0, "Minimized settings retained icon work or bitmaps");
            presenter.Restore(); settings.Activate();
            await Wait(() => picker.IconResources > 0 && picker.IconRequests == 0, "Restored settings did not recover icons");
            delayed = true; picker.SearchDraft = "App045";
            await Wait(() => pending.Count == 6, "Closing fixture did not start its delayed read");
            var closing = pending[5];
            main.CloseSettings();
            Check(picker.IsDisposed && picker.IconReader is null && picker.IconResources == 0 && closing.Token.IsCancellationRequested,
                "Closing settings retained a reader/source or failed to cancel");
            Check(picker.IconRequests == 1, "Closing prematurely returned the non-cooperative read slot");
            closing.Completion.SetResult(Raster(closing));
            await Wait(() => picker.IconRequests == 0 && !picker.HasRetainedData, "Late close result did not drain without publishing");
            Check(!editor.HasRetainedData && peak <= 2 && picker.IconPeakRequests <= 2 && outstanding == 0,
                "Closed editor or cross-generation request accounting retained state");
            Check(core.Refresh().Revision == revision && (await core.GetCaptureStatusAsync()).Starts == capture.Starts
                && (await core.GetTargetStatusAsync()).Starts == target.Starts, "Icon lifecycle saved settings or restarted services");
            main.OpenSettings(); settings = main.Settings!; settings.ShowOpenerSettings();
            await Wait(() => settings.IsContentReady, "Reopened settings did not load");
            var quitting = await Open(settings.Openers);
            await Wait(() => quitting.View.IconResources > 0 && quitting.View.IconRequests == 0, "Reopened native icon path failed");
            heldGate = (SemaphoreSlim)typeof(NativeCore).GetField("requestGate", BindingFlags.Instance | BindingFlags.NonPublic)!.GetValue(core)!;
            await heldGate.WaitAsync().WaitAsync(TimeSpan.FromSeconds(5)); gateHeld = true;
            quitting.View.SearchDraft = "no such application"; quitting.View.SearchDraft = "";
            await Wait(() => quitting.View.IconRequests == 2, "Quit fixture did not admit two actual managed icon requests");
            Task<bool> quit = main.Quit();
            Check(!quit.IsCompleted, "Quit bypassed the held native request gate");
            await Wait(() => quitting.View.IsDisposed && quitting.View.IconRequests == 0 && !quitting.View.HasRetainedData,
                "Quit did not drain picker continuations while the dispatcher was alive");
            before = heartbeats;
            await Wait(() => heartbeats >= before + 3, "Exit blocked the dispatcher");
            ReleaseGate();
            Check(await quit.WaitAsync(TimeSpan.FromSeconds(10)), "Native core did not close after icon drain");
            Check(main.Settings is null && main.OpenLifetime.IsStopped, "Exit retained borrowed windows or periodic work");
            webView2ModuleLoaded = Process.GetCurrentProcess().Modules.Cast<ProcessModule>().Select(module => module.ModuleName)
                .Concat(AppDomain.CurrentDomain.GetAssemblies().Select(assembly => assembly.GetName().Name ?? ""))
                .Any(name => name.Contains("WebView2", StringComparison.OrdinalIgnoreCase));
            Check(!webView2ModuleLoaded, "Native icons loaded WebView2");
            const string prefix = "TieZ-WinUI-原生-";
            string root = Path.GetFullPath(directory), namePart = Path.GetFileName(root);
            Check(Path.GetDirectoryName(root) == Path.GetFullPath(Path.GetTempPath()).TrimEnd(Path.DirectorySeparatorChar)
                && namePart.StartsWith(prefix, StringComparison.Ordinal) && Guid.TryParseExact(namePart[prefix.Length..], "N", out _)
                && (File.GetAttributes(root) & System.IO.FileAttributes.ReparsePoint) == 0, "Fixture cleanup left its owned namespace");
            Directory.Delete(root, recursive: true); passed = true;
        }
        catch (Exception failure)
        {
            error = failure.ToString(); ReleaseGate();
            if (diagnosticView is { IsDisposed: false } inspected)
            {
                var list = (ListView)inspected.FindName("Choices");
                diagnostics = JsonSerializer.Serialize(new { scrollingTarget, inspected.IconRequests, inspected.IconResources,
                    inspected.IsLoaded, inspected.IsLoading, inspected.IsStale, hostVisible = inspected.XamlRoot?.IsHostVisible,
                    viewportWidth = list.ActualWidth, viewportHeight = list.ActualHeight,
                    scrolls = Descendants(list).OfType<ScrollViewer>().Select(scroll => new { scroll.VerticalOffset, scroll.ScrollableHeight, scroll.ViewportHeight }),
                    views = inspected.IconViews.Select(view => new { name = view.Choice?.Name, view.HasImage, view.Unavailable,
                        view.IsLoaded, view.ActualWidth, view.ActualHeight, view.DisplayedCatalog,
                        y = view.TransformToVisual(list).TransformPoint(default).Y }) });
            }
            foreach (var item in pending) item.Completion.TrySetCanceled();
            await main.Quit();
        }
        finally
        {
            ReleaseGate(); heartbeat?.Stop();
            File.WriteAllText(report, JsonSerializer.Serialize(new { passed, error, scope = "application-icons", checks, realReads, realImages,
                controlledPeak = peak, outstanding, heartbeats, screenshot, diagnostics, webView2ModuleLoaded, elapsedSeconds = elapsed.Elapsed.TotalSeconds }));
            Environment.ExitCode = passed ? 0 : 1;
        }
    }
    private SemaphoreSlim? heldGate;
    private bool gateHeld;
    private void ReleaseGate() { if (gateHeld) { gateHeld = false; heldGate!.Release(); } }
}
