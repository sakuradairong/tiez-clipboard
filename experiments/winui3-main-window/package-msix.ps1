[CmdletBinding()]
param(
    [ValidateSet("Release")]
    [string]$Configuration = "Release",

    [ValidateSet("x64")]
    [string]$Platform = "x64",

    [ValidatePattern('^[A-Za-z0-9.-]{3,50}$')]
    [string]$IdentityName = "TieZ.Community",

    [ValidateNotNullOrEmpty()]
    [string]$Publisher = "CN=TieZ Development",

    [ValidateNotNullOrEmpty()]
    [string]$PublisherDisplayName = "TieZ Community",

    [string]$AppInstallerBaseUri,
    [string]$CertificatePath,
    [string]$CertificatePassword,
    [string]$TimestampUrl,
    [switch]$RequireSigning,
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$Root = Split-Path -Parent $MyInvocation.MyCommand.Path
$RepositoryRoot = [System.IO.Path]::GetFullPath((Join-Path $Root "..\.."))
$ArtifactDirectory = Join-Path $Root "artifacts\$Platform\$Configuration"
$PackageDirectory = Join-Path $Root "artifacts\packages"
$LayoutDirectory = Join-Path $Root "artifacts\msix-layout\$Platform"
$ValidationDirectory = Join-Path $Root "artifacts\msix-validation\$Platform"
$Packages = Join-Path $Root "packages"
$VersionScript = Join-Path $RepositoryRoot "scripts\verify-release-versions.mjs"
$ManifestTemplate = Join-Path $Root "packaging\AppxManifest.xml.in"
$AppInstallerTemplate = Join-Path $Root "packaging\TieZ.appinstaller.in"
$Icons = Join-Path $RepositoryRoot "src-tauri\icons"

function Assert-GeneratedPath {
    param([Parameter(Mandatory)][string]$Path)

    $generatedRoot = [System.IO.Path]::GetFullPath((Join-Path $Root "artifacts")).TrimEnd('\') + '\'
    $resolved = [System.IO.Path]::GetFullPath($Path)
    if (-not $resolved.StartsWith($generatedRoot, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing to modify a generated path outside the WinUI artifacts directory: $resolved"
    }
    return $resolved
}

function Reset-GeneratedDirectory {
    param([Parameter(Mandatory)][string]$Path)

    $resolved = Assert-GeneratedPath $Path
    if (Test-Path -LiteralPath $resolved) {
        Remove-Item -LiteralPath $resolved -Recurse -Force
    }
    New-Item -ItemType Directory -Force -Path $resolved | Out-Null
    return $resolved
}

function Resolve-SdkTool {
    param([Parameter(Mandatory)][string]$Name)

    $tool = Get-ChildItem -LiteralPath $Packages -Recurse -File -Filter $Name -ErrorAction SilentlyContinue |
        Where-Object { $_.FullName -match '\\x64\\' } |
        Sort-Object FullName -Descending |
        Select-Object -First 1
    if (-not $tool) {
        throw "$Name was not found. Run .\build.ps1 once to restore Windows SDK BuildTools."
    }
    return $tool.FullName
}

function Escape-Xml {
    param([Parameter(Mandatory)][string]$Value)
    return [System.Security.SecurityElement]::Escape($Value)
}

function Write-Template {
    param(
        [Parameter(Mandatory)][string]$Template,
        [Parameter(Mandatory)][string]$Destination,
        [Parameter(Mandatory)][hashtable]$Values
    )

    $content = Get-Content -LiteralPath $Template -Raw
    foreach ($entry in $Values.GetEnumerator()) {
        $content = $content.Replace("{{$($entry.Key)}}", (Escape-Xml ([string]$entry.Value)))
    }
    if ($content -match '{{[A-Z0-9_]+}}') {
        throw "Template still contains an unresolved token: $($Matches[0])"
    }
    [System.IO.File]::WriteAllText(
        $Destination,
        $content,
        [System.Text.UTF8Encoding]::new($false)
    )
}

if ($env:OS -ne "Windows_NT") {
    throw "MSIX packaging must run on Windows."
}
if ($RequireSigning) {
    if ([string]::IsNullOrWhiteSpace($CertificatePath)) {
        throw "-RequireSigning needs -CertificatePath. Unsigned release packages are forbidden."
    }
    if ([string]::IsNullOrWhiteSpace($TimestampUrl)) {
        throw "-RequireSigning needs -TimestampUrl so the release signature remains verifiable."
    }
    if ([string]::IsNullOrWhiteSpace($AppInstallerBaseUri)) {
        throw "-RequireSigning needs -AppInstallerBaseUri so the release remains upgradeable."
    }
}
if ($CertificatePath -and -not (Test-Path -LiteralPath $CertificatePath -PathType Leaf)) {
    throw "The signing certificate does not exist: $CertificatePath"
}
if ($AppInstallerBaseUri) {
    $baseUri = $null
    if (-not [System.Uri]::TryCreate($AppInstallerBaseUri, [System.UriKind]::Absolute, [ref]$baseUri) -or
        $baseUri.Scheme -ne "https") {
        throw "AppInstallerBaseUri must be an absolute HTTPS URI."
    }
    $AppInstallerBaseUri = $AppInstallerBaseUri.TrimEnd('/')
}
if ($TimestampUrl) {
    $timestampUri = $null
    if (-not [System.Uri]::TryCreate($TimestampUrl, [System.UriKind]::Absolute, [ref]$timestampUri) -or
        $timestampUri.Scheme -ne "https") {
        throw "TimestampUrl must be an absolute HTTPS URI."
    }
}

$node = Get-Command node.exe -ErrorAction SilentlyContinue
if (-not $node) {
    throw "node.exe was not found. Install Node.js LTS so release versions can be verified."
}
Push-Location -LiteralPath $RepositoryRoot
try {
    $Version = [string](& $node.Source $VersionScript --print)
    $VersionExitCode = $LASTEXITCODE
}
finally {
    Pop-Location
}
$Version = $Version.Trim()
if ($VersionExitCode -ne 0 -or $Version -notmatch '^\d+\.\d+\.\d+$') {
    throw "TieZ release version validation failed."
}
$VersionParts = $Version.Split('.') | ForEach-Object { [int]$_ }
if ($VersionParts.Count -ne 3 -or ($VersionParts | Where-Object { $_ -gt 65535 })) {
    throw "TieZ version cannot be represented by MSIX: $Version"
}
$PackageVersion = "$Version.0"

if (-not $SkipBuild) {
    & (Join-Path $Root "build.ps1") -Configuration $Configuration -Platform $Platform
    if ($LASTEXITCODE -ne 0) {
        throw "WinUI Release build failed."
    }
}

$Executable = Join-Path $ArtifactDirectory "TieZ.exe"
$CoreDll = Join-Path $ArtifactDirectory "tiez_winui_core.dll"
if (-not (Test-Path -LiteralPath $Executable -PathType Leaf) -or
    -not (Test-Path -LiteralPath $CoreDll -PathType Leaf)) {
    throw "The WinUI runtime payload is incomplete. Run .\build.ps1 -Configuration Release."
}

$MakeAppx = Resolve-SdkTool "makeappx.exe"
$SignTool = Resolve-SdkTool "signtool.exe"
$LayoutDirectory = Reset-GeneratedDirectory $LayoutDirectory
$ValidationDirectory = Reset-GeneratedDirectory $ValidationDirectory
$PackageDirectory = Reset-GeneratedDirectory $PackageDirectory

$MsixName = "TieZ_${PackageVersion}_${Platform}.msix"
$MsixPath = Join-Path $PackageDirectory $MsixName
$HashPath = "$MsixPath.sha256"
$AppInstallerName = "TieZ-$Platform.appinstaller"
$AppInstallerPath = Join-Path $PackageDirectory $AppInstallerName

try {
    Copy-Item (Join-Path $ArtifactDirectory "*") $LayoutDirectory -Recurse -Force
    Get-ChildItem -LiteralPath $LayoutDirectory -Recurse -File |
        Where-Object {
            $_.Extension -in @(".pdb", ".lib", ".exp", ".ilk") -or
            $_.Name -in @("ready.txt", "measurement.json")
        } |
        Remove-Item -Force

    $assetDirectory = Join-Path $LayoutDirectory "Assets"
    New-Item -ItemType Directory -Force -Path $assetDirectory | Out-Null
    foreach ($asset in @("StoreLogo.png", "Square44x44Logo.png", "Square150x150Logo.png")) {
        Copy-Item (Join-Path $Icons $asset) (Join-Path $assetDirectory $asset) -Force
    }

    Write-Template $ManifestTemplate (Join-Path $LayoutDirectory "AppxManifest.xml") @{
        IDENTITY_NAME = $IdentityName
        PUBLISHER = $Publisher
        PUBLISHER_DISPLAY_NAME = $PublisherDisplayName
        PACKAGE_VERSION = $PackageVersion
    }

    $forbidden = @(Get-ChildItem -LiteralPath $LayoutDirectory -Recurse -File |
        Where-Object { $_.Name -match 'WebView2|msedgewebview2' })
    if ($forbidden.Count -gt 0) {
        throw "The MSIX payload unexpectedly contains WebView2 files."
    }

    $packOutput = @(& $MakeAppx pack /o /h SHA256 /d $LayoutDirectory /p $MsixPath 2>&1)
    $packExitCode = $LASTEXITCODE
    if ($packExitCode -ne 0 -or -not (Test-Path -LiteralPath $MsixPath -PathType Leaf)) {
        $packOutput | Write-Host
        throw "MakeAppx failed to create the MSIX package."
    }

    $unpackOutput = @(& $MakeAppx unpack /o /p $MsixPath /d $ValidationDirectory 2>&1)
    $unpackExitCode = $LASTEXITCODE
    if ($unpackExitCode -ne 0) {
        $unpackOutput | Write-Host
        throw "MakeAppx could not unpack the generated MSIX package."
    }
    [xml]$packedManifest = Get-Content (Join-Path $ValidationDirectory "AppxManifest.xml") -Raw
    $packedIdentity = $packedManifest.SelectSingleNode("/*[local-name()='Package']/*[local-name()='Identity']")
    if (-not $packedIdentity -or
        $packedIdentity.GetAttribute("Name") -ne $IdentityName -or
        $packedIdentity.GetAttribute("Publisher") -ne $Publisher -or
        $packedIdentity.GetAttribute("Version") -ne $PackageVersion) {
        throw "The packed MSIX identity does not match the requested release identity."
    }
    $fileVirtualization = $packedManifest.SelectSingleNode(
        "/*[local-name()='Package']/*[local-name()='Properties']/*[local-name()='FileSystemWriteVirtualization']"
    )
    $unvirtualizedCapability = $packedManifest.SelectSingleNode(
        "/*[local-name()='Package']/*[local-name()='Capabilities']/*[local-name()='Capability'][@Name='unvirtualizedResources']"
    )
    if (-not $fileVirtualization -or
        $fileVirtualization.NamespaceURI -ne "http://schemas.microsoft.com/appx/manifest/desktop/windows10/6" -or
        $fileVirtualization.InnerText -ne "disabled" -or
        -not $unvirtualizedCapability) {
        throw "The packed MSIX must keep TieZ AppData unvirtualized for Tauri-compatible storage."
    }
    if (-not (Test-Path (Join-Path $ValidationDirectory "TieZ.exe") -PathType Leaf) -or
        -not (Test-Path (Join-Path $ValidationDirectory "tiez_winui_core.dll") -PathType Leaf)) {
        throw "The packed MSIX is missing TieZ.exe or tiez_winui_core.dll."
    }

    if ($CertificatePath) {
        $certificate = [System.Security.Cryptography.X509Certificates.X509Certificate2]::new(
            (Resolve-Path -LiteralPath $CertificatePath).Path,
            $CertificatePassword,
            [System.Security.Cryptography.X509Certificates.X509KeyStorageFlags]::EphemeralKeySet
        )
        $expectedSubject = [System.Security.Cryptography.X509Certificates.X500DistinguishedName]::new($Publisher).Name
        if ($certificate.SubjectName.Name -ne $expectedSubject) {
            throw "The MSIX Publisher must exactly match the signing certificate subject."
        }
        $signArguments = @("sign", "/fd", "SHA256", "/a", "/f", $CertificatePath)
        if ($CertificatePassword) {
            $signArguments += @("/p", $CertificatePassword)
        }
        if ($TimestampUrl) {
            $signArguments += @("/tr", $TimestampUrl, "/td", "SHA256")
        }
        $signArguments += $MsixPath
        & $SignTool @signArguments
        if ($LASTEXITCODE -ne 0) {
            throw "SignTool failed to sign the MSIX package."
        }
        & $SignTool verify /pa /v $MsixPath
        if ($LASTEXITCODE -ne 0) {
            throw "SignTool could not verify the signed MSIX package."
        }
    }

    $hash = (Get-FileHash -LiteralPath $MsixPath -Algorithm SHA256).Hash.ToLowerInvariant()
    [System.IO.File]::WriteAllText(
        $HashPath,
        "$hash  $MsixName`r`n",
        [System.Text.Encoding]::ASCII
    )

    if ($AppInstallerBaseUri) {
        Write-Template $AppInstallerTemplate $AppInstallerPath @{
            APPINSTALLER_URI = "$AppInstallerBaseUri/$AppInstallerName"
            PACKAGE_URI = "$AppInstallerBaseUri/$MsixName"
            IDENTITY_NAME = $IdentityName
            PUBLISHER = $Publisher
            PACKAGE_VERSION = $PackageVersion
        }
    }
} finally {
    foreach ($directory in @($LayoutDirectory, $ValidationDirectory)) {
        if (Test-Path -LiteralPath $directory) {
            Remove-Item -LiteralPath $directory -Recurse -Force
        }
    }
}

Write-Host ""
Write-Host "MSIX: $MsixPath" -ForegroundColor Green
Write-Host "SHA256: $HashPath" -ForegroundColor Green
if ($AppInstallerBaseUri) {
    Write-Host "App Installer: $AppInstallerPath" -ForegroundColor Green
}
if (-not $CertificatePath) {
    Write-Warning "The package is unsigned and is for build validation only. Release publishing must use -RequireSigning."
}
