# Head-to-head: IDM vs RDM on the same URL, alternating runs.
# Records wall time (until the finished file is in place) and peak TCP connections of each process.
param(
    [string]$Url = "https://proof.ovh.net/files/10Mb.dat",
    [long]$Size = 10485760,
    [int]$Rounds = 2,
    [int]$Connections = 8,
    [string]$Dir = "$env:TEMP\rdm-bench",
    # Per run; a run that hits it counts as failed and is left out of the averages.
    [int]$TimeoutSec = 600
)

$idm = "C:\Program Files (x86)\Internet Download Manager\IDMan.exe"
$serverPort = ([Uri]$Url).Port
$rdm = Join-Path $PSScriptRoot "..\target\release\rdm.exe"
New-Item -ItemType Directory -Force $Dir | Out-Null

# $sw is started by the caller just before launching the tool, so start-up time counts too.
function Wait-File($path, $procName, $sw) {
    $peak = 0
    while ($sw.Elapsed.TotalSeconds -lt $TimeoutSec) {
        if ((Test-Path $path) -and (Get-Item $path).Length -eq $Size) { break }
        $ids = (Get-Process $procName -ErrorAction SilentlyContinue).Id
        if ($ids) {
            $n = @(Get-NetTCPConnection -State Established -ErrorAction SilentlyContinue |
                Where-Object { $ids -contains $_.OwningProcess -and $_.RemotePort -eq $serverPort }).Count
            if ($n -gt $peak) { $peak = $n }
        }
        Start-Sleep -Milliseconds 100
    }
    $ok = (Test-Path $path) -and (Get-Item $path).Length -eq $Size
    [pscustomobject]@{ Seconds = [math]::Round($sw.Elapsed.TotalSeconds, 2); PeakConns = $peak; Ok = $ok }
}

function Report($tool, $round, $m) {
    $row = [pscustomobject]@{ Tool = $tool; Round = $round; Seconds = $m.Seconds; PeakConns = $m.PeakConns; Ok = $m.Ok }
    # Printed as each run ends, so a later stuck run doesn't lose earlier results.
    Write-Host ("{0} round {1}: {2}s, peak {3} conns, ok={4}" -f $tool, $round, $m.Seconds, $m.PeakConns, $m.Ok)
    $row
}

function Get-IdmEntryCount {
    @(Get-ChildItem "HKCU:\Software\DownloadManager" -ErrorAction SilentlyContinue | Where-Object { $_.PSChildName -match '^\d+$' }).Count
}

$results = foreach ($r in 1..$Rounds) {
    $name = "idm-$r-$(Get-Random).bin"
    $entries = Get-IdmEntryCount
    $sw = [Diagnostics.Stopwatch]::StartNew()
    & $idm /d $Url /p $Dir /f $name /n
    # A wedged IDM silently ignores /d; fail fast instead of timing out.
    $deadline = (Get-Date).AddSeconds(15)
    while ((Get-IdmEntryCount) -le $entries -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 250 }
    if ((Get-IdmEntryCount) -le $entries) { throw "IDM ignored the /d command (no new entry in its list). Restart IDM and retry." }
    Report "IDM" $r (Wait-File (Join-Path $Dir $name) "IDMan" $sw)

    $name = "rdm-$r-$(Get-Random).bin"
    $sw = [Diagnostics.Stopwatch]::StartNew()
    $p = Start-Process -FilePath $rdm -ArgumentList "`"$Url`"", "-d", "`"$Dir`"", "-c", $Connections, "-o", $name -WindowStyle Hidden -PassThru
    Report "RDM" $r (Wait-File (Join-Path $Dir $name) "rdm" $sw)
    if (!$p.HasExited) { Stop-Process -Id $p.Id -Force }
}
$results | Format-Table Tool, Round, Seconds, PeakConns, Ok -AutoSize
$results | Group-Object Tool | ForEach-Object {
    $done = @($_.Group | Where-Object Ok)
    if (!$done) { "{0}: no successful runs" -f $_.Name; return }
    $avg = ($done | Measure-Object Seconds -Average).Average
    "{0}: avg {1:N2}s = {2:N2} MiB/s over {3}/{4} successful runs" -f $_.Name, $avg, ($Size / 1MB / $avg), $done.Count, $_.Group.Count
}
