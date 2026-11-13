# Guardian Server - build script
#
# Produces one self-contained GUI executable, GuardianServer.exe, with an
# embedded manifest that requests administrator rights (it adds a Windows
# Firewall rule and keeps its data under ProgramData).
#
# 32-bit on purpose, like Guardian Console: a 32-bit executable runs on both
# 32- and 64-bit Windows. scripts/package.sh does the same build on Linux.

param(
    [string]$OutDir = ""
)

$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

if (-not $OutDir) {
    $OutDir = Join-Path $PSScriptRoot "..\..\dist\server-windows"
}
if (-not (Test-Path $OutDir)) {
    New-Item -ItemType Directory -Path $OutDir -Force | Out-Null
}

# The .syso is deleted afterwards: the linker adds it to every binary in the
# package, including the test binary, which would then demand elevation.
$syso = "rsrc_windows_386.syso"
try {
    Write-Host "Generating $syso ..." -ForegroundColor Yellow
    go run github.com/akavel/rsrc@v0.10.2 -manifest server-gui.manifest -ico guardian.ico -arch 386 -o $syso
    if ($LASTEXITCODE -ne 0) { throw "rsrc failed" }

    Write-Host "Building GuardianServer.exe (386) ..." -ForegroundColor Yellow
    $env:GOOS = "windows"
    $env:GOARCH = "386"
    $env:CGO_ENABLED = "0"
    $out = Join-Path $OutDir "GuardianServer.exe"
    go build -trimpath -ldflags "-s -w -H windowsgui" -o $out
    if ($LASTEXITCODE -ne 0) { throw "go build failed" }

    Write-Host "      -> $out" -ForegroundColor Green
} catch {
    Write-Host "Build failed: $($_.Exception.Message)" -ForegroundColor Red
    exit 1
} finally {
    if (Test-Path $syso) { Remove-Item $syso -Force }
}
