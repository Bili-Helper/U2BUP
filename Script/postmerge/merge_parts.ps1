#!/usr/bin/env pwsh
<#
.SYNOPSIS
    将 merged 目录中同一场、同一画幅的 _partN 文件合并为单个 MP4。
.DESCRIPTION
    按 part 数字顺序合并，校验编号、编码、时长和输出大小。
    验证成功后默认把原 part 移入 parts_backup；使用 -DeleteParts 才会删除。
#>

param(
    [Parameter(Mandatory = $true)]
    [string]$SourceDir,
    [switch]$DryRun,
    [switch]$DeleteParts,
    [int]$MinFreeSpaceGB = 2
)

$ffmpegBin = "C:\Users\Administrator\scoop\apps\ffmpeg\8.0\bin\ffmpeg.exe"
$ffprobeBin = "C:\Users\Administrator\scoop\apps\ffmpeg\8.0\bin\ffprobe.exe"
if (-not (Test-Path -LiteralPath $ffmpegBin)) { throw "ffmpeg not found at $ffmpegBin" }
if (-not (Test-Path -LiteralPath $ffprobeBin)) { throw "ffprobe not found at $ffprobeBin" }

$resolvedSource = (Resolve-Path -LiteralPath $SourceDir).Path
$nestedMerged = Join-Path $resolvedSource "merged"
if (Test-Path -LiteralPath $nestedMerged -PathType Container) {
    $mergedDir = $nestedMerged
} elseif ((Split-Path $resolvedSource -Leaf) -eq "merged") {
    $mergedDir = $resolvedSource
} else {
    throw "找不到 merged 目录: $SourceDir"
}

function Get-MediaInfo {
    param([Parameter(Mandatory = $true)][string]$Path)

    $jsonText = & $ffprobeBin -v error `
        -show_entries "stream=codec_type,codec_name,profile,width,height,pix_fmt,sample_rate,channels" `
        -show_entries "format=duration" -of json -- $Path 2>&1 | Out-String
    if ($LASTEXITCODE -ne 0) { return $null }

    try {
        $json = $jsonText | ConvertFrom-Json
        $video = $json.streams | Where-Object codec_type -eq "video" | Select-Object -First 1
        $audio = $json.streams | Where-Object codec_type -eq "audio" | Select-Object -First 1
        $duration = [double]$json.format.duration
        if ($null -eq $video -or $null -eq $audio -or $duration -le 0) { return $null }

        return [PSCustomObject]@{
            Duration = $duration
            Signature = "$($video.codec_name)|$($video.profile)|$($video.width)x$($video.height)|$($video.pix_fmt)|$($audio.codec_name)|$($audio.sample_rate)|$($audio.channels)"
        }
    } catch {
        return $null
    }
}

$records = @(
    foreach ($path in [System.IO.Directory]::EnumerateFiles($mergedDir, "*.mp4", [System.IO.SearchOption]::TopDirectoryOnly)) {
        $name = [System.IO.Path]::GetFileName($path)
        if ($name -match '^(.*)_part(\d+)\.mp4$') {
            [PSCustomObject]@{
                BaseName   = $Matches[1]
                PartNumber = [int]$Matches[2]
                Path       = $path
                Name       = $name
                Length     = [System.IO.FileInfo]::new($path).Length
            }
        }
    }
)

$groups = @($records | Group-Object BaseName | Sort-Object Name)
if ($groups.Count -eq 0) {
    Write-Host "没有找到需要合并的 part 文件" -ForegroundColor Green
    exit 0
}

Write-Host "找到 $($groups.Count) 组，共 $($records.Count) 个 part 文件" -ForegroundColor Cyan
$completed = 0
$failed = 0
$archived = 0
$deleted = 0

