# 设置 ffmpeg 可执行文件路径
$ffmpegPath = "C:\Users\Administrator\scoop\apps\ffmpeg\8.0\bin\ffmpeg.exe"

# 1. 修正：获取当前文件夹的名称
$currentPath = Get-Location
$folderName = Split-Path $currentPath.Path -Leaf

# 2. 获取当前文件夹中 .ts 视频文件列表，并按名称排序（确保时间顺序）
$tsFiles = Get-ChildItem -Path $currentPath -Filter "*.ts" | Sort-Object Name 

# 3. 从文件名中提取日期进行分组
# 假设文件名格式为 "YYYY-MM-DD HH-MM-SS-标题.ts"
$videoFilesByDate = $tsFiles | Group-Object {
    # 提取文件名开头的 YYYY-MM-DD
    if ($_.Name.Length -ge 10 -and $_.Name -match "^\d{4}-\d{2}-\d{2}") {
        return $_.Name.Substring(0, 10)
    }
    return "UnknownDate"
}

# 4. 构造每天的视频文件列表并合并
foreach ($group in $videoFilesByDate) {
    $datePart = $group.Name # 提取出的日期字符串，例如 "2025-11-01"
    $videoFileObjects = $group.Group # 这一天的所有文件对象
    $videoFileNames = $videoFileObjects.Name # 这一天的所有文件名

    # 检查是否有未知日期分组
    if ($datePart -eq "UnknownDate") {
        Write-Warning "Skipping files with unknown date format: $($videoFileNames -join ', ')"
        continue
    }

    # 5. 保留标题逻辑：从这一天最早的文件中提取标题
    # $videoFileObjects 已经根据名称排序，所以 $videoFileObjects[0] 是最早的文件
    $firstFileName = $videoFileObjects[0].Name
    
    # 假设文件名格式为 "YYYY-MM-DD HH-MM-SS-标题.ts"
    # 我们要找到第一个 '-' 之后的所有内容 (包括后续的 '-')，然后去除文件扩展名 (.ts)
    if ($firstFileName -match '(?<=\d{4}-\d{2}-\d{2}\s\d{2}-\d{2}-\d{2}-)(.+?)\.ts$') {
        # 匹配到日期时间之后的标题部分
        $titlePart = $Matches[1]
    } else {
        # 如果没有明确的标题分隔符或格式不匹配
        $titlePart = "未命名"
    }

    # 6. 构造最终的输出文件名：[文件夹名称]-[日期]-[标题].mp4
    $outputFileName = "$($folderName)-$($datePart)-$($titlePart).mp4"
    
    Write-Host "--- Processing Date: $datePart ---"
    Write-Host "Output File: $outputFileName"

    # 7. 构造合并列表文件 (list.txt)
    $videoFileNames | ForEach-Object { 
        "file '$_'" 
    } | Out-File -Encoding UTF8NoBOM -FilePath list.txt

    # 8. 构造并执行合并命令
    $command = "$ffmpegPath -f concat -safe 0 -i list.txt -c copy '$outputFileName'"

    Write-Host "Executing: $command"
    Invoke-Expression $command

    # 清理 list.txt 文件
    Remove-Item list.txt
}

Write-Host "--- All merging tasks completed. ---"