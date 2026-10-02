# Mechanism probe: ShellExecuteW("open") on a UWP-associated file (.jpeg => Photos
# on this machine, matching a fresh Windows machine's .png => Photos) from threads
# with different COM states. Verifies the code mechanism only.

$ErrorActionPreference = 'Stop'

Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
public static class SEProbe {
    [DllImport("shell32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern IntPtr ShellExecuteW(IntPtr hwnd, string verb, string file, string parameters, string directory, int showCmd);

    [DllImport("ole32.dll")]
    static extern int CoInitializeEx(IntPtr pvReserved, uint dwCoInit);

    [DllImport("ole32.dll")]
    static extern void CoUninitialize();

    // comMode: -1 = leave thread COM untouched; 0 = MTA; 2 = STA
    static long RunOnce(string file, int comMode) {
        int hr = 0;
        if (comMode >= 0) {
            hr = CoInitializeEx(IntPtr.Zero, (uint)comMode);
        }
        long ret;
        try {
            ret = (long)ShellExecuteW(IntPtr.Zero, "open", file, null, null, 5);
        } finally {
            if (comMode >= 0 && hr == 0) { CoUninitialize(); }
        }
        return ret;
    }

    public static long RunOnFreshThread(string file, int comMode) {
        long result = -999;
        var t = new System.Threading.Thread(() => { result = RunOnce(file, comMode); });
        t.IsBackground = true;
        t.Start();
        t.Join();
        return result;
    }

    public static long RunOnCurrentThread(string file, int comMode) {
        return RunOnce(file, comMode);
    }
}
"@

$jpeg = [Convert]::FromBase64String("/9j/4AAQSkZJRgABAQEAYABgAAD/2wBDAAgGBgcGBQgHBwcJCQgKDBQNDAsLDBkSEw8UHRofHh0aHBwgJC4nICIsIxwcKDcpLDAxNDQ0Hyc5PTgyPC4zNDL/2wBDAQkJCQwLDBgNDRgyIRwhMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjL/wAARCAABAAEDASIAAhEBAxEB/8QAHwAAAQUBAQEBAQEAAAAAAAAAAAECAwQFBgcICQoL/8QAtRAAAgEDAwIEAwUFBAQAAAF9AQIDAAQRBRIhMUEGE1FhByJxFDKBkaEII0KxwRVS0fAkM2JyggkKFhcYGRolJicoKSo0NTY3ODk6Q0RFRkdISUpTVFVWV1hZWmNkZWZnaGlqc3R1dnd4eXqDhIWGh4iJipKTlJWWl5iZmqKjpKWmp6ipqrKztLW2t7i5usLDxMXGx8jJytLT1NXW19jZ2uHi4+Tl5ufo6erx8vP09fb3+Pn6/9oACAEBAAA/APn+v//Z")
$file = Join-Path $env:TEMP "tiez_se_probe.jpeg"
[IO.File]::WriteAllBytes($file, $jpeg)
Write-Output "probe file: $file (exists=$(Test-Path $file))"

$k = Get-ItemProperty -Path "HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\.jpeg\UserChoice" -ErrorAction SilentlyContinue
Write-Output ("assoc .jpeg => {0}" -f $k.ProgId)

function Interpret([long]$r) {
    if ($r -gt 32) { return "SUCCESS" }
    switch ($r) {
        0  { return "SE_ERR_OOM (0)" }
        2  { return "SE_ERR_FNF (2) file not found" }
        3  { return "SE_ERR_PNF (3) path not found" }
        5  { return "SE_ERR_ACCESSDENIED (5)" }
        8  { return "SE_ERR_OOM (8)" }
        26 { return "SE_ERR_SHARE (26)" }
        27 { return "SE_ERR_ASSOCINCOMPLETE (27)" }
        28 { return "SE_ERR_DDETIMEOUT (28)" }
        29 { return "SE_ERR_DDEFAIL (29)" }
        30 { return "SE_ERR_DDEBUSY (30)" }
        31 { return "SE_ERR_NOASSOC (31)" }
        32 { return "SE_ERR_DLLNOTFOUND (32)" }
        default { return "ERROR ($r)" }
    }
}

Write-Output ("main PS thread apartment: {0}" -f [Threading.Thread]::CurrentThread.GetApartmentState())

$cases = @(
    @{ Label = 'A_FRESH_THREAD_NO_COM_INIT'; Mode = -1 },
    @{ Label = 'B_FRESH_THREAD_EXPLICIT_MTA'; Mode = 0 },
    @{ Label = 'C_FRESH_THREAD_EXPLICIT_STA'; Mode = 2 }
)
foreach ($c in $cases) {
    $r = [SEProbe]::RunOnFreshThread($file, $c.Mode)
    Write-Output ("{0}: {1} (ret={2})" -f $c.Label, (Interpret $r), $r)
    Start-Sleep -Milliseconds 800
}

$rMain = [SEProbe]::RunOnCurrentThread($file, -1)
Write-Output ("D_MAIN_STA_PS_THREAD: {0} (ret={1})" -f (Interpret $rMain), $rMain)
