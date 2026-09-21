function du($path=".") {
    Get-ChildItem $path |
    ForEach-Object {
        $file = $_
        Get-ChildItem -File -Recurse $_.FullName | Measure-Object -Property length -Sum |
        Select-Object -Property @{Name="Name";Expression={$file}},
                                @{Name="Size(MB)";Expression={[math]::round(($_.Sum / 1MB),2)}} # round 2 decimal places
    }
}