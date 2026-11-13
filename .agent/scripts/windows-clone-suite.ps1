# Runs in the offline Windows clone as SYSTEM (`vm-win run`): the workspace test suite.
# Reads W:\job\suite.json (`top`: the first path component of the checkout on the build host, `src`:
# its path as Windows reads it) and W:\job\tests.list (the test executables in W:\job). The checkout
# was mirrored to the same absolute path below the share, because the test executables embed it.
# Writes W:\job\full-<executable>.txt, one log per executable. windows-test.sh is its other half.
$ErrorActionPreference = 'Continue'
$config = Get-Content -Raw -LiteralPath W:\job\suite.json | ConvertFrom-Json
$top = $config.top
$src = $config.src
$names = @(Get-Content -LiteralPath W:\job\tests.list)
# Memory-mapped reads from the virtiofs share fail (STATUS_IN_PAGE_ERROR): everything runs
# from the clone's own disk. A copy can fail in-page and drop a file without a word, so
# each file is copied on its own, retried, and counted.
robocopy "W:\$top" "C:\$top" /E /NFL /NDL /NJH /NJS /NP | Out-Null
New-Item -ItemType Directory -Path C:\job -Force | Out-Null
$lost = 0
foreach ($name in $names) {
    $ok = $false
    for ($try = 1; $try -le 5 -and -not $ok; $try++) {
        try {
            Copy-Item -LiteralPath "W:\job\$name" -Destination "C:\job\$name" -Force -ErrorAction Stop
            $ok = (Get-Item -LiteralPath "C:\job\$name").Length -eq (Get-Item -LiteralPath "W:\job\$name").Length
        } catch { Start-Sleep -Seconds 2 }
    }
    if (-not $ok) { "COPY FAILED: $name"; $lost++ }
}
"test executables copied: $($names.Count - $lost) of $($names.Count)"
if ($lost -gt 0) { exit 1 }
Set-Location $src
$failed = 0
foreach ($name in $names) {
    "=== $name"
    $full = "W:\job\full-$([IO.Path]::GetFileNameWithoutExtension($name)).txt"
    & "C:\job\$name" --test-threads=2 *>&1 | Out-File -FilePath $full -Encoding utf8
    $code = $LASTEXITCODE
    Select-String -Path $full -Pattern '^test result:' | ForEach-Object { $_.Line }
    if ($code -ne 0) { "FAILED: $name exit $code"; $failed++ }
}
"test executables failed: $failed"
exit $failed
