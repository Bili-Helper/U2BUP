# 设置 ffmpeg 可执行文件路径
$ffmpegPath = "C:\Users\Administrator\scoop\apps\ffmpeg\8.0\bin\ffmpeg.exe"

# 1. 获取当前文件夹的名称
$currentPath = Get-Location
$folderName = Split-Path $currentPath.Path -Leaf

# 2. 获取当前文件夹中 .ts 视频文件列表，并按名称排序（确保时间顺序）
$tsFiles = Get-ChildItem -Path $currentPath -Filter "*.mkv" | Sort-Object Name 

# 3. 从文件名中提取日期进行分组
# 假设文件名格式为 "YYYY-MM-DD HH-MM-SS 标题名.mkv"
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

    # 5. 【标题提取逻辑】: 从这一天最早的文件中提取标题
    $firstFileName = $videoFileObjects[0].BaseName # 只获取不包含扩展名的文件名 (例如：2025-11-01 08-02-51未都来了)
    
    # 日期时间部分的固定长度是 19 (YYYY-MM-DD HH-MM-SS)
    # 如果文件名是 'YYYY-MM-DD HH-MM-SS标题...'，标题从第 20 个字符开始（索引 19）
    if ($firstFileName.Length -ge 20 -and $firstFileName.Substring(4,1) -eq '-' -and $firstFileName.Substring(13,1) -eq ' ') {
        # 假设日期时间格式是 YYYY-MM-DD HH-MM-SS，总共 19 个字符
        $titlePart = $firstFileName.Substring(19)
    } elseif ($firstFileName -match '(?<=\d{4}-\d{2}-\d{2}\s\d{2}-\d{2}-\d{2}\s)(.+)$') {
        # 尝试使用正则表达式匹配：在 'YYYY-MM-DD HH-MM-SS ' 之后的所有内容
        $titlePart = $Matches[1]
    } else {
        # 兜底方案：如果文件名格式不规范，使用未命名
        $titlePart = "未命名"
    }

    # 6. 构造最终的输出文件名：[文件夹名称]-[日期] [标题].mp4
    # 注意：我们使用空格来分隔日期和标题，使文件名更美观
    $outputFileName = "$($folderName)-$($datePart) $($titlePart).mp4"
    
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
    #Remove-Item list.txt
}

Write-Host "--- All merging tasks completed. ---"