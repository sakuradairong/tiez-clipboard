using System.Diagnostics;
using System.Reflection;
using System.Runtime.InteropServices;
using System.Text.Json;
using Microsoft.UI.Xaml;
using TieZ.Core;

namespace TieZ.Native;

// Actual WinUI dispatcher/lifetime acceptance for the shared catalog API; no app is launched.
internal sealed class NativeAppsWindowSmoke(MainWindow main, NativeCore core, string directory, string report)
{
    private readonly Stopwatch elapsed=Stopwatch.StartNew();
    private int checks,heartbeats;private bool finished,held;
    private SemaphoreSlim? gate;
    private Microsoft.UI.Dispatching.DispatcherQueueTimer? heartbeat;
    [DllImport("tiez_native_core",EntryPoint="tiez_core_request",CallingConvention=CallingConvention.Cdecl)]
    private static extern nint RawRequest([In]byte[] bytes,nuint length);
    [DllImport("tiez_native_core",EntryPoint="tiez_core_free",CallingConvention=CallingConvention.Cdecl)] private static extern void RawFree(nint value);
    private void Check(bool value,string message){checks++;if(!value)throw new InvalidOperationException(message);}
    private bool Running(ulong job)
    {
        ulong handle=(ulong)typeof(NativeCore).GetField("handle",BindingFlags.Instance|BindingFlags.NonPublic)!.GetValue(core)!;
        byte[] bytes=JsonSerializer.SerializeToUtf8Bytes(new {op="apps_poll",handle,job_id=job});nint response=RawRequest(bytes,(nuint)bytes.Length);
        try{using var document=JsonDocument.Parse(Marshal.PtrToStringUTF8(response)!);var root=document.RootElement;
            return root.GetProperty("ok").GetBoolean()&&root.GetProperty("result").GetProperty("state").GetString()=="running";}
        finally{RawFree(response);}
    }
    private async Task WaitUntil(Func<bool> ready,string message)
    {
        var watch=Stopwatch.StartNew();while(!ready()&&watch.Elapsed<TimeSpan.FromSeconds(8))await Task.Delay(15);Check(ready(),message);
    }
    private void Release(){if(held){gate!.Release();held=false;}}
    internal async void Start()
    {
        try
        {
            await WaitUntil(()=>main.ResidentMode&&!main.History.IsBusy,"Native window was not ready");
            heartbeat=main.DispatcherQueue.CreateTimer();heartbeat.Interval=TimeSpan.FromMilliseconds(15);heartbeat.Tick+=(_,_)=>heartbeats++;heartbeat.Start();
            ulong revision=core.Snapshot.Revision;
            var target=await core.GetTargetStatusAsync();var capture=await core.GetCaptureStatusAsync();
            int initial=heartbeats;
            var query=core.GetApplicationCatalogAsync(refresh:true);await WaitUntil(()=>Running(1),"Actual scan did not run");
            main.HideToTray();main.OpenSettings();
            await WaitUntil(()=>main.Settings is {IsContentReady:true},"Independent settings did not load during scan");
            main.CloseSettings();main.OpenSettings();main.CloseSettings();
            var first=await query;
            Check(heartbeats>=initial+3,"Catalog blocked the actual dispatcher");
            Check(first.JobId==1&&!first.Cached&&first.Snapshot.Scanned>0,"Native catalog result missing");
            Check(!main.AppWindow.IsVisible,"Catalog completion unexpectedly showed the hidden main window");main.ShowMain();
            var cached=await core.GetApplicationCatalogAsync();
            Check(cached.Cached&&cached.JobId==2&&cached.Snapshot.Scanned==first.Snapshot.Scanned,"Windows did not share the core cache");
            Check(core.Refresh().Revision==revision&&(await core.GetTargetStatusAsync()).Starts==target.Starts
                &&(await core.GetCaptureStatusAsync()).Starts==capture.Starts,"Catalog/settings lifetime saved or restarted services");
            var pending=core.GetApplicationCatalogAsync(refresh:true);await WaitUntil(()=>Running(3),"Exit scan did not run");
            gate=(SemaphoreSlim)typeof(NativeCore).GetField("requestGate",BindingFlags.Instance|BindingFlags.NonPublic)!.GetValue(core)!;
            await gate.WaitAsync().WaitAsync(TimeSpan.FromSeconds(8));held=true;
            int before=heartbeats;Task<bool> quit=main.Quit();Check(!quit.IsCompleted,"Quit skipped pending catalog cancellation");
            await WaitUntil(()=>heartbeats>=before+3,"Dispatcher stopped while shutdown waited for catalog cancellation");
            Release();heartbeat.Stop();Check(await quit.WaitAsync(TimeSpan.FromSeconds(8)),"Native app did not finish catalog shutdown");
            try{pending.GetAwaiter().GetResult();throw new Exception("Exit scan was not cancelled");}catch(OperationCanceledException){checks++;}
            Check(main.Settings is null&&main.OpenLifetime.IsStopped,"Quit retained settings or periodic work");
            Finish(true,null);
        }
        catch(Exception failure){Release();heartbeat?.Stop();Finish(false,failure.ToString());await main.Quit();}
    }
    private void Finish(bool passed,string? error)
    {
        if(finished)return;finished=true;heartbeat?.Stop();
        bool webView2ModuleLoaded=Process.GetCurrentProcess().Modules.Cast<ProcessModule>().Select(module=>module.ModuleName)
            .Concat(AppDomain.CurrentDomain.GetAssemblies().Select(assembly=>assembly.GetName().Name??""))
            .Any(name=>name.Contains("WebView2",StringComparison.OrdinalIgnoreCase));
        try
        {
            Check(!webView2ModuleLoaded,"Catalog loaded WebView2");
            if(passed)
            {
                string path=Path.GetFullPath(directory);
                Check(Path.GetDirectoryName(path)==Path.GetFullPath(Path.GetTempPath()).TrimEnd(Path.DirectorySeparatorChar)
                    &&Path.GetFileName(path).StartsWith("TieZ-WinUI-原生-",StringComparison.Ordinal)
                    &&(File.GetAttributes(path)&FileAttributes.ReparsePoint)==0,"Cleanup left owned fixture namespace");
                Directory.Delete(path,recursive:true);
            }
        }
        catch(Exception failure){passed=false;error=$"{error}\n{failure}";}
        File.WriteAllText(report,JsonSerializer.Serialize(new {passed,error,scope="application-catalog-runtime",checks,heartbeats,webView2ModuleLoaded,elapsedSeconds=elapsed.Elapsed.TotalSeconds}));
        Environment.ExitCode=passed?0:1;
    }
}
