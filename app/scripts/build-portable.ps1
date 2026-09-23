param([string]$FfmpegDir = 'C:\Users\Administrator\scoop\apps\ffmpeg\current')
$ErrorActionPreference = 'Stop'
$applicationRoot = Split-Path -Parent $PSScriptRoot
Set-Location -LiteralPath $applicationRoot
$cargoCommand = Get-Command cargo -ErrorAction SilentlyContinue
$cargoPath = if ($cargoCommand) { $cargoCommand.Source } else { Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe' }
& npm --prefix web ci --no-audit --no-fund
if ($LASTEXITCODE -ne 0) { throw 'npm ci failed' }
& npm --prefix web run build
if ($LASTEXITCODE -ne 0) { throw 'WebUI build failed' }
& (Join-Path $PSScriptRoot 'stage-media.ps1') -FfmpegDir $FfmpegDir
& $cargoPath build --release -p u2bup-desktop -p u2bup-server --locked
if ($LASTEXITCODE -ne 0) { throw 'Rust build failed' }
$versionMatch = Select-String -LiteralPath (Join-Path $applicationRoot 'Cargo.toml') -Pattern '^version = "(\d+\.\d+\.\d+)"$' | Select-Object -First 1
if (-not $versionMatch) { throw 'Workspace version not found in Cargo.toml' }
$version = $versionMatch.Matches[0].Groups[1].Value
$destination = Join-Path $applicationRoot "dist\U2BUP-$version-windows-x64"
New-Item -ItemType Directory -Path $destination -Force | Out-Null
Copy-Item -LiteralPath 'target\release\u2bup-desktop.exe' -Destination (Join-Path $destination 'U2BUP.exe') -Force
Copy-Item -LiteralPath 'target\release\u2bup-server.exe' -Destination (Join-Path $destination 'u2bup-server.exe') -Force
$mediaDestination = Join-Path $destination 'resources'
New-Item -ItemType Directory -Path $mediaDestination -Force | Out-Null
Get-ChildItem -LiteralPath 'desktop\resources' -File | ForEach-Object { Copy-Item -LiteralPath $_.FullName -Destination (Join-Path $mediaDestination $_.Name) -Force }
Copy-Item -LiteralPath 'README.md' -Destination (Join-Path $destination 'README.md') -Force
Copy-Item -LiteralPath 'THIRD-PARTY-NOTICES.md' -Destination (Join-Path $destination 'THIRD-PARTY-NOTICES.md') -Force
Copy-Item -LiteralPath 'scripts\Start-PortableWeb.ps1' -Destination (Join-Path $destination 'Start-Web.ps1') -Force
Write-Output "Portable folder: $destination"
