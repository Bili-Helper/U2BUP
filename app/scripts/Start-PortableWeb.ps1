param([Parameter(Mandatory=$true)][string]$Library,[int]$Port=4173)
$ErrorActionPreference='Stop'
Set-Location -LiteralPath $PSScriptRoot
& '.\u2bup-server.exe' --library $Library --data (Join-Path $PSScriptRoot 'data') --output (Join-Path $PSScriptRoot 'exports') --port $Port --ffmpeg (Join-Path $PSScriptRoot 'resources\ffmpeg.exe') --ffprobe (Join-Path $PSScriptRoot 'resources\ffprobe.exe')
