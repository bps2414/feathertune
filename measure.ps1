# Measures the process tree of an exe (the app, or the spike) and its msedgewebview2.exe children.
# Criterion: Win32_Process WorkingSetSize, PrivatePageCount and CPU time,
# CPU as % of one core. Results go to results\<Name>.json, the exe's stdout to results\<Name>.log.
#
#   ./measure.ps1 -Name playing -ArgumentList '--measure','https://music.youtube.com/watch?v=<id>'
#   ./measure.ps1 -Name tray -HideAfter 25 -Wait 50 -ArgumentList ...      # closes the window -> tray
#   ./measure.ps1 -Name mini -MiniAfter 25 -Wait 50 -ArgumentList ...    # opens the mini player (hides the window)
#   ./measure.ps1 -Name tray-1h -HideAfter 25 -Minutes 60 -ArgumentList ...  # one sample per minute
#   ./measure.ps1 -Name spike -Exe spike\target\release\spike.exe -ArgumentList '"<url>"','"spike\data\x"'
param(
    [Parameter(Mandatory)][string]$Name,
    [string]$Exe = (Join-Path $PSScriptRoot 'app\target\release\feathertune.exe'),
    [string[]]$ArgumentList = @(),
    [int]$Wait = 45,       # seconds before the first snapshot
    [int]$Window = 20,     # CPU window between the two snapshots
    [int]$HideAfter = 0,   # >0: sends WM_CLOSE to the main window after N s (the app hides in the tray)
    [int]$MiniAfter = 0,   # >0: runs `<exe> --mini-player` after N s (the running instance opens the mini player)
    [int]$Minutes = 0      # >0: sampling mode, one sample per minute for N minutes (after HideAfter)
)
$ErrorActionPreference = 'Stop'
$Exe = (Resolve-Path $Exe).Path
$results = Join-Path $PSScriptRoot 'results'
$log = Join-Path $results "$Name.log"
New-Item -ItemType Directory -Force $results | Out-Null

# Two instances with the same WebView2 profile share one browser process, which breaks the tree.
$running = Get-CimInstance Win32_Process | Where-Object { $_.ExecutablePath -eq $Exe }
if ($running) { throw "$(Split-Path $Exe -Leaf) is already running (pid $($running.ProcessId -join ', ')). Close it first." }

