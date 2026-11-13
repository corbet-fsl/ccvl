# Runs in the offline Windows clone as SYSTEM (`vm-win run`): the executable that ships.
# Reads W:\job\expect.json (the sha256 and runtime identity it must have) and the source of the
# commit in W:\job\src; writes W:\job\result.json (one entry per check: id, name, ok, detail)
# and W:\job\public-check.txt. windows-test.sh (outside the clone) is its other half.
$ErrorActionPreference = 'Continue'
$expect = Get-Content -Raw -LiteralPath W:\job\expect.json | ConvertFrom-Json
$work = 'C:\ccvl-windows-test'
New-Item -ItemType Directory -Path $work -Force | Out-Null
$checks = @()
function Add-Check([string]$Id, [string]$Name, [bool]$Ok, [string]$Detail) {
    $script:checks += [pscustomobject]@{ id = $Id; name = $Name; ok = $Ok; detail = $Detail }
    '{0,-5}{1}: {2}' -f $(if ($Ok) { 'ok' } else { 'FAIL' }), $Name, $Detail
}
# The executable runs from the clone's own disk: memory-mapped reads from the share fail.
# A copy from the share can fail in-page and drop the file: retry, and compare the size.
$exe = Join-Path $work 'ccvl.exe'
$copied = $false
for ($try = 1; $try -le 5 -and -not $copied; $try++) {
    try {
        Copy-Item -LiteralPath 'W:\job\ccvl-windows-x86_64.exe' -Destination $exe -Force -ErrorAction Stop
        $copied = (Get-Item -LiteralPath $exe).Length -eq (Get-Item -LiteralPath 'W:\job\ccvl-windows-x86_64.exe').Length
    } catch { Start-Sleep -Seconds 2 }
}
$hash = if ($copied) { (Get-FileHash -Algorithm SHA256 -LiteralPath $exe).Hash.ToLowerInvariant() } else { 'not copied' }
Add-Check 'sha256' 'the executable that runs is the release asset (sha256)' ($hash -ceq $expect.sha256) $hash
$arch = [Runtime.InteropServices.RuntimeInformation]::OSArchitecture
Add-Check 'architecture' 'the clone is x86_64 Windows' ($arch -eq 'X64') "$arch"
$version = (& $exe --version 2>&1 | Out-String).Trim()
Add-Check 'version' 'the executable starts (--version)' ($LASTEXITCODE -eq 0) $version
$runtime = (& $exe runtime-id 2>&1 | Out-String).Trim()
Add-Check 'runtime-id' 'the identity compiled into it is the release identity (runtime-id)' (($LASTEXITCODE -eq 0) -and ($runtime -ceq $expect.runtime_id)) $runtime
# The source of the same commit, on the clone's own disk, is the working directory of the check.
robocopy W:\job\src "$work\src" /E /NFL /NDL /NJH /NJS /NP | Out-Null
Set-Location "$work\src"
$clock = [Diagnostics.Stopwatch]::StartNew()
& $exe public-check *>&1 | Out-File -FilePath W:\job\public-check.txt -Encoding utf8
$code = $LASTEXITCODE
Add-Check 'public-check' 'public-check on the source of the commit (every document rendered and verified)' ($code -eq 0) ('exit {0} in {1:N0} s' -f $code, $clock.Elapsed.TotalSeconds)
$checks | ConvertTo-Json -Depth 3 | Out-File -FilePath W:\job\result.json -Encoding ascii
$failed = @($checks | Where-Object { -not $_.ok }).Count
"checks failed in the clone: $failed"
exit $failed
