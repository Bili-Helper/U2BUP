#!/usr/bin/env pwsh
<#
.SYNOPSIS
    智能合并 Bilibili 直播录制视频片段
.DESCRIPTION
    根据相同直播会话(Session)和相同分辨率比例分组合并 FLV/TS/MKV 文件。
    处理跨午夜、PK/切流导致分辨率比例变化、文件名时区偏移等问题。
.PARAMETER SourceDir
    包含 FLV 文件的目录（每个主播一个子目录）
.PARAMETER SpecificDir
    只处理指定的子目录名（可选）
.PARAMETER OutputDir
    输出目录，默认在 SourceDir 下创建 merged 子目录
.PARAMETER DryRun
    仅预览合并计划，不实际执行
.PARAMETER SkipProbe
    跳过 ffprobe 检测（用于调试/重跑）
.PARAMETER MinSizeKB
    跳过小于此大小(KB)的文件，默认 512KB
.PARAMETER MinDurationSeconds
    跳过时长小于此秒数的异常文件，默认 1 秒
.PARAMETER MaxGapSeconds
    同一 session 内文件间最大间隔秒数，默认 1800 (30分钟)
#>

param(
    [Parameter(Mandatory=$true)]
    [string]$SourceDir,
    [string]$SpecificDir,
    [string]$OutputDir,
    [switch]$DryRun,
    [switch]$SkipProbe,
    [int]$MinSizeKB = 512,
    [int]$MinDurationSeconds = 1,
    [int]$MaxGapSeconds = 1800,
    [int]$MaxDurationSeconds = 21600,
    [int]$MinFreeSpaceGB = 10
)

# ─── ffmpeg 路径 ─────────────────────────────────────────────
$ffmpegBin = "C:\Users\Administrator\scoop\apps\ffmpeg\8.0\bin\ffmpeg.exe"
$ffprobeBin = "C:\Users\Administrator\scoop\apps\ffmpeg\8.0\bin\ffprobe.exe"
if (-not (Test-Path $ffmpegBin)) { throw "ffmpeg not found at $ffmpegBin" }
if (-not (Test-Path $ffprobeBin)) { throw "ffprobe not found at $ffprobeBin" }

# ─── 辅助函数 ────────────────────────────────────────────────
function Get-DisplayAspectBucket {
    param([double]$ratio)
    return [math]::Round($ratio, 1)
}

function Format-Duration {
    param([int]$seconds)
    return '{0:hh\:mm\:ss}' -f [timespan]::FromSeconds($seconds)
}

# ─── 第一步：扫描目录 ──────────────────────────────────
Write-Host "`n============================================" -ForegroundColor Cyan
Write-Host "  Bilibili 录播合并工具 v2" -ForegroundColor Cyan
Write-Host "============================================" -ForegroundColor Cyan

$targetDirs = @()
if ($SpecificDir) {
    $fullPath = Join-Path -Path $SourceDir -ChildPath $SpecificDir
    if (Test-Path $fullPath) {
        $targetDirs += Get-Item $fullPath
    } else {
        Write-Error "指定目录不存在: $fullPath"
        exit 1
    }
} else {
    $targetDirs = Get-ChildItem -Path $SourceDir -Directory | Sort-Object Name
}

Write-Host "`n找到 $($targetDirs.Count) 个主播目录" -ForegroundColor Yellow

