Add-Type -AssemblyName System.Windows.Forms
$procs = Get-Process | Where-Object { $_.MainWindowTitle -ne '' -and ($_.ProcessName -match 'tiez_app') }
foreach ($p in $procs) {
    Write-Output ("proc={0} pid={1} title={2}" -f $p.ProcessName, $p.Id, $p.MainWindowTitle)
}
# Also enumerate all top-level visible windows with a signature title
$sig = @"
[DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc cb, IntPtr lp);
public delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lp);
[DllImport("user32.dll")] public static extern int GetWindowText(IntPtr hWnd, System.Text.StringBuilder sb, int max);
[DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hWnd);
[DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint pid);
"@
Add-Type -MemberDefinition $sig -Name U32 -Namespace W
$found = New-Object System.Collections.ArrayList
$cb = [W.U32+EnumWindowsProc] {
    param($h, $lp)
    $sb = New-Object System.Text.StringBuilder 256
    [void][W.U32]::GetWindowText($h, $sb, 256)
    $title = $sb.ToString()
    if ([W.U32]::IsWindowVisible($h) -and $title -match 'open|打开') {
        $pid2 = 0
        [void][W.U32]::GetWindowThreadProcessId($h, [ref]$pid2)
        $null = $found.Add(("hwnd={0} pid={1} title={2}" -f $h, $pid2, $title))
    }
    return $true
}
[void][W.U32]::EnumWindows($cb, [IntPtr]::Zero)
$found | ForEach-Object { Write-Output $_ }
