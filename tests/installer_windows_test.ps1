$ErrorActionPreference = 'Stop'

if ($env:OS -ne 'Windows_NT') {
    Write-Output 'installer windows test: skipped (Windows required)'
    exit 0
}

$repoRoot = Split-Path -Parent $PSScriptRoot
$installer = Join-Path $repoRoot 'install.ps1'
$testRoot = Join-Path ([IO.Path]::GetTempPath()) ("zerobeat-installer-windows-{0}" -f [guid]::NewGuid())
$releaseRoot = Join-Path $testRoot 'release'
$payloadRoot = Join-Path $testRoot 'payload'
$prefix = Join-Path $testRoot 'prefix'
$dataSentinel = Join-Path $prefix 'user-data.keep'

function Assert-True([bool]$condition, [string]$message) {
    if (-not $condition) { throw $message }
}

function Invoke-Installer([string[]]$arguments) {
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $installer @arguments 2>&1 | Out-Null
    $exitCode = $LASTEXITCODE
    return [int]$exitCode
}

try {
    New-Item -ItemType Directory -Path $releaseRoot, $payloadRoot -Force | Out-Null
    New-Item -ItemType Directory -Path $prefix -Force | Out-Null
    Set-Content -LiteralPath (Join-Path $payloadRoot 'zerobeat-cli.exe') -Value 'cli' -NoNewline
    Set-Content -LiteralPath (Join-Path $payloadRoot 'zerobeatd.exe') -Value 'daemon' -NoNewline
    Set-Content -LiteralPath (Join-Path $payloadRoot 'README.md') -Value 'readme' -NoNewline
    Set-Content -LiteralPath (Join-Path $payloadRoot 'LICENSE') -Value 'license' -NoNewline
    Set-Content -LiteralPath (Join-Path $payloadRoot 'avcodec-61.dll') -Value 'dll' -NoNewline

    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $archive = Join-Path $releaseRoot 'zerobeat-windows-x86_64.zip'
    [IO.Compression.ZipFile]::CreateFromDirectory($payloadRoot, $archive)
    $hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $archive).Hash.ToLowerInvariant()
    Set-Content -LiteralPath "$archive.sha256" -Value ("{0}  {1}" -f $hash, (Split-Path -Leaf $archive)) -NoNewline

    $baseUri = ([Uri]::new($releaseRoot)).AbsoluteUri.TrimEnd('/')
    Set-Content -LiteralPath "$archive.sha256" -Value ("{0}  {1}" -f (('0' * 64) -join ''), (Split-Path -Leaf $archive)) -NoNewline
    $exitCode = Invoke-Installer @('-Version', 'v0.1.6', '-Prefix', $prefix, '-ReleaseBaseUrl', $baseUri, '-AllowLocalTest', '-NoPath')
    Assert-True ($exitCode -ne 0) 'installer must reject an invalid archive checksum'
    Set-Content -LiteralPath "$archive.sha256" -Value ("{0}  {1}" -f $hash, (Split-Path -Leaf $archive)) -NoNewline
    Set-Content -LiteralPath (Join-Path $prefix 'avcodec-61.dll') -Value 'unmanaged' -NoNewline
    $exitCode = Invoke-Installer @('-Version', 'v0.1.6', '-Prefix', $prefix, '-ReleaseBaseUrl', $baseUri, '-AllowLocalTest', '-NoPath')
    Assert-True ($exitCode -ne 0) 'installer must not overwrite an unmanaged DLL'
    Assert-True ((Get-Content -LiteralPath (Join-Path $prefix 'avcodec-61.dll') -Raw) -eq 'unmanaged') 'installer modified an unmanaged DLL'
    Remove-Item -LiteralPath (Join-Path $prefix 'avcodec-61.dll') -Force
    $exitCode = Invoke-Installer @('-Version', 'v0.1.6', '-Prefix', $prefix, '-ReleaseBaseUrl', $baseUri, '-AllowLocalTest', '-NoPath')
    Assert-True ($exitCode -eq 0) 'install should succeed with a valid local release fixture'
    foreach ($name in @('zerobeat-cli.exe', 'zerobeatd.exe', 'avcodec-61.dll')) {
        Assert-True (Test-Path (Join-Path $prefix $name)) "installed file is missing: $name"
    }
    Assert-True (Test-Path (Join-Path $prefix 'install-manifest.json')) 'install manifest is missing'
    $manifest = Get-Content -LiteralPath (Join-Path $prefix 'install-manifest.json') -Raw | ConvertFrom-Json
    $installedHash = (Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $prefix 'zerobeat-cli.exe')).Hash.ToLowerInvariant()
    Assert-True ($manifest.files.'zerobeat-cli.exe' -eq $installedHash) 'manifest hash does not match installed CLI'

    Set-Content -LiteralPath $dataSentinel -Value 'preserve me' -NoNewline
    Add-Content -LiteralPath (Join-Path $prefix 'zerobeat-cli.exe') -Value 'modified'
    $exitCode = Invoke-Installer @('-Prefix', $prefix, '-Uninstall', '-NoPath')
    Assert-True ($exitCode -ne 0) 'uninstall must refuse a modified managed file'
    Assert-True (Test-Path (Join-Path $prefix 'zerobeat-cli.exe')) 'refused uninstall removed a modified file'
    Remove-Item -LiteralPath (Join-Path $prefix 'zerobeat-cli.exe') -Force
    Set-Content -LiteralPath (Join-Path $prefix 'zerobeat-cli.exe') -Value 'cli' -NoNewline
    $exitCode = Invoke-Installer @('-Prefix', $prefix, '-Uninstall', '-NoPath')
    Assert-True ($exitCode -eq 0) 'uninstall should succeed when managed files are unchanged'
    Assert-True (Test-Path $dataSentinel) 'uninstall removed user data'
    Assert-True (-not (Test-Path (Join-Path $prefix 'zerobeatd.exe'))) 'uninstall left managed daemon behind'

    $unsafeRoot = Join-Path $testRoot 'unsafe'
    New-Item -ItemType Directory -Path $unsafeRoot -Force | Out-Null
    foreach ($name in @('zerobeat-cli.exe', 'zerobeatd.exe', 'README.md', 'LICENSE')) {
        Set-Content -LiteralPath (Join-Path $unsafeRoot $name) -Value $name -NoNewline
    }
    Set-Content -LiteralPath (Join-Path $unsafeRoot 'unexpected.txt') -Value 'unexpected' -NoNewline
    $unsafeArchive = Join-Path $releaseRoot 'zerobeat-windows-x86_64.zip'
    Remove-Item -LiteralPath $unsafeArchive -Force
    [IO.Compression.ZipFile]::CreateFromDirectory($unsafeRoot, $unsafeArchive)
    $unsafeHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $unsafeArchive).Hash.ToLowerInvariant()
    Set-Content -LiteralPath "$unsafeArchive.sha256" -Value ("{0}  {1}" -f $unsafeHash, (Split-Path -Leaf $unsafeArchive)) -NoNewline
    $exitCode = Invoke-Installer @('-Version', 'v0.1.6', '-Prefix', $prefix, '-ReleaseBaseUrl', $baseUri, '-AllowLocalTest', '-NoPath')
    Assert-True ($exitCode -ne 0) 'installer must reject a ZIP with unexpected entries'

    Write-Output 'installer windows test: passed'
} finally {
    if (Test-Path $testRoot) { Remove-Item -LiteralPath $testRoot -Recurse -Force }
}
