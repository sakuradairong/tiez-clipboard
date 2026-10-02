# Mechanism probe: does ShellExecuteW("open") fail for a file whose default app
# is a UWP app when the calling thread has no (or MTA) COM initialization,
# while succeeding on an STA-initialized thread?
#
# This verifies the *code mechanism* only; it says nothing about any remote machine.

$ErrorActionPreference = 'Stop'

Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
public static class SEProbe {
    [DllImport("shell32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    public static extern IntPtr ShellExecuteW(IntPtr hwnd, string verb, string file, string parameters, string directory, int showCmd);

    [DllImport("ole32.dll")]
    public static extern int CoInitializeEx(IntPtr pvReserved, uint dwCoInit);

    [DllImport("ole32.dll")]
    public static extern void CoUninitialize();

    // comMode: 0 = COINIT_MULTITHREADED (0x0), 2 = COINIT_APARTMENTTHREADED (0x2)
    public static long Run(string verb, string file, int comMode, bool explicitInit) {
        int hr = 0;
        if (explicitInit) {
            hr = CoInitializeEx(IntPtr.Zero, (uint)comMode);
        }
        long ret;
        try {
            IntPtr r = ShellExecuteW(IntPtr.Zero, verb, file, null, null, 5 /*SW_SHOW*/);
            ret = (long)r;
        } finally {
            if (explicitInit && hr == 0) {
                CoUninitialize();
            }
        }
        return ret;
    }
}
"@

# 1x1 transparent PNG
$png = [Convert]::FromBase64String("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==")
$file = Join-Path $env:TEMP "tiez_se_probe.png"
[IO.File]::WriteAllBytes($file, $png)
Write-Output "probe file: $file (exists=$(Test-Path $file))"

foreach ($ext in '.png', '.gif', '.html') {
    $k = Get-ItemProperty -Path "HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\$ext\UserChoice" -ErrorAction SilentlyContinue
    Write-Output ("assoc {0} => {1}" -f $ext, $k.ProgId)
}

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

function RunOnThread([string]$label, [int]$comMode, [bool]$explicitInit) {
    $box = New-Object System.Collections.ArrayList
    $f = $file; $m = $comMode; $e = $explicitInit
    $ts = [System.Threading.ParameterizedThreadStart] {
        param($nullParam)
        $r = [SEProbe]::Run('open', $f, $m, $e)
        $null = $box.Add($r)
    }
    $t = New-Object System.Threading.Thread -ArgumentList $ts
    $t.IsBackground = $true
    $t.Start($null)
    $t.Join()
    $r = $box[0]
    Write-Output ("{0}: {1} (ret={2})" -f $label, (Interpret $r), $r)
}

Write-Output ("main PS thread apartment: {0}" -f [Threading.Thread]::CurrentThread.GetApartmentState())

RunOnThread "A_RAW_THREAD_NO_COM_INIT" 0 $false
RunOnThread "B_RAW_THREAD_EXPLICIT_MTA" 0 $true
RunOnThread "C_RAW_THREAD_EXPLICIT_STA" 2 $true

$rMain = [SEProbe]::Run('open', $file, 0, $false)
Write-Output ("D_MAIN_PS_THREAD: {0} (ret={1})" -f (Interpret $rMain), $rMain)