Add-Type -Namespace Measure -Name Win32 -MemberDefinition @'
[DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
[DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
[DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
[DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
[DllImport("user32.dll")] public static extern bool AttachThreadInput(uint from, uint to, bool attach);
[DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();
[DllImport("user32.dll")] public static extern void SwitchToThisWindow(IntPtr h, bool altTab);
public static void BringToFront(IntPtr h) {
    uint pid; uint fg = GetWindowThreadProcessId(GetForegroundWindow(), out pid); uint me = GetCurrentThreadId();
    AttachThreadInput(me, fg, true); SetForegroundWindow(h); AttachThreadInput(me, fg, false);
    if (GetForegroundWindow() != h) SwitchToThisWindow(h, true);
}
'@

$start = @{ FilePath = $Exe; PassThru = $true; RedirectStandardOutput = $log }
if ($ArgumentList) { $start.ArgumentList = $ArgumentList }
$p = Start-Process @start
$t0 = Get-Date

function Snapshot([int]$root) {
    $all = Get-CimInstance Win32_Process
    $ids = [System.Collections.Generic.HashSet[int]]::new(); [void]$ids.Add($root)
    do {
        $added = 0
        foreach ($x in $all) { if ($ids.Contains([int]$x.ParentProcessId) -and $ids.Add([int]$x.ProcessId)) { $added++ } }
    } while ($added)
    $all | Where-Object { $ids.Contains([int]$_.ProcessId) } | ForEach-Object {
        $t = if ($_.Name -ne 'msedgewebview2.exe') { $_.Name } elseif ($_.CommandLine -match '--type=([\w-]+)') { $Matches[1] } else { 'browser' }
        if ($_.CommandLine -match '--utility-sub-type=([\w.]+)') { $t += ':' + ($Matches[1] -replace '\.mojom\.\w+$', '') }
        [pscustomobject]@{ pid = $_.ProcessId; type = $t; cpu = ($_.KernelModeTime + $_.UserModeTime) / 1e7; ws = $_.WorkingSetSize / 1MB; priv = $_.PrivatePageCount / 1MB }
    }
}

function Compare-Snapshots($a, $b, [double]$seconds) {
    foreach ($x in $b) {
        $prev = $a | Where-Object pid -eq $x.pid
        $cpu = if ($prev) { ($x.cpu - $prev.cpu) / $seconds * 100 } else { 0 }
        [pscustomobject]@{ type = $x.type; cpuPct = [math]::Round($cpu, 1); wsMB = [math]::Round($x.ws, 1); privMB = [math]::Round($x.priv, 1) }
    }
}

function Total($rows) {
    [pscustomobject]@{
        processes = @($rows).Count
        cpuPct = [math]::Round(($rows | Measure-Object cpuPct -Sum).Sum, 1)
        wsMB = [math]::Round(($rows | Measure-Object wsMB -Sum).Sum)
        privMB = [math]::Round(($rows | Measure-Object privMB -Sum).Sum)
    }
}

function Wait-Until([double]$seconds) {
    $left = $seconds - ((Get-Date) - $t0).TotalSeconds
    if ($left -gt 0) { Start-Sleep -Milliseconds ([int]($left * 1000)) }
}

try {
    # Work that runs only for the window in front (the title bar animation) needs it in front: sharing
    # input with the current foreground thread lets SetForegroundWindow through (a synthetic Alt press
    # would put the app in its menu loop). `focused` ends up in the result.
    Wait-Until 18
    $h = (Get-Process -Id $p.Id).MainWindowHandle
    $focused = $false
    for ($try = 0; $try -lt 5 -and -not $focused -and $h -ne [IntPtr]::Zero; $try++) {
        [Measure.Win32]::BringToFront($h)
        Start-Sleep -Milliseconds 400
        $focused = [Measure.Win32]::GetForegroundWindow() -eq $h
    }
    if (-not $focused) {
        $fg = [Measure.Win32]::GetForegroundWindow(); $fgPid = 0
        [void][Measure.Win32]::GetWindowThreadProcessId($fg, [ref]$fgPid)
        Write-Host "not in front: main window $h, foreground $fg (process $((Get-Process -Id $fgPid -ErrorAction SilentlyContinue).ProcessName), the app's: $($fgPid -eq $p.Id))"
    }

    if ($HideAfter -gt 0) {
        Wait-Until $HideAfter
        $h = (Get-Process -Id $p.Id).MainWindowHandle
        if ($h -eq [IntPtr]::Zero) { throw 'main window not found' }
        [void][Measure.Win32]::PostMessage($h, 0x10, [IntPtr]::Zero, [IntPtr]::Zero)  # WM_CLOSE
    }
    if ($MiniAfter -gt 0) {
        Wait-Until $MiniAfter
        Start-Process $Exe -ArgumentList '--mini-player' -Wait
    }

    if ($Minutes -gt 0) {
        $prev = Snapshot $p.Id
        $samples = for ($m = 1; $m -le $Minutes; $m++) {
            Wait-Until ($HideAfter + 60 * $m)
            if ($p.HasExited) { throw "the app exited at minute $m" }
            $cur = Snapshot $p.Id
            $tot = Total (Compare-Snapshots $prev $cur 60)
            $prev = $cur
            $s = [pscustomobject]@{ minute = $m; processes = $tot.processes; cpuPct = $tot.cpuPct; wsMB = $tot.wsMB; privMB = $tot.privMB }
            "{0,3} min: CPU {1}%, WS {2} MB, private {3} MB" -f $m, $s.cpuPct, $s.wsMB, $s.privMB | Write-Host
            $s
        }
        $ws5 = ($samples | Where-Object minute -eq ([math]::Min(5, $Minutes))).wsMB
        $wsEnd = $samples[-1].wsMB
        $sum = [pscustomobject]@{
            name = $Name; exe = $Exe; args = $ArgumentList -join ' '; minutes = $Minutes
            ws5MB = $ws5; wsEndMB = $wsEnd; wsEndOverWs5 = [math]::Round($wsEnd / $ws5, 3); noLeak = ($wsEnd -le $ws5 * 1.1)
            cpuMedianPct = ($samples.cpuPct | Sort-Object)[[int][math]::Floor($samples.Count / 2)]
            samples = $samples
        }
        "== $Name : WS at 5 min $ws5 MB, at the end $wsEnd MB (x$($sum.wsEndOverWs5), no leak: $($sum.noLeak)), median CPU $($sum.cpuMedianPct)%"
    } else {
        Wait-Until $Wait
        $a = Snapshot $p.Id
        Start-Sleep $Window
        $b = Snapshot $p.Id
        $rows = Compare-Snapshots $a $b $Window
        $tot = Total $rows
        $sum = [pscustomobject]@{
            name = $Name; exe = $Exe; args = $ArgumentList -join ' '
            processes = $tot.processes; cpuPct = $tot.cpuPct; wsMB = $tot.wsMB; privMB = $tot.privMB
            byType = $rows | Sort-Object privMB -Descending
        }
        "== $Name : $($sum.processes) processes, CPU $($sum.cpuPct)% of one core, WS $($sum.wsMB) MB, private $($sum.privMB) MB"
        $rows | Sort-Object privMB -Descending | Format-Table -AutoSize | Out-String
    }
} finally {
    taskkill /PID $p.Id /T /F | Out-Null
}

Start-Sleep -Milliseconds 300
$out = Get-Content $log
$bar = $out | Select-String '^player_bar_ms=(\d+)' | Select-Object -First 1
if ($bar) { $sum | Add-Member playerBarMs ([int]$bar.Matches[0].Groups[1].Value) }
$sum | Add-Member focused $focused
$sum | ConvertTo-Json -Depth 4 | Set-Content (Join-Path $results "$Name.json")
$out
