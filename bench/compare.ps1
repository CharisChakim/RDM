# Head-to-head: IDM vs RDM on the same URL, alternating runs.
# Records wall time (until the finished file is in place) and peak TCP connections of each process.
param(
    [string]$Url = "https://proof.ovh.net/files/10Mb.dat",
    [long]$Size = 10485760,
    [int]$Rounds = 2,
    [int]$Connections = 8,
    [string]$Dir = "$env:TEMP\rdm-bench"
)

$idm = "C:\Program Files (x86)\Internet Download Manager\IDMan.exe"
$rdm = Join-Path $PSScriptRoot "..\target\release\rdm.exe"
New-Item -ItemType Directory -Force $Dir | Out-Null

function Wait-File($path, $procName, $timeoutSec = 600) {
    $sw = [Diagnostics.Stopwatch]::StartNew()
    $peak = 0
    while ($sw.Elapsed.TotalSeconds -lt $timeoutSec) {
        if ((Test-Path $path) -and (Get-Item $path).Length -eq $Size) { break }
        $ids = (Get-Process $procName -ErrorAction SilentlyContinue).Id
        if ($ids) {
            $n = @(Get-NetTCPConnection -State Established -ErrorAction SilentlyContinue |
                Where-Object { $ids -contains $_.OwningProcess -and $_.RemotePort -in 80, 443 }).Count
            if ($n -gt $peak) { $peak = $n }
        }
        Start-Sleep -Milliseconds 100
    }
    [pscustomobject]@{ Seconds = [math]::Round($sw.Elapsed.TotalSeconds, 2); PeakConns = $peak; Ok = (Test-Path $path) }
}

$results = foreach ($r in 1..$Rounds) {
    $name = "idm-$r-$(Get-Random).bin"
    $t0 = Get-Date
    & $idm /d $Url /p $Dir /f $name /n
    $m = Wait-File (Join-Path $Dir $name) "IDMan"
    [pscustomobject]@{ Tool = "IDM"; Round = $r } | Add-Member -PassThru -NotePropertyMembers @{ Seconds = $m.Seconds; PeakConns = $m.PeakConns; Ok = $m.Ok }

    $name = "rdm-$r-$(Get-Random).bin"
    Start-Process -FilePath $rdm -ArgumentList "`"$Url`"", "-d", "`"$Dir`"", "-c", $Connections, "-o", $name -WindowStyle Hidden
    $m = Wait-File (Join-Path $Dir $name) "rdm"
    [pscustomobject]@{ Tool = "RDM"; Round = $r } | Add-Member -PassThru -NotePropertyMembers @{ Seconds = $m.Seconds; PeakConns = $m.PeakConns; Ok = $m.Ok }
}
$results | Format-Table Tool, Round, Seconds, PeakConns, Ok -AutoSize
$results | Group-Object Tool | ForEach-Object {
    $avg = ($_.Group | Measure-Object Seconds -Average).Average
    "{0}: avg {1:N2}s = {2:N2} MiB/s" -f $_.Name, $avg, ($Size / 1MB / $avg)
}