foreach ($group in $groups) {
    $parts = @($group.Group | Sort-Object PartNumber)
    $numbers = @($parts.PartNumber)
    $expected = @(1..$parts.Count)
    if (($numbers -join ',') -ne ($expected -join ',')) {
        Write-Warning "跳过编号不连续的组: $($group.Name)；实际为 $($numbers -join ',')"
        $failed++
        continue
    }

    $outputName = "$($group.Name).mp4"
    $outputPath = Join-Path $mergedDir $outputName
    $totalBytes = ($parts | Measure-Object Length -Sum).Sum
    Write-Host "`n合并 $($parts.Count) 个 part（$([math]::Round($totalBytes / 1GB, 2))GB）→ $outputName" -ForegroundColor Green

    if (Test-Path -LiteralPath $outputPath) {
        Write-Warning "目标已经存在，保留原 part 并跳过: $outputName"
        $failed++
        continue
    }

    $partInfos = @()
    foreach ($part in $parts) {
        $info = Get-MediaInfo -Path $part.Path
        if ($null -eq $info) { break }
        $partInfos += $info
    }
    if ($partInfos.Count -ne $parts.Count) {
        Write-Warning "输入文件探测失败，保留原 part 并跳过: $outputName"
        $failed++
        continue
    }

    $signatures = @($partInfos.Signature | Select-Object -Unique)
    if ($signatures.Count -ne 1) {
        Write-Warning "part 编码参数不一致，不能安全流复制: $outputName"
        $failed++
        continue
    }

    $expectedDuration = ($partInfos | Measure-Object Duration -Sum).Sum
    if ($DryRun) {
        Write-Host "  DryRun：预计时长 $([timespan]::FromSeconds($expectedDuration).ToString('hh\:mm\:ss'))，不执行" -ForegroundColor Yellow
        continue
    }

    $driveRoot = [System.IO.Path]::GetPathRoot($mergedDir)
    $available = [System.IO.DriveInfo]::new($driveRoot).AvailableFreeSpace
    $reserve = [int64]$MinFreeSpaceGB * 1GB
    if ($available -lt $totalBytes + $reserve) {
        Write-Warning "空间不足，保留原 part 并跳过；需要约 $([math]::Round($totalBytes/1GB, 2))GB 并保留 ${MinFreeSpaceGB}GB"
        $failed++
        continue
    }

    $guid = [System.Guid]::NewGuid().ToString('N')
    $listPath = Join-Path $mergedDir "_concat_parts_${guid}.txt"
    $tempPath = Join-Path $mergedDir "_building_parts_${guid}.mp4"
    $parts | ForEach-Object {
        $escapedPath = $_.Path -replace "'", "'\''"
        "file '$escapedPath'"
    } | Out-File -LiteralPath $listPath -Encoding UTF8NoBOM

    $result = & $ffmpegBin -fflags +genpts -f concat -safe 0 -i $listPath `
        -c copy -avoid_negative_ts 1 -y $tempPath 2>&1
    $exitCode = $LASTEXITCODE
    Remove-Item -LiteralPath $listPath -Force -ErrorAction SilentlyContinue

    $outputInfo = if ($exitCode -eq 0 -and (Test-Path -LiteralPath $tempPath)) {
        Get-MediaInfo -Path $tempPath
    } else { $null }
    $tempBytes = if (Test-Path -LiteralPath $tempPath) { [System.IO.FileInfo]::new($tempPath).Length } else { 0 }
    $durationTolerance = [math]::Max(5.0, $expectedDuration * 0.01)
    $durationOk = ($null -ne $outputInfo -and [math]::Abs($outputInfo.Duration - $expectedDuration) -le $durationTolerance)
    $signatureOk = ($null -ne $outputInfo -and $outputInfo.Signature -eq $signatures[0])
    $sizeOk = ($tempBytes -ge $totalBytes * 0.85)

    if ($exitCode -ne 0 -or -not $durationOk -or -not $signatureOk -or -not $sizeOk) {
        $result | Select-Object -Last 8 | ForEach-Object { Write-Host "  $_" -ForegroundColor Red }
        Remove-Item -LiteralPath $tempPath -Force -ErrorAction SilentlyContinue
        Write-Warning "输出验证失败，原 part 保持不变: $outputName"
        $failed++
        continue
    }

    Move-Item -LiteralPath $tempPath -Destination $outputPath
    Write-Host "  验证通过：$([timespan]::FromSeconds($outputInfo.Duration).ToString('hh\:mm\:ss'))，$([math]::Round($tempBytes/1GB, 2))GB" -ForegroundColor Green

    if ($DeleteParts) {
        foreach ($part in $parts) {
            Remove-Item -LiteralPath $part.Path -Force
            $deleted++
        }
    } else {
        $backupDir = Join-Path $mergedDir "parts_backup"
        if (-not (Test-Path -LiteralPath $backupDir)) {
            New-Item -Path $backupDir -ItemType Directory | Out-Null
        }
        foreach ($part in $parts) {
            Move-Item -LiteralPath $part.Path -Destination (Join-Path $backupDir $part.Name)
            $archived++
        }
    }
    $completed++
}

Write-Host "`n完成 $completed 组；归档 $archived 个 part；删除 $deleted 个 part；失败 $failed 组" -ForegroundColor Cyan
if ($failed -gt 0) { exit 1 }
