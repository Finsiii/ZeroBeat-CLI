[CmdletBinding()]
param(
    [ValidatePattern('^(latest|v[0-9]+\.[0-9]+\.[0-9]+)$')]
    [string]$Version = 'latest',
    [string]$Prefix = '',
    [switch]$Uninstall,
    [switch]$NoPath,
    [string]$ReleaseBaseUrl = '',
    [switch]$AllowLocalTest
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$Repository = 'Finsiii/ZeroBeat-CLI'
$ArtifactName = 'zerobeat-windows-x86_64.zip'
$ManifestName = 'install-manifest.json'
$FixedEntries = @('zerobeat-cli.exe', 'zerobeatd.exe', 'README.md', 'LICENSE')

function Fail([string]$Message) {
    throw "install.ps1: $Message"
}

function Test-ReparsePoint([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path)) { return $false }
    $item = Get-Item -LiteralPath $Path -Force
    return (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0)
}

function Assert-SafePath([string]$Path) {
    if (Test-ReparsePoint $Path) { Fail "refusing reparse point: $Path" }
}

function Assert-SafePathChain([string]$Path) {
    $current = [IO.Path]::GetFullPath($Path)
    while ($current) {
        Assert-SafePath $current
        $parent = Split-Path -Parent $current
        if (-not $parent -or [StringComparer]::OrdinalIgnoreCase.Equals($parent, $current)) { break }
        $current = $parent
    }
}

function Get-FullPrefix {
    if ([string]::IsNullOrWhiteSpace($Prefix)) {
        if ([string]::IsNullOrWhiteSpace($env:LOCALAPPDATA)) {
            Fail 'LOCALAPPDATA is not set; provide -Prefix explicitly'
        }
        $script:Prefix = Join-Path $env:LOCALAPPDATA 'Programs\ZeroBeat'
    }
    try { $script:Prefix = [IO.Path]::GetFullPath($Prefix) } catch { Fail 'Prefix is not a valid path' }
    Assert-SafePathChain $Prefix
}

function Get-BaseUri {
    if ($ReleaseBaseUrl) {
        if ($AllowLocalTest -and $ReleaseBaseUrl.StartsWith('file://', [StringComparison]::OrdinalIgnoreCase)) {
            return [Uri]::new($ReleaseBaseUrl.TrimEnd('/'))
        }
        if (-not $ReleaseBaseUrl.StartsWith("https://github.com/$Repository/releases/", [StringComparison]::OrdinalIgnoreCase)) {
            Fail 'ReleaseBaseUrl must be the ZeroBeat GitHub HTTPS release URL'
        }
        return [Uri]::new($ReleaseBaseUrl.TrimEnd('/'))
    }
    if ($AllowLocalTest) { Fail 'AllowLocalTest requires a file:// ReleaseBaseUrl' }
    if ($Version -eq 'latest') {
        return [Uri]"https://github.com/$Repository/releases/latest/download"
    }
    return [Uri]"https://github.com/$Repository/releases/download/$Version"
}

function Get-ReleaseUri([Uri]$BaseUri, [string]$Name) {
    return [Uri]::new($BaseUri.AbsoluteUri.TrimEnd('/') + '/' + [Uri]::EscapeDataString($Name))
}

function Save-ReleaseFile([Uri]$Uri, [string]$Destination) {
    if ($Uri.Scheme -eq 'file') {
        Copy-Item -LiteralPath $Uri.LocalPath -Destination $Destination -Force
        return
    }
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    Invoke-WebRequest -UseBasicParsing -MaximumRedirection 5 -Uri $Uri.AbsoluteUri -OutFile $Destination
}

function Read-ArchiveChecksum([string]$Path) {
    $lines = @(Get-Content -LiteralPath $Path)
    if ($lines.Count -ne 1) { Fail 'checksum file must contain exactly one entry' }
    if ($lines[0] -notmatch '^(?<hash>[0-9A-Fa-f]{64})\s+\*?(?<name>[^\r\n]+)$') {
        Fail 'checksum file does not contain a SHA-256 digest'
    }
    if ($Matches.name -ne $ArtifactName) { Fail 'checksum file names a different archive' }
    return $Matches.hash.ToLowerInvariant()
}

