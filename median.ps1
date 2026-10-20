# Median of several measure.ps1 runs. Run measure.ps1 three times with names <prefix>1..3, then:
#   ./median.ps1 -Prefix base-open-
# Reads results\<prefix>*.json (written by measure.ps1) and prints the median of each column,
# plus the min-max range, ready to paste into the plan tables.
param([Parameter(Mandatory)][string]$Prefix)
$ErrorActionPreference = 'Stop'
$runs = @(Get-ChildItem (Join-Path $PSScriptRoot 'results') -Filter "$Prefix*.json" | ForEach-Object { Get-Content $_.FullName -Raw | ConvertFrom-Json })
if ($runs.Count -eq 0) { throw "no results\$Prefix*.json" }

function Median($values) {
    $v = @($values | Where-Object { $null -ne $_ } | Sort-Object)
    if ($v.Count -eq 0) { return $null }
    if ($v.Count % 2) { $v[[int][math]::Floor($v.Count / 2)] } else { ($v[$v.Count / 2 - 1] + $v[$v.Count / 2]) / 2 }
}
function Row($label, $values, [string]$unit) {
    $v = @($values | Where-Object { $null -ne $_ } | Sort-Object)
    if ($v.Count -eq 0) { return }
    "{0,-10} median {1}{4}  (min {2}, max {3})" -f $label, [math]::Round((Median $v), 1), $v[0], $v[-1], $unit
}

"== $Prefix : $($runs.Count) runs"
Row 'processes' $runs.processes ''
Row 'CPU' $runs.cpuPct ' %'
Row 'private' $runs.privMB ' MB'
Row 'WS' $runs.wsMB ' MB'
Row 'to bar' $runs.playerBarMs ' ms'
