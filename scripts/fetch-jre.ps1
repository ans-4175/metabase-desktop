# Download Eclipse Temurin JRE for Windows + extract into src-tauri/resources/runtime/
# Usage:  pwsh scripts/fetch-jre.ps1                              # JRE 21 (default)
#         $env:JAVA_VERSION = "25"; pwsh scripts/fetch-jre.ps1    # specific version
$ErrorActionPreference = "Stop"

$RepoRoot = Split-Path -Parent $PSScriptRoot
$JavaVersion = if ($env:JAVA_VERSION) { $env:JAVA_VERSION } else { "21" }

$Arch = "x64"
if ($env:PROCESSOR_ARCHITECTURE -match "ARM") { $Arch = "aarch64" }

$Url = "https://api.adoptium.net/v3/binary/latest/$JavaVersion/ga/windows/$Arch/jre/hotspot/normal/eclipse"
$Tmp = Join-Path ([System.IO.Path]::GetTempPath()) ([Guid]::NewGuid().ToString())
New-Item -ItemType Directory -Path $Tmp | Out-Null

Write-Host ">> Downloading Temurin JRE $JavaVersion (windows/$Arch)"
Invoke-WebRequest -Uri $Url -OutFile "$Tmp/jre.zip"

Write-Host ">> Extracting..."
Expand-Archive -Path "$Tmp/jre.zip" -DestinationPath $Tmp -Force

$Src = Get-ChildItem -Path $Tmp -Directory |
    Where-Object { Test-Path (Join-Path $_.FullName "bin/java.exe") } |
    Select-Object -First 1
if (-not $Src) {
    Write-Error "Could not find java.exe in the extracted JRE"
    exit 1
}

$Dest = Join-Path $RepoRoot "src-tauri/resources/runtime"
if (Test-Path $Dest) { Remove-Item -Recurse -Force $Dest }
New-Item -ItemType Directory -Path $Dest | Out-Null
Copy-Item -Path (Join-Path $Src.FullName "*") -Destination $Dest -Recurse -Force
Remove-Item -Recurse -Force $Tmp

Write-Host ">> JRE installed at $Dest"
