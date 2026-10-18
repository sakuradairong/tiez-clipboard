using System.Diagnostics;
using System.Reflection;
using System.Runtime.InteropServices;
using System.Text.Json;
using TieZ.Core;

internal static class AppsSmoke
{
    private static int checks;
    private static void Check(bool value, string message) { checks++; if (!value) throw new Exception(message); }
    private static T Read<T>(Task<T> task) => task.GetAwaiter().GetResult();
    private static ulong Handle(NativeCore core) => (ulong)typeof(NativeCore).GetField("handle",BindingFlags.Instance|BindingFlags.NonPublic)!.GetValue(core)!;
    private static void Cancelled(Task task)
    {
        try { task.GetAwaiter().GetResult(); } catch (OperationCanceledException) { checks++; return; }
        throw new Exception("Expected cancelled catalog request");
    }
    [DllImport("tiez_native_core",EntryPoint="tiez_core_request",CallingConvention=CallingConvention.Cdecl)]
    private static extern nint RawRequest([In] byte[] bytes,nuint length);
    [DllImport("tiez_native_core",EntryPoint="tiez_core_free",CallingConvention=CallingConvention.Cdecl)]
    private static extern void RawFree(nint value);
    private static JsonElement Raw(object command)
    {
        byte[] bytes=JsonSerializer.SerializeToUtf8Bytes(command);nint response=RawRequest(bytes,(nuint)bytes.Length);
        try {using var result=JsonDocument.Parse(Marshal.PtrToStringUTF8(response)!);return result.RootElement.Clone();}
        finally {RawFree(response);}
    }
    private static void Running(NativeCore core,ulong job=1)
    {
        var watch=Stopwatch.StartNew();
        while(watch.Elapsed<TimeSpan.FromSeconds(8))
        {
            var result=Raw(new {op="apps_poll",handle=Handle(core),job_id=job});
            if(result.GetProperty("ok").GetBoolean() && result.GetProperty("result").GetProperty("state").GetString()=="running") {checks++;return;}
            Thread.Sleep(5);
        }
        throw new Exception("Real catalog scan did not enter running state");
    }
    private static void DisposeActive(string database,bool asynchronous)
    {
        using var core=new NativeCore(database);ulong handle=Handle(core);
        var pending=core.GetApplicationCatalogAsync(refresh:true);Running(core);
        if(asynchronous) core.DisposeAsync().AsTask().GetAwaiter().GetResult();else core.Dispose();
        Cancelled(pending);
        Check(Raw(new {op="apps_poll",handle,job_id=1}).GetProperty("error").GetProperty("code").GetString()=="invalid_handle","Disposed catalog handle remained valid");
        using var reopened=new NativeCore(database);
        Check(reopened.Snapshot.Revision==0,"Active scan disposal saved settings");
    }
    internal static void Run(string directory)
    {
        checks=0;Directory.CreateDirectory(directory);
        string database=Path.Combine(directory,"应用.db");
        using(var core=new NativeCore(database))
        {
            ulong handle=Handle(core);
            using var cancelled=new CancellationTokenSource();cancelled.Cancel();
            Cancelled(core.GetApplicationCatalogAsync(cancellationToken:cancelled.Token));
            Check(Raw(new {op="apps_poll",handle,job_id=1}).GetProperty("error").GetProperty("code").GetString()=="apps_job","Pre-cancelled request created a native job");
            using var active=new CancellationTokenSource();var pending=core.GetApplicationCatalogAsync(true,active.Token);Running(core);
            Check(core.Refresh().Revision==0 && Read(core.GetHistoryAsync(new())).Items.Count==0,"Catalog scan blocked or changed settings/history");
            active.Cancel();Cancelled(pending);
            Check(Raw(new {op="apps_poll",handle,job_id=1}).GetProperty("error").GetProperty("code").GetString()=="apps_cancelled","Managed cancellation did not reach Rust");
        }
        using(var core=new NativeCore(database))
        {
            int notifications=0;core.Changed+=_=>notifications++;
            var first=Read(core.GetApplicationCatalogAsync(refresh:true));
            Check(first.JobId==1 && !first.Cached && first.Snapshot.Scanned>0,"Real snapshot or job ID missing");
            Check(first.Snapshot.Scanned==first.Snapshot.Applications.Count+first.Snapshot.Unreadable+first.Snapshot.Duplicates,"Wire counts did not reconcile");
            Check(first.Snapshot.Applications.All(app=>app.Kind=="unavailable" ? app.Handler is null && app.Unavailable is not null : app.Handler is not null && app.Unavailable is null),"Availability contract was lost at C ABI boundary");
            Check(new[]{"text","code","rich_text","image","file","video","url","link"}.All(first.Snapshot.Recommendations.ContainsKey),"Content recommendation groups missing");
            Check(first.Snapshot.Recommendations.Values.All(group=>group.Applications.All(item=>item.Application.Handler is not null && item.Reason is "association" or "keyword")),"Recommendation DTO is invalid");
            var cached=Read(core.GetApplicationCatalogAsync());
            Check(cached.Cached && cached.JobId>first.JobId && JsonSerializer.Serialize(cached.Snapshot)==JsonSerializer.Serialize(first.Snapshot),"Read-only cache rescanned or changed snapshot");
            Check(core.Refresh().Revision==0 && notifications==0 && Read(core.GetCaptureStatusAsync()).Starts==0,"Catalog saved, notified or restarted capture");
            try {Task.Run(()=>Read(core.GetApplicationCatalogAsync())).GetAwaiter().GetResult();throw new Exception("Wrong owner accepted");}
            catch(InvalidOperationException) {checks++;}
            var gate=(SemaphoreSlim)typeof(NativeCore).GetField("appsGate",BindingFlags.Instance|BindingFlags.NonPublic)!.GetValue(core)!;
            Check(gate.Wait(0),"Catalog gate not released after reads");
            try
            {
                var queued=core.GetApplicationCatalogAsync();
                var closing=core.DisposeAsync().AsTask();Cancelled(queued);
                Check(!closing.IsCompleted,"Close skipped a held application gate");
                gate.Release();gate=null!;closing.GetAwaiter().GetResult();
            }
            finally {gate?.Release();}
            Console.WriteLine($"PASS apps metadata: scanned={first.Snapshot.Scanned}, candidates={first.Snapshot.Applications.Count(app=>app.Handler is not null)}, recommendations={first.Snapshot.Recommendations.Count}");
        }
        DisposeActive(Path.Combine(directory,"异步关闭.db"),true);
        DisposeActive(Path.Combine(directory,"同步关闭.db"),false);
        bool webView=Process.GetCurrentProcess().Modules.Cast<ProcessModule>().Select(module=>module.ModuleName)
            .Concat(AppDomain.CurrentDomain.GetAssemblies().Select(assembly=>assembly.GetName().Name??""))
            .Any(name=>name.Contains("WebView2",StringComparison.OrdinalIgnoreCase));
        Check(!webView,"Catalog loaded WebView2");
        Console.WriteLine($"PASS application catalog real DLL: {checks} checks, cancellation/cache/readonly/owner/queued and active disposal; no WebView2");
    }
}
