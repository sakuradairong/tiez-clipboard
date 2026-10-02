param([int]$TargetPid)
$sig = @"
[DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc cb, IntPtr lp);
public delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lp);
[DllImport("user32.dll")] public static extern int GetWindowText(IntPtr hWnd, System.Text.StringBuilder sb, int max);
[DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hWnd);
[DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint pid);
[DllImport("user32.dll")] public static extern int GetClassName(IntPtr hWnd, System.Text.StringBuilder sb, int max);
"@
Add-Type -MemberDefinition $sig -Name U32B -Namespace WB
$rows = New-Object System.Collections.ArrayList
$cb = [WB.U32B+EnumWindowsProc] {
    param($h, $lp)
    $wpid = 0
    [void][WB.U32B]::GetWindowThreadProcessId($h, [ref]$wpid)
    if ($wpid -eq $TargetPid) {
        $t = New-Object System.Text.StringBuilder 512
        [void][WB.U32B]::GetWindowText($h, $t, 512)
        $c = New-Object System.Text.StringBuilder 256
        [void][WB.U32B]::GetClassName($h, $c, 256)
        $null = $rows.Add(("hwnd={0} visible={1} class={2} title={3}" -f $h, [WB.U32B]::IsWindowVisible($h), $c, $t))
    }
    return $true
}
[void][WB.U32B]::EnumWindows($cb, [IntPtr]::Zero)
$rows | ForEach-Object { Write-Output $_ }
