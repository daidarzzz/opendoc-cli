# Unattended OpenDoc CLI installer for Windows
# One-line installation:
#   irm https://raw.githubusercontent.com/daidarzzz/opendoc-cli/master/install.ps1 | iex

$ErrorActionPreference = "Stop"

$repo = "daidarzzz/opendoc-cli"
$installDir = Join-Path $HOME ".local\bin"

Write-Host "Installing OpenDoc CLI (odc)..." -ForegroundColor Cyan

# 1. Find the latest release on GitHub
$apiUrl = "https://api.github.com/repos/$repo/releases/latest"
$headers = @{ "User-Agent" = "opendoc-cli-installer" }

try {
    $release = Invoke-RestMethod -Uri $apiUrl -Headers $headers -UseBasicParsing
} catch {
    Write-Error "Could not read the ${repo} release information: $_"
    exit 1
}

$tag = $release.tag_name
$asset = $release.assets | Where-Object { $_.name -like "*x86_64-pc-windows-msvc.zip" } | Select-Object -First 1

if (-not $asset) {
    Write-Error "No Windows x86_64 archive was found in release $tag."
    exit 1
}

Write-Host "Downloading version $tag..." -ForegroundColor Gray

# 2. Download and unpack in a temporary directory
$tempDir = Join-Path ([System.IO.Path]::GetTempPath()) ([System.Guid]::NewGuid().ToString())
New-Item -ItemType Directory -Force -Path $tempDir | Out-Null
$zipPath = Join-Path $tempDir $asset.name

try {
    Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $zipPath -UseBasicParsing
    Expand-Archive -Path $zipPath -DestinationPath $tempDir -Force

    # 3. Create the destination and install both executables
    if (-not (Test-Path $installDir)) {
        New-Item -ItemType Directory -Force -Path $installDir | Out-Null
    }

    $binaries = @("odc.exe", "opendoc-cli.exe")
    foreach ($bin in $binaries) {
        if (-not (Test-Path -LiteralPath (Join-Path $tempDir $bin) -PathType Leaf)) {
            throw "Release archive is missing $bin."
        }
    }
    foreach ($bin in $binaries) {
        Copy-Item -LiteralPath (Join-Path $tempDir $bin) -Destination (Join-Path $installDir $bin) -Force
    }
    $marker = Join-Path $tempDir 'opendoc-cli.release'
    if (Test-Path -LiteralPath $marker -PathType Leaf) {
        Copy-Item -LiteralPath $marker -Destination $installDir -Force
    }
} finally {
    Remove-Item -Recurse -Force -Path $tempDir -ErrorAction SilentlyContinue
}

# 4. Add the installation directory to the user PATH if needed
$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
$pathParts = if ($userPath) { $userPath -split ';' | Where-Object { $_.Trim() -ne "" } } else { @() }

if ($pathParts -notcontains $installDir) {
    $newPath = if ([string]::IsNullOrWhiteSpace($userPath)) { $installDir } else { "$userPath;$installDir" }
    [Environment]::SetEnvironmentVariable("Path", $newPath, "User")
    $env:Path = "$env:Path;$installDir"
    Write-Host "Added to the user PATH: $installDir" -ForegroundColor Green
}

Write-Host ""
Write-Host "Installed OpenDoc CLI $tag in $installDir" -ForegroundColor Green
Write-Host "Run 'odc' or 'opendoc-cli' in your terminal." -ForegroundColor Cyan