function Get-ArchiveEntries([string]$ArchivePath) {
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $zip = [IO.Compression.ZipFile]::OpenRead($ArchivePath)
    try {
        $entries = @()
        foreach ($entry in $zip.Entries) {
            if ([string]::IsNullOrEmpty($entry.Name)) { Fail 'archive contains a directory entry' }
            $name = $entry.FullName.Replace('\', '/')
            if ($name -match '(^/|:/|(^|/)\.\.?(/|$)|/$)' -or $name.Contains('/')) {
                Fail "archive contains an unsafe path: $name"
            }
            if (($FixedEntries -notcontains $name) -and ($name -notmatch '^[A-Za-z0-9][A-Za-z0-9._-]*\.dll$')) {
                Fail "archive contains unexpected path: $name"
            }
            if ($entries -contains $name) { Fail "archive contains duplicate path: $name" }
            $entries += $name
        }
        foreach ($required in $FixedEntries) {
            if ($entries -notcontains $required) { Fail "archive is missing $required" }
        }
        return $entries
    } finally {
        $zip.Dispose()
    }
}

function Expand-SafeArchive([string]$ArchivePath, [string]$Destination, [string[]]$Entries) {
    $zip = [IO.Compression.ZipFile]::OpenRead($ArchivePath)
    try {
        foreach ($name in $Entries) {
            $entry = $zip.GetEntry($name)
            $target = Join-Path $Destination $name
            [IO.Compression.ZipFileExtensions]::ExtractToFile($entry, $target, $false)
        }
    } finally {
        $zip.Dispose()
    }
}

function Get-ManagedNames([object]$Manifest) {
    if ($null -eq $Manifest -or $Manifest.format_version -ne 1 -or $null -eq $Manifest.files) {
        Fail 'installer manifest is invalid'
    }
    $names = @($Manifest.files.psobject.Properties | ForEach-Object { $_.Name })
    foreach ($required in $FixedEntries) {
        if ($names -notcontains $required) { Fail "installer manifest is missing $required" }
    }
    foreach ($name in $names) {
        if (($FixedEntries -notcontains $name) -and ($name -notmatch '^[A-Za-z0-9][A-Za-z0-9._-]*\.dll$')) {
            Fail "installer manifest contains an unexpected path: $name"
        }
        $property = $Manifest.files.psobject.Properties | Where-Object { $_.Name -eq $name }
        $digest = [string]$property.Value
        if ($digest -notmatch '^[0-9A-Fa-f]{64}$') { Fail "installer manifest has an invalid hash: $name" }
    }
    return $names
}

function Get-ManifestHash([object]$Manifest, [string]$Name) {
    $property = $Manifest.files.psobject.Properties | Where-Object { $_.Name -eq $Name }
    return [string]$property.Value
}

function Read-ExistingManifest {
    $path = Join-Path $Prefix $ManifestName
    if (-not (Test-Path -LiteralPath $path)) { return $null }
    Assert-SafePath $path
    try { return Get-Content -LiteralPath $path -Raw | ConvertFrom-Json } catch { Fail 'installer manifest is not valid JSON' }
}

function Assert-ManifestFiles([object]$Manifest) {
    $names = Get-ManagedNames $Manifest
    foreach ($name in $names) {
        $path = Join-Path $Prefix $name
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { Fail "managed file is missing: $name" }
        Assert-SafePath $path
        $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $path).Hash
        $expected = (Get-ManifestHash $Manifest $name).ToUpperInvariant()
        if ($actual.ToUpperInvariant() -ne $expected) { Fail "managed file was modified: $name" }
    }
}

