param(
    [string]$FfmpegDir = 'C:\Users\Administrator\scoop\apps\ffmpeg\current'
)
$ErrorActionPreference = 'Stop'
$applicationRoot = Split-Path -Parent $PSScriptRoot
$resourceDir = Join-Path $applicationRoot 'desktop\resources'
New-Item -ItemType Directory -Path $resourceDir -Force | Out-Null
$components = @()
foreach ($binary in @('ffmpeg.exe', 'ffprobe.exe')) {
    $source = Join-Path $FfmpegDir "bin\$binary"
    if (-not (Test-Path -LiteralPath $source)) { throw "Missing binary: $source" }
    $target = Join-Path $resourceDir $binary
    Copy-Item -LiteralPath $source -Destination $target -Force
    $version = (& $source -version | Select-Object -First 1)
    $components += [ordered]@{ name=$binary; version=$version; sha256=(Get-FileHash -LiteralPath $target -Algorithm SHA256).Hash; bytes=(Get-Item -LiteralPath $target).Length }
}
foreach ($license in @('LICENSE', 'LICENSE.txt', 'COPYING.GPLv3', 'COPYING.LGPLv3')) {
    $source = Join-Path $FfmpegDir $license
    if (Test-Path -LiteralPath $source) { Copy-Item -LiteralPath $source -Destination (Join-Path $resourceDir "FFmpeg-$license") -Force }
}
[ordered]@{ platform='windows-x86_64'; stagedAt=(Get-Date).ToUniversalTime().ToString('o'); source='Existing local Gyan FFmpeg distribution'; components=$components } | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $resourceDir 'media-manifest.json') -Encoding utf8
Write-Output "Staged bundled media tools: $resourceDir"
