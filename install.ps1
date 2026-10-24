# Script de instalación desatendida para OpenDoc CLI en Windows
# Uso en 1 línea:
#   irm https://raw.githubusercontent.com/daidarzzz/opendoc-cli/master/install.ps1 | iex

$ErrorActionPreference = "Stop"

$repo = "daidarzzz/opendoc-cli"
$installDir = Join-Path $HOME ".local\bin"

Write-Host "Instalando OpenDoc CLI (odc)..." -ForegroundColor Cyan

# 1. Obtener información de la última versión desde GitHub API
$apiUrl = "https://api.github.com/repos/$repo/releases/latest"
$headers = @{ "User-Agent" = "opendoc-cli-installer" }

try {
    $release = Invoke-RestMethod -Uri $apiUrl -Headers $headers -UseBasicParsing
} catch {
    Write-Error "No se pudo obtener la información de releases de $repo: $_"
    exit 1
}

$tag = $release.tag_name
$asset = $release.assets | Where-Object { $_.name -like "*x86_64-pc-windows-msvc.zip" } | Select-Object -First 1

if (-not $asset) {
    Write-Error "No se encontró ningún archivo zip para Windows x86_64 en la release $tag."
    exit 1
}

Write-Host "Descargando versión $tag..." -ForegroundColor Gray

# 2. Directorio temporal de descarga y descompresión
$tempDir = Join-Path ([System.IO.Path]::GetTempPath()) ([System.Guid]::NewGuid().ToString())
New-Item -ItemType Directory -Force -Path $tempDir | Out-Null
$zipPath = Join-Path $tempDir $asset.name

try {
    Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $zipPath -UseBasicParsing
    Expand-Archive -Path $zipPath -DestinationPath $tempDir -Force

    # 3. Asegurar directorio de destino y copiar binarios
    if (-not (Test-Path $installDir)) {
        New-Item -ItemType Directory -Force -Path $installDir | Out-Null
    }

    $binaries = @("odc.exe", "opendoc-cli.exe")
    foreach ($bin in $binaries) {
        $source = Join-Path $tempDir $bin
        if (Test-Path $source) {
            Copy-Item -Path $source -Destination (Join-Path $installDir $bin) -Force
        }
    }
} finally {
    Remove-Item -Recurse -Force -Path $tempDir -ErrorAction SilentlyContinue
}

# 4. Comprobar y actualizar PATH de usuario
$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
$pathParts = if ($userPath) { $userPath -split ';' | Where-Object { $_.Trim() -ne "" } } else { @() }

if ($pathParts -notcontains $installDir) {
    $newPath = if ([string]::IsNullOrWhiteSpace($userPath)) { $installDir } else { "$userPath;$installDir" }
    [Environment]::SetEnvironmentVariable("Path", $newPath, "User")
    $env:Path = "$env:Path;$installDir"
    Write-Host "Ruta añadida al PATH del usuario: $installDir" -ForegroundColor Green
}

Write-Host ""
Write-Host "OpenDoc CLI $tag se ha instalado correctamente en $installDir" -ForegroundColor Green
Write-Host "Ya puedes ejecutar 'odc' o 'opendoc-cli' en tu terminal." -ForegroundColor Cyan
