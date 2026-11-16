$ErrorActionPreference = 'Stop'
$stage = $env:ODC_UPDATE_STAGE
$destination = $env:ODC_UPDATE_DIR
$backup = Join-Path $stage 'previous'
$executables = @('odc.exe', 'opendoc-cli.exe')
$replaced = [System.Collections.Generic.List[string]]::new()
$lock = $null
$keepBackup = $false

try {
    $parent = Get-Process -Id ([int]$env:ODC_UPDATE_PARENT) -ErrorAction SilentlyContinue
    if ($parent -and -not $parent.WaitForExit(15000)) {
        throw 'The running CLI did not exit in time.'
    }
    $lockPath = Join-Path $destination '.odc-update.lock'
    $lock = [System.IO.File]::Open($lockPath, 'OpenOrCreate', 'ReadWrite', 'None')
    $version = & (Join-Path $destination 'odc.exe') --version
    if ($LASTEXITCODE -ne 0 -or $version -notlike "* $env:ODC_UPDATE_CURRENT") {
        throw 'The installed version changed during the update.'
    }
    New-Item -ItemType Directory -Path $backup | Out-Null
    foreach ($name in $executables) {
        Copy-Item -LiteralPath (Join-Path $destination $name) -Destination $backup
    }
    foreach ($name in $executables) {
        [System.IO.File]::Replace((Join-Path $stage $name), (Join-Path $destination $name), (Join-Path $stage "$name.replaced"))
        $replaced.Add($name)
    }
    Write-Host "Updated OpenDoc CLI to $env:ODC_UPDATE_LATEST."
} catch {
    $failure = $_
    foreach ($name in $replaced) {
        try {
            [System.IO.File]::Replace((Join-Path $backup $name), (Join-Path $destination $name), (Join-Path $stage "$name.failed"))
        } catch {
            $keepBackup = $true
            [Console]::Error.WriteLine("Could not restore $name. Backup: $backup")
        }
    }
    [Console]::Error.WriteLine("OpenDoc CLI update failed: $failure")
    exit 1
} finally {
    if ($lock) { $lock.Dispose() }
    if (-not $keepBackup) {
        Remove-Item -LiteralPath $stage -Recurse -Force -ErrorAction SilentlyContinue
    }
}
