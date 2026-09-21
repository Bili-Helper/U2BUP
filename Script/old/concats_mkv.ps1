# 设置 ffmpeg 可执行文件路径
$ffmpegPath = "C:\Users\Administrator\scoop\apps\ffmpeg\8.0\bin\ffmpeg.exe"

# 获取当前文件夹路径
$currentFolder = Get-Location

# 获取当前文件夹中相同日期的视频文件列表，并按时间升序排列
$videoFilesByDate = Get-ChildItem -Path $currentFolder -Filter "*.mkv" | Group-Object { $_.LastWriteTime.ToShortDateString() }

# 构造每天的视频文件列表并合并
foreach ($videoFiles in $videoFilesByDate) {
    $videoFileName = $videoFiles.Group | Select-Object -ExpandProperty Name
    # 获取输出文件名
    if ($videoFileName.Count -eq 1) {
        $outputFileName = $videoFileName.Split('-',2)[1]
    } else {
        $outputFileName = $videoFileName[0].Split('-',2)[1]
    }
    $outputFileName += ".mp4"

    # 构造合并命令
    $concatList = $videoFileName | ForEach-Object { "file '$($_)'" } | Out-File -Encoding UTF8NoBOM -FilePath list.txt

    $command = "$ffmpegPath -f concat -safe 0 -i list.txt -c copy '$outputFileName'"

    # 执行合并命令
    Invoke-Expression $command
}