function Add-UserPath([string]$Path) {
    $current = [Environment]::GetEnvironmentVariable('Path', 'User')
    $parts = @()
    if ($current) { $parts = @($current -split ';' | Where-Object { $_ }) }
    foreach ($part in $parts) {
        if ([StringComparer]::OrdinalIgnoreCase.Equals($part.TrimEnd('\'), $Path.TrimEnd('\'))) { return $false }
    }
    $parts += $Path
    [Environment]::SetEnvironmentVariable('Path', ($parts -join ';'), 'User')
    return $true
}

function Remove-UserPath([string]$Path) {
    $current = [Environment]::GetEnvironmentVariable('Path', 'User')
    if (-not $current) { return }
    $parts = @($current -split ';' | Where-Object {
        $_ -and -not [StringComparer]::OrdinalIgnoreCase.Equals($_.TrimEnd('\'), $Path.TrimEnd('\'))
    })
    [Environment]::SetEnvironmentVariable('Path', ($parts -join ';'), 'User')
}

function Invoke-Uninstall {
    $manifest = Read-ExistingManifest
    if ($null -eq $manifest) { Fail 'installer manifest is missing; refusing uninstall' }
    Assert-ManifestFiles $manifest
    $names = Get-ManagedNames $manifest
    foreach ($name in $names) { Remove-Item -LiteralPath (Join-Path $Prefix $name) -Force }
    Remove-Item -LiteralPath (Join-Path $Prefix $ManifestName) -Force
    if ($manifest.path_added -eq $true) { Remove-UserPath $Prefix }
    Write-Output 'Removed ZeroBeat executables and installer manifest; user data was preserved.'
}

function Invoke-Install {
    $existing = Read-ExistingManifest
    if ($null -ne $existing) {
        Assert-ManifestFiles $existing
    } elseif ((Test-Path -LiteralPath (Join-Path $Prefix 'zerobeat-cli.exe')) -or
              (Test-Path -LiteralPath (Join-Path $Prefix 'zerobeatd.exe'))) {
        Fail 'existing destination binary has no valid installer manifest; refusing overwrite'
    }

    $transaction = Join-Path ([IO.Path]::GetTempPath()) ("zerobeat-install-{0}" -f [guid]::NewGuid())
    $stage = Join-Path $transaction 'stage'
    $backup = Join-Path $transaction 'backup'
    $entries = @()
    $installedNames = @()
    $pathManaged = $null -ne $existing -and $existing.path_added -eq $true
    $pathAddedThisRun = $false
    $preserveTransaction = $false
    New-Item -ItemType Directory -Path $stage, $backup -Force | Out-Null
    try {
        $base = Get-BaseUri
        $archivePath = Join-Path $transaction $ArtifactName
        $checksumPath = Join-Path $transaction "$ArtifactName.sha256"
        Save-ReleaseFile (Get-ReleaseUri $base $ArtifactName) $archivePath
        Save-ReleaseFile (Get-ReleaseUri $base "$ArtifactName.sha256") $checksumPath
        $expected = Read-ArchiveChecksum $checksumPath
        $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $archivePath).Hash.ToLowerInvariant()
        if ($actual -ne $expected) { Fail 'archive SHA-256 verification failed' }
        $entries = Get-ArchiveEntries $archivePath
        Expand-SafeArchive $archivePath $stage $entries

        if ($null -eq $existing) {
            foreach ($name in @($entries + $ManifestName)) {
                if (Test-Path -LiteralPath (Join-Path $Prefix $name)) {
                    Fail "destination contains an unmanaged file: $name"
                }
            }
        }

        $hashes = [ordered]@{}
        foreach ($name in $entries) {
            $hashes[$name] = (Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $stage $name)).Hash.ToLowerInvariant()
        }
        if (-not $NoPath -and -not $pathManaged) {
            try {
                $pathAddedThisRun = Add-UserPath $Prefix
                $pathManaged = $pathAddedThisRun
            } catch {
                Write-Warning "could not add $Prefix to the per-user PATH"
            }
        }
        $manifest = [ordered]@{ format_version = 1; version = $Version; path_added = $pathManaged; files = $hashes }
        $manifestStage = Join-Path $stage $ManifestName
        $manifest | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $manifestStage -Encoding UTF8

        if (-not (Test-Path -LiteralPath $Prefix)) { New-Item -ItemType Directory -Path $Prefix -Force | Out-Null }
        Assert-SafePathChain $Prefix
        $oldNames = @()
        if ($null -ne $existing) { $oldNames = Get-ManagedNames $existing }
        $toBackup = @($oldNames + $ManifestName | Select-Object -Unique)
        foreach ($name in $toBackup) {
            $destination = Join-Path $Prefix $name
            if (Test-Path -LiteralPath $destination) { Move-Item -LiteralPath $destination -Destination (Join-Path $backup $name) -Force }
        }
        foreach ($name in @($entries + $ManifestName)) {
            Move-Item -LiteralPath (Join-Path $stage $name) -Destination (Join-Path $Prefix $name) -Force
            $installedNames += $name
        }
        Write-Output "Installed ZeroBeat ($Version) to $Prefix"
        if (-not $NoPath) { Write-Output 'Open a new terminal to use zerobeat-cli.exe from PATH.' }
    } catch {
        $installError = $_
        try {
            foreach ($name in $installedNames) {
                $destination = Join-Path $Prefix $name
                if (Test-Path -LiteralPath $destination) { Remove-Item -LiteralPath $destination -Force }
            }
            if (Test-Path -LiteralPath $backup) {
                foreach ($item in Get-ChildItem -LiteralPath $backup -Force) {
                    Move-Item -LiteralPath $item.FullName -Destination (Join-Path $Prefix $item.Name) -Force
                }
            }
            if ($pathAddedThisRun) { Remove-UserPath $Prefix }
        } catch {
            $preserveTransaction = $true
            throw "install failed and rollback also failed; recovery files remain at ${transaction}: $($_.Exception.Message)"
        }
        throw $installError
    } finally {
        if (-not $preserveTransaction -and (Test-Path -LiteralPath $transaction)) {
            Remove-Item -LiteralPath $transaction -Recurse -Force
        }
    }
}

Get-FullPrefix
$processArchitecture = if ($env:PROCESSOR_ARCHITEW6432) { $env:PROCESSOR_ARCHITEW6432 } else { $env:PROCESSOR_ARCHITECTURE }
if ($env:OS -ne 'Windows_NT' -or -not [Environment]::Is64BitOperatingSystem -or $processArchitecture -ne 'AMD64') {
    Fail 'this release supports Windows x64 only'
}
if ($Uninstall) { Invoke-Uninstall } else { Invoke-Install }
