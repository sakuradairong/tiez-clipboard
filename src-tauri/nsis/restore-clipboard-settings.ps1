$ErrorActionPreference = 'Stop'

function Test-ClipboardValueOwnership {
    param(
        [bool]$CurrentPresent,
        $CurrentValue,
        $CurrentKind,
        [bool]$AppliedPresent,
        $AppliedValue,
        $AppliedKind
    )
    if ($CurrentPresent -ne $AppliedPresent) { return $false }
    if (-not $AppliedPresent) { return $true }
    return $CurrentKind -eq $AppliedKind -and [object]::Equals($CurrentValue, $AppliedValue)
}

function Restore-TieZClipboardSettings {
    $backupRoot = 'Software\tiez\ClipboardSettingsBackup'
    $settings = @(
        @{ Path = 'Software\Microsoft\Clipboard'; Name = 'EnableClipboardHistory' },
        @{ Path = 'Software\Microsoft\Clipboard'; Name = 'EnableCloudClipboard' },
        @{ Path = 'Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced'; Name = 'DisabledHotkeys' },
        @{ Path = 'Software\Microsoft\Windows\CurrentVersion\Policies\Explorer'; Name = 'DisallowClipboardHistory' },
        @{ Path = 'Software\Policies\Microsoft\Windows\System'; Name = 'AllowClipboardHistory' },
        @{ Path = 'Software\Policies\Microsoft\Windows\System'; Name = 'AllowCrossDeviceClipboard' }
    )
    $hkcu = [Microsoft.Win32.Registry]::CurrentUser
    $readOptions = [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames
    $restartExplorer = $false

    foreach ($setting in $settings) {
        $snapshotPath = "$backupRoot\$($setting.Name)"
        $backup = $null
        $target = $null
        $removeSnapshot = $false
        try {
            $backup = $hkcu.OpenSubKey($snapshotPath)
            if ($null -eq $backup) { continue }
            $originalPresent = $backup.GetValue('OriginalPresent', $null)
            $appliedPresent = $backup.GetValue('AppliedPresent', $null)
            if ($backup.GetValueKind('OriginalPresent') -ne [Microsoft.Win32.RegistryValueKind]::DWord -or
                $backup.GetValueKind('AppliedPresent') -ne [Microsoft.Win32.RegistryValueKind]::DWord -or
                $originalPresent -notin @(0, 1) -or $appliedPresent -notin @(0, 1)) {
                throw 'Incomplete clipboard setting snapshot'
            }

            $appliedValue = $null
            $appliedKind = $null
            if ($appliedPresent -eq 1) {
                $appliedKind = $backup.GetValueKind('AppliedValue')
                $appliedValue = $backup.GetValue('AppliedValue', $null, $readOptions)
                if ($appliedKind -notin @([Microsoft.Win32.RegistryValueKind]::String, [Microsoft.Win32.RegistryValueKind]::DWord)) {
                    throw 'Unsupported applied clipboard setting type'
                }
            }
            $target = $hkcu.OpenSubKey($setting.Path, $true)
            $currentPresent = $null -ne $target -and $target.GetValueNames() -contains $setting.Name
            $currentValue = $null
            $currentKind = $null
            if ($currentPresent) {
                $currentKind = $target.GetValueKind($setting.Name)
                $currentValue = $target.GetValue($setting.Name, $null, $readOptions)
            }

            $stillOwned = Test-ClipboardValueOwnership $currentPresent $currentValue $currentKind ($appliedPresent -eq 1) $appliedValue $appliedKind
            if ($stillOwned) {
                if ($originalPresent -eq 1) {
                    $originalKind = $backup.GetValueKind('OriginalValue')
                    if ($originalKind -notin @([Microsoft.Win32.RegistryValueKind]::String, [Microsoft.Win32.RegistryValueKind]::DWord)) {
                        throw 'Unsupported original clipboard setting type'
                    }
                    $originalValue = $backup.GetValue('OriginalValue', $null, $readOptions)
                    if ($null -eq $target) { $target = $hkcu.CreateSubKey($setting.Path) }
                    $target.SetValue($setting.Name, $originalValue, $originalKind)
                } elseif ($currentPresent) {
                    $target.DeleteValue($setting.Name, $false)
                }
                if ($setting.Name -eq 'DisabledHotkeys') { $restartExplorer = $true }
                Write-Output "Restored TieZ-owned setting: $($setting.Name)"
            }
            # A later user/policy change owns the current value; leave it untouched.
            $removeSnapshot = $true
        } catch {
            Write-Warning "Kept clipboard setting $($setting.Name): $($_.Exception.Message)"
        } finally {
            if ($null -ne $target) { $target.Dispose() }
            if ($null -ne $backup) { $backup.Dispose() }
        }
        if ($removeSnapshot) {
            try { $hkcu.DeleteSubKey($snapshotPath, $false) } catch { Write-Warning $_.Exception.Message }
        }
    }

    if ($restartExplorer) {
        Stop-Process -Name explorer -Force -ErrorAction SilentlyContinue
        Start-Process -FilePath explorer.exe -WindowStyle Hidden
    }
}

# Dot sourcing exposes the ownership predicate for tests without registry writes.
if ($MyInvocation.InvocationName -ne '.') { Restore-TieZClipboardSettings }