foreach ($dir in $targetDirs) {
    $dirName = $dir.Name
    # 从目录名提取主播名（格式：{roomId}-{主播名}）
    $streamerName = $dirName
    if ($dirName -match '^\d+-(.+)$') {
        $streamerName = $Matches[1]
    }
    Write-Host "`n═══════════════════════════════════════════" -ForegroundColor Green
    Write-Host "  处理目录: $dirName" -ForegroundColor Green
    Write-Host "  主播: $streamerName" -ForegroundColor Green
    Write-Host "─────────────────────────────────────────────"

    $flvFiles = Get-ChildItem -Path $dir.FullName -File |
        Where-Object { $_.Extension.ToLowerInvariant() -in '.flv', '.ts', '.mkv' } |
        Sort-Object Name
    if ($flvFiles.Count -eq 0) {
        Write-Host "  ⚠  没有 FLV/TS/MKV 视频文件，跳过" -ForegroundColor Yellow
        continue
    }
    Write-Host "  找到 $($flvFiles.Count) 个视频文件" -ForegroundColor Gray

    # 跳过过小的文件（录制碎片/损坏片段）
    $minBytes = $MinSizeKB * 1024
    $smallFiles = ($flvFiles | Where-Object { $_.Length -lt $minBytes }).Count
    if ($smallFiles -gt 0) {
        Write-Host "  🗑️  跳过 $smallFiles 个小于 ${MinSizeKB}KB 的碎片文件" -ForegroundColor DarkYellow
        $flvFiles = $flvFiles | Where-Object { $_.Length -ge $minBytes }
    }

    # ─── 第二步：解析文件名 + ffprobe 探测 ─────────────────
    $fileInfos = @()
    foreach ($flv in $flvFiles) {
        $base = $flv.BaseName

        # 解析文件名：支持两种格式
        # 格式1：录制-{roomId}-{YYYYMMDD}-{HHMMSS}-{seq}-{title}.flv
        # 格式2：{YYYY}-{MM}-{DD} {HH}-{MM}-{SS}-{xxx} {title}.flv
        $roomId    = 0
        $dateStr   = ''
        $timeStr   = ''
        $title     = ''
        $rawTitle  = ''
        
        if ($base -match '^录制-(\d+)-(\d{8})-(\d{6})-(\d+)-(.*)$') {
            # 格式1：Bilive 标准格式
            $roomId    = [int]$Matches[1]
            $dateStr   = $Matches[2]
            $timeStr   = $Matches[3]
            $title     = $Matches[5]
        } elseif ($base -match '^(\d{4})-(\d{2})-(\d{2}) (\d{2})-(\d{2})-(\d{2})-\d+\s+(.*)$') {
            # 格式2：替代格式 YYYY-MM-DD HH-MM-SS-xxx title
            $roomId    = 0  # 无 roomId 信息
            $dateStr   = "$($Matches[1])$($Matches[2])$($Matches[3])"
            $timeStr   = "$($Matches[4])$($Matches[5])$($Matches[6])"
            $title     = $Matches[7]
        } else {
            Write-Warning "  跳过无法解析的文件名: $($flv.Name)"
            continue
        }

        # Bilive 在异常重连时可能给标题追加 _PART000；它不是直播标题的一部分。
        $rawTitle = $title
        $title = ($title -replace '(?i)_PART\d+$', '').Trim()

        # 文件名中的日期 = 真实直播时间（北京时间 UTC+8）
        $liveDateTime = [datetime]::ParseExact("$dateStr $timeStr", "yyyyMMdd HHmmss", $null)

        # ffprobe 获取完整流参数 + 时长；相同比例但不同分辨率/编码不能直接混拼。
        $width = $null; $height = $null; $duration = $null
        $videoCodec = ''; $videoProfile = ''; $pixelFormat = ''
        $audioCodec = ''; $sampleRate = 0; $channels = 0

        if (-not $SkipProbe) {
            $probeOk = $false
            for ($attempt = 0; $attempt -lt 2; $attempt++) {
                try {
                    $jsonStr = & $ffprobeBin -v error `
                        -show_entries stream=codec_type,codec_name,profile,width,height,pix_fmt,sample_rate,channels `
                        -show_entries format=duration -of json -- $flv.FullName 2>&1 | Out-String
                    $probeExitCode = $LASTEXITCODE
                    $json = $jsonStr | ConvertFrom-Json
                    $videoStream = $json.streams | Where-Object codec_type -eq 'video' | Select-Object -First 1
                    $audioStream = $json.streams | Where-Object codec_type -eq 'audio' | Select-Object -First 1
                    $width   = [int]$videoStream.width
                    $height  = [int]$videoStream.height
                    $videoCodec = [string]$videoStream.codec_name
                    $videoProfile = [string]$videoStream.profile
                    $pixelFormat = [string]$videoStream.pix_fmt
                    if ($null -ne $audioStream) {
                        $audioCodec = [string]$audioStream.codec_name
                        $sampleRate = [int]$audioStream.sample_rate
                        $channels = [int]$audioStream.channels
                    } else {
                        $audioCodec = 'none'
                    }
                    $duration = [math]::Round([double]$json.format.duration, 0)
                    if ($probeExitCode -eq 0 -and $width -gt 0 -and $height -gt 0 -and $duration -ge $MinDurationSeconds) {
                        $probeOk = $true
                        break
                    }
                } catch {
                    Start-Sleep -Milliseconds 300
                }
            }
            if (-not $probeOk) {
                Write-Warning "  ⚠  无法探测元数据，跳过: $($flv.Name)"
                continue
            }
        } else {
            $width = 0; $height = 0; $duration = 0
            $videoCodec = 'unknown'; $videoProfile = 'unknown'; $pixelFormat = 'unknown'
            $audioCodec = 'unknown'; $sampleRate = 0; $channels = 0
        }

        $ratio = [double]$width / [double]$height
        $ratioBucket = Get-DisplayAspectBucket $ratio
        $configKey = "$ratioBucket|$width|$height|$videoCodec|$videoProfile|$pixelFormat|$audioCodec|$sampleRate|$channels"
        $endDateTime = $liveDateTime.AddSeconds($duration)

        $fileInfos += [PSCustomObject]@{
            File        = $flv
            FileName    = $flv.Name
            FullPath    = $flv.FullName
            RoomId      = $roomId
            DateStr     = $dateStr
            TimeStr     = $timeStr
            LiveStart   = $liveDateTime
            LiveEnd     = $endDateTime
            Title       = $title
            RawTitle    = $rawTitle
            Width       = $width
            Height      = $height
            Ratio       = $ratio
            RatioBucket = $ratioBucket
            ConfigKey   = $configKey
            VideoCodec  = $videoCodec
            VideoProfile = $videoProfile
            PixelFormat = $pixelFormat
            AudioCodec  = $audioCodec
            SampleRate  = $sampleRate
            Channels    = $channels
            Duration    = $duration
            SizeMB      = [math]::Round($flv.Length / 1MB, 1)
        }
    }

    if ($fileInfos.Count -eq 0) {
        Write-Host "  ⚠  没有可处理的视频文件" -ForegroundColor Yellow
        continue
    }
    Write-Host "  成功解析 $($fileInfos.Count) 个文件" -ForegroundColor Gray

    # ─── 第三步：按直播时间排序 ────────────────────────────
    $fileInfos = $fileInfos | Sort-Object LiveStart

    Write-Host "`n  📋 文件概览:" -ForegroundColor Cyan
    $fileInfos | ForEach-Object {
        $ratioLabel = if ($_.RatioBucket -eq 0.6) { "9:16竖屏" } elseif ($_.RatioBucket -eq 1.8) { "16:9横屏" } else { "$($_.RatioBucket)" }
        Write-Host "    $($_.DateStr) $($_.TimeStr) | $($_.Width)x$($_.Height) ($ratioLabel) | $(Format-Duration $_.Duration) | $($_.SizeMB)MB | $($_.Title)" -ForegroundColor Gray
    }

    # ─── 第四步：分 Session ────────────────────────────
    $sessions = @()
    $currentSession = @($fileInfos[0])
    for ($i = 1; $i -lt $fileInfos.Count; $i++) {
        $prev = $fileInfos[$i - 1]
        $curr = $fileInfos[$i]
        $gap = ($curr.LiveStart - $prev.LiveEnd).TotalSeconds

        $sameTitle = ($curr.Title -eq $prev.Title)
        $withinGap = ($gap -le $MaxGapSeconds -and $gap -ge -300)

        if ($sameTitle -and $withinGap) {
            $currentSession += $curr
        } else {
            $reason = if (-not $sameTitle) { "标题变化" } else { "时间不连续(gap=${gap}s)" }
            Write-Host "  ✂️  新 session（$reason）: '$($prev.Title)' → '$($curr.Title)'" -ForegroundColor DarkYellow
            $sessions += ,@($currentSession)
            $currentSession = @($curr)
        }
    }
    $sessions += ,@($currentSession)
    Write-Host "  🎬 共识别 $($sessions.Count) 个直播场次(Session)" -ForegroundColor Cyan

    # ─── 第五步：Session 内按 Ratio 分组 ───────────────────
    $mergeTasks = @()
    $plannedOutputNames = @{}
    foreach ($session in $sessions) {
        $sessionStart = $session[0].LiveStart
        $sessionEnd   = $session[-1].LiveEnd
        $sessionTitle = $session[0].Title
        $sessionRoom  = $session[0].RoomId

        $startDate = $sessionStart.ToString("yyyyMMdd")
        $endDate   = $sessionEnd.ToString("yyyyMMdd")
        $dateRange = if ($startDate -eq $endDate) { $startDate } else { "${startDate}-${endDate}" }
        $startTime = $sessionStart.ToString("HHmmss")

        Write-Host "`n  📺 Session: $dateRange | $sessionTitle | $($session.Count) 个文件" -ForegroundColor Magenta

        # 按完整流配置分组，防止同画幅比例下的不同分辨率/编码被混入一个 MP4。
        $ratioGroups = @($session | Group-Object ConfigKey)
        $ratioConfigCounts = @{}
        foreach ($configGroup in $ratioGroups) {
            $ratioKey = [string]$configGroup.Group[0].RatioBucket
            if ($ratioConfigCounts.ContainsKey($ratioKey)) { $ratioConfigCounts[$ratioKey]++ }
            else { $ratioConfigCounts[$ratioKey] = 1 }
        }

        foreach ($group in $ratioGroups) {
            $bucket = [double]$group.Group[0].RatioBucket
            $groupFiles = $group.Group | Sort-Object LiveStart

            # 同 ratio 内再检查时间连续性
            $ratioSubGroups = @()
            $currentSubGroup = @($groupFiles[0])
            for ($gi = 1; $gi -lt $groupFiles.Count; $gi++) {
                $prevF = $groupFiles[$gi - 1]
                $currF = $groupFiles[$gi]
                $gap = ($currF.LiveStart - $prevF.LiveEnd).TotalSeconds
                if ($gap -le $MaxGapSeconds -and $gap -ge -300) {
                    $currentSubGroup += $currF
                } else {
                    $ratioSubGroups += ,@($currentSubGroup)
                    $currentSubGroup = @($currF)
                }
            }
            $ratioSubGroups += ,@($currentSubGroup)

            $subIdx = 0
            foreach ($subGroup in $ratioSubGroups) {
                # 按最大时长拆分（贪婪算法，尽量填满 <= ${MaxDurationSeconds}s）
                $durGroups = @()
                $currentDurGroup = @($subGroup[0])
                $currentDur = $subGroup[0].Duration
                for ($si = 1; $si -lt $subGroup.Count; $si++) {
                    $nextFile = $subGroup[$si]
                    if ($currentDur + $nextFile.Duration -le $MaxDurationSeconds) {
                        $currentDurGroup += $nextFile
                        $currentDur += $nextFile.Duration
                    } else {
                        $durGroups += ,@($currentDurGroup)
                        $currentDurGroup = @($nextFile)
                        $currentDur = $nextFile.Duration
                    }
                }
                $durGroups += ,@($currentDurGroup)

                $durPart = 0
                foreach ($durGroup in $durGroups) {
                    $subIdx++
                    $durPart++
                    $resolutions = $durGroup | Select-Object -Property Width, Height -Unique
                    $resStr = ($resolutions | ForEach-Object { "$($_.Width)x$($_.Height)" }) -join ', '

                    $ratioLabel = if ([double]$bucket -eq 0.6) { "9x16" } elseif ([double]$bucket -eq 1.8) { "16x9" } else { "r${bucket}" }
                    if ($ratioConfigCounts[[string]$bucket] -gt 1) {
                        $configFile = $durGroup[0]
                        $profileTag = ($configFile.VideoProfile -replace '[^A-Za-z0-9]+', '')
                        $ratioLabel += "_$($configFile.Width)x$($configFile.Height)_$($configFile.VideoCodec)_${profileTag}"
                    }

                    # 开播时间是 Session 的稳定唯一标识，避免同日同标题的多场直播发生文件名碰撞。
                    $outputName = "${streamerName}_${dateRange}_${startTime}_${sessionTitle}_[${ratioLabel}]"
                    if ($ratioSubGroups.Count -gt 1 -or $durGroups.Count -gt 1) {
                        $outputName += "_part${subIdx}"
                    }
                    $outputName += ".mp4"
                    # 清理文件名中的非法字符
                    $outputName = $outputName -replace '[<>:"/\\|?*#&! ]', '_'

                    # 极端情况下仍可能重名；在执行前消歧，绝不能靠“已存在，跳过”吞掉任务。
                    if ($plannedOutputNames.ContainsKey($outputName)) {
                        $plannedOutputNames[$outputName]++
                        $stem = [System.IO.Path]::GetFileNameWithoutExtension($outputName)
                        $outputName = "${stem}_dup$($plannedOutputNames[$outputName]).mp4"
                    } else {
                        $plannedOutputNames[$outputName] = 1
                    }

                    $durStr = Format-Duration (($durGroup | Measure-Object Duration -Sum).Sum)
                    $resLabel = if ([double]$bucket -eq 0.6) { "竖屏9:16" } elseif ([double]$bucket -eq 1.8) { "横屏16:9" } else { "比例${bucket}" }
                    $partLabel = if ($durGroups.Count -gt 1) { " ($durPart/$($durGroups.Count))" } else { "" }
                    Write-Host "    └─ 📦 ${resLabel} ($resStr) 时长: $durStr${partLabel} → $outputName" -ForegroundColor Green
                    foreach ($f in $durGroup) {
                        Write-Host "       · $($f.DateStr) $($f.TimeStr) | $($f.Width)x$($f.Height) | $(Format-Duration $f.Duration) | $($f.SizeMB)MB" -ForegroundColor Gray
                    }

                    $mergeTasks += [PSCustomObject]@{
                        OutputName      = $outputName
                        Files           = $durGroup
                        RatioBucket     = [double]$bucket
                        DateRange       = $dateRange
                        SessionTitle    = $sessionTitle
                        TotalSizeMB     = [math]::Round(($durGroup | Measure-Object SizeMB -Sum).Sum, 0)
                        TotalDur        = ($durGroup | Measure-Object Duration -Sum).Sum
                    }
                }
            }
        }
    }

    # ─── 第六步：执行合并 ────────────────────────────────────
    if ($mergeTasks.Count -eq 0) { continue }

    $outDir = if ($OutputDir) { $OutputDir } else { Join-Path $dir.FullName "merged" }

    Write-Host "`n  ─── 合并任务概览 ─────────────────────" -ForegroundColor Cyan
    $totalSize = 0
    foreach ($task in $mergeTasks) {
        $durStr = Format-Duration $task.TotalDur
        Write-Host "  📄 $($task.OutputName)" -ForegroundColor White
        Write-Host "     文件数: $($task.Files.Count) | 总时长: $durStr | 总大小: $($task.TotalSizeMB)MB" -ForegroundColor Gray
        $totalSize += $task.TotalSizeMB
    }
    Write-Host "  ────────────────────────────────────" -ForegroundColor Cyan
    Write-Host "  合计: $($mergeTasks.Count) 个输出文件, 预估空间 $($totalSize)MB" -ForegroundColor Yellow

    if ($DryRun) {
        Write-Host "  🚩 DryRun 模式，不执行合并" -ForegroundColor Yellow
        continue
    }

    if (-not (Test-Path $outDir)) { New-Item -Path $outDir -ItemType Directory -Force | Out-Null }

    # 只为尚未完成的任务预留空间；这样中断后可以续跑，全部完成后也能安全重跑。
    $pendingTasks = @($mergeTasks | Where-Object {
        -not (Test-Path -LiteralPath (Join-Path $outDir $_.OutputName))
    })
    if ($pendingTasks.Count -eq 0) {
        Write-Host "  ⏭  所有输出均已存在，无需重复合并" -ForegroundColor Yellow
        continue
    }

    # 按实际输出盘检查空间，并保留系统运行余量。
    $requiredBytes = ($pendingTasks | ForEach-Object { $_.Files | ForEach-Object { $_.File.Length } } | Measure-Object -Sum).Sum
    $outputRoot = [System.IO.Path]::GetPathRoot([System.IO.Path]::GetFullPath($outDir))
    $driveInfo = [System.IO.DriveInfo]::new($outputRoot)
    $reserveBytes = [int64]$MinFreeSpaceGB * 1GB
    if ($requiredBytes + $reserveBytes -gt $driveInfo.AvailableFreeSpace) {
        Write-Error "  输出盘空间不足！预计需要 $([math]::Round($requiredBytes/1GB, 2))GB，另保留 ${MinFreeSpaceGB}GB；可用 $([math]::Round($driveInfo.AvailableFreeSpace/1GB, 2))GB"
        continue
    }

    # 执行合并
    foreach ($task in $pendingTasks) {
        $outputPath = Join-Path $outDir $task.OutputName

        if (Test-Path -LiteralPath $outputPath) {
            Write-Host "  ⏭  已存在，跳过: $($task.OutputName)" -ForegroundColor Yellow
            continue
        }

        Write-Host "`n  🚀 合并: $($task.OutputName)" -ForegroundColor Green

        $listPath = Join-Path $outDir "_concat_$([System.Guid]::NewGuid().ToString('N')).txt"
        $task.Files | ForEach-Object {
            "file '$($_.FullPath)'"
        } | Out-File -FilePath $listPath -Encoding UTF8NoBOM

        $tempOutputPath = Join-Path $outDir "_building_$([System.Guid]::NewGuid().ToString('N')).mp4"
        $ffCmd = @(
            '-fflags', '+genpts',
            '-f', 'concat',
            '-safe', '0',
            '-i', $listPath,
            '-c', 'copy',
            '-avoid_negative_ts', '1',
            '-y',
            $tempOutputPath
        )

        Write-Host "  ⚡ ffmpeg concat ($($task.Files.Count) 个文件)..."
        Write-Host "  -> $outputPath" -ForegroundColor DarkGray

        $sw = [System.Diagnostics.Stopwatch]::StartNew()

        if ($DryRun) { $success = $true }
        else {
            # 使用 & 直接调用，正确处理 Unicode 路径
            $result = & $ffmpegBin $ffCmd 2>&1
            $exitCode = $LASTEXITCODE
            $success = ($exitCode -eq 0 -and (Test-Path $tempOutputPath))
            if ($success) {
                $probeDurationText = & $ffprobeBin -v error -show_entries format=duration -of default=noprint_wrappers=1:nokey=1 $tempOutputPath 2>&1 | Out-String
                $probeExitCode = $LASTEXITCODE
                $probeDuration = 0.0
                $durationParsed = [double]::TryParse($probeDurationText.Trim(), [ref]$probeDuration)
                $expectedDuration = [double]$task.TotalDur
                $durationTolerance = [math]::Max(5.0, $expectedDuration * 0.01)
                $durationOk = ($durationParsed -and [math]::Abs($probeDuration - $expectedDuration) -le $durationTolerance)
                $expectedBytes = ($task.Files | ForEach-Object { $_.File.Length } | Measure-Object -Sum).Sum
                $tempBytes = (Get-Item -LiteralPath $tempOutputPath).Length
                $sizeOk = ($tempBytes -ge $expectedBytes * 0.85)
                $success = ($probeExitCode -eq 0 -and $durationOk -and $sizeOk)
            }
            if (-not $success) {
                $result | Select-Object -Last 5 | ForEach-Object { Write-Host "  $_" -ForegroundColor Red }
            }
        }

        $sw.Stop()
        if (Test-Path $listPath) { Remove-Item $listPath -Force -ErrorAction SilentlyContinue }

        if ($success) {
            Move-Item -LiteralPath $tempOutputPath -Destination $outputPath
            if (Test-Path -LiteralPath $outputPath) {
                $outSize = [math]::Round((Get-Item -LiteralPath $outputPath).Length / 1MB, 1)
                Write-Host "  ✅ 完成! $(Format-Duration $sw.Elapsed.TotalSeconds) | 输出: ${outSize}MB" -ForegroundColor Green
            } else {
                Write-Host "  ⚠ 完成但文件未找到: $($task.OutputName)" -ForegroundColor Yellow
            }
        } else {
            if (Test-Path $tempOutputPath) { Remove-Item -LiteralPath $tempOutputPath -Force -ErrorAction SilentlyContinue }
            Write-Host "  ❌ 失败 (exit code: $exitCode)" -ForegroundColor Red
        }
    }

    Write-Host "`n  ✅ 目录处理完毕: $dirName" -ForegroundColor Green
}

Write-Host "`n============================================" -ForegroundColor Cyan
Write-Host "  全部处理完成！" -ForegroundColor Cyan
Write-Host "============================================" -ForegroundColor Cyan
