param([switch]$Build, [string]$Library = (Join-Path $PSScriptRoot '..\LiveRec'), [int]$Port = 4173)
$ErrorActionPreference = 'Stop'
Set-Location -LiteralPath $PSScriptRoot
$cargoCommand = Get-Command cargo -ErrorAction SilentlyContinue
$cargoPath = if ($cargoCommand) { $cargoCommand.Source } else { Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe' }
$binary = Join-Path $PSScriptRoot 'target\debug\u2bup-server.exe'
if ($Build -or -not (Test-Path -LiteralPath $binary)) {
    & npm --prefix web ci --no-audit --no-fund
    if ($LASTEXITCODE -ne 0) { throw 'npm ci failed' }
    & npm --prefix web run build
    if ($LASTEXITCODE -ne 0) { throw 'WebUI build failed' }
    & $cargoPath build -p u2bup-server --locked
    if ($LASTEXITCODE -ne 0) { throw 'Rust build failed' }
}
& $binary --library $Library --port $Port
