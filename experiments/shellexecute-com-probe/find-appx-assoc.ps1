Get-ChildItem 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts' | ForEach-Object {
    $ext = $_.PSChildName
    $uc = Get-ItemProperty -Path ($_.PSPath + '\UserChoice') -ErrorAction SilentlyContinue
    if ($uc -and $uc.ProgId -like 'AppX*') {
        Write-Output ('{0} => {1}' -f $ext, $uc.ProgId)
    }
}
