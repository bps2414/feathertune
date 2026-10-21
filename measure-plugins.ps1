# Measures what every plugin costs, with measure.ps1: the same fixed track, the window in front, 3 runs per
# scenario, in a shuffled order per round so the drift of the site and of WebView2 does not fall on one side.
# Scenarios: base (defaults), alloff, off-<id> for every id in app\src\plugins.rs, tray, mini, lyrics (the
# Lyrics tab open) and lyrics-off (the same with synced_lyrics off), desktop-lyrics and share-lyrics open.
# Plugins are switched off with --measure --plugins-off=<ids> (in memory; config.toml is never touched).
# Output: results\<scenario>-<n>.json/.log, results\plugins-progress.log and results\plugins-summary.md
# (a median and min-max per scenario, the delta against base, "inside the noise" when it is smaller than
# the spread of the base runs). About 2 h for the short runs, plus an hour in the tray at the end.
#
#   ./measure-plugins.ps1 -Url 'https://music.youtube.com/watch?v=<id>'          # everything
#   ./measure-plugins.ps1 -Url ... -Only '^(base|off-ambient_mode)$' -SkipHour   # a few scenarios
#   ./measure-plugins.ps1 -SummaryOnly                                           # rebuild the summary
# Close feathertune first (the installed copy shares the profile) and leave the PC alone meanwhile.
param(
    [string]$Url,
    [int]$Runs = 3,
    [string]$Only = '',        # regex on the scenario names
    [int]$Minutes = 60,        # the tray leak test
    [switch]$SkipHour,
    [switch]$Resume,           # keep the runs that already have a valid result
    [switch]$NoWarmup,         # the lyrics cache and the profile are already warm
    [switch]$SummaryOnly
)
$ErrorActionPreference = 'Stop'
[System.Threading.Thread]::CurrentThread.CurrentCulture = [cultureinfo]::InvariantCulture
$root = $PSScriptRoot
$results = Join-Path $root 'results'
$profileDir = Join-Path $env:LOCALAPPDATA 'feathertune'
$progress = Join-Path $results 'plugins-progress.log'
New-Item -ItemType Directory -Force $results | Out-Null

function Log([string]$m) {
    $line = '{0:HH:mm:ss} {1}' -f (Get-Date), $m
    Write-Host $line
    Add-Content $progress $line
}

$ids = [regex]::Matches((Get-Content (Join-Path $root 'app\src\plugins.rs') -Raw), '(?m)^\s+id:\s*"([a-z0-9_]+)"') |
    ForEach-Object { $_.Groups[1].Value }
if (-not $ids) { throw 'no plugin ids found in app\src\plugins.rs' }

$scenarios = @(
    @{ Name = 'base'; Args = @() }
    @{ Name = 'alloff'; Args = @('--plugins-off=all') }
) + @($ids | ForEach-Object { @{ Name = "off-$_"; Args = @("--plugins-off=$_") } }) + @(
    @{ Name = 'tray'; Args = @(); Hide = 25; Wait = 50 }
    @{ Name = 'mini'; Args = @(); Mini = 25; Wait = 50 }
    @{ Name = 'lyrics'; Args = @('--open=lyrics-tab') }
    @{ Name = 'lyrics-off'; Args = @('--open=lyrics-tab', '--plugins-off=synced_lyrics') }
    @{ Name = 'desktop-lyrics'; Args = @('--open=desktop-lyrics') }
    @{ Name = 'share-lyrics'; Args = @('--open=share-lyrics') }
)
$active = @($scenarios | Where-Object { -not $Only -or $_.Name -match $Only })

# ---- checking a run ---------------------------------------------------------------------------------------

# '' when the run is good, else why not: the app closed early, no player bar, not playing, not signed in,
# not in front, or something asked with --open that did not open.
function Test-Run([string]$name, $s, [switch]$Sampling) {
    $jf = Join-Path $results "$name.json"; $lf = Join-Path $results "$name.log"
    if (-not (Test-Path $jf) -or -not (Test-Path $lf)) { return 'no result (the app closed early?)' }
    $r = Get-Content $jf -Raw | ConvertFrom-Json
    $log = @(Get-Content $lf)
    if (-not $r.playerBarMs) { return 'no player bar' }
    if (-not $Sampling -and $r.processes -lt 3) { return "only $($r.processes) processes" }
    # The status lines are JSON with sorted keys: {"loggedIn":true,"paused":false,"time":12}
    $st = $log | Select-String '^\s*\d+s \{"loggedIn":(\w+),"paused":(\w+),"time":(\w+)\}' | Select-Object -Last 1
    if (-not $st) { return 'no status line (the app closed early?)' }
    if ($st.Matches[0].Groups[3].Value -eq 'null') { return 'no player' }
    if ($st.Matches[0].Groups[2].Value -ne 'false') { return 'not playing' }
    if ($st.Matches[0].Groups[1].Value -ne 'true') { return 'not signed in' }
    # The time moves between the last two status lines.
    $times = @($log | Select-String '^\s*\d+s \{.*"time":(\d+)\}' | ForEach-Object { [int]$_.Matches[0].Groups[1].Value })
    if ($times.Count -ge 2 -and $times[-1] -le $times[-2]) { return 'the track is not advancing' }
    if ($log -match '^plugins-off: unknown') { return 'unknown plugin id' }
    foreach ($a in $s.Args) {
        if ($a -match '^--open=(.+)$') {
            if (-not ($log -match "^opened=$([regex]::Escape($Matches[1])) ms=\d+")) { return "$($Matches[1]) did not open" }
        }
    }
    if (-not $s.Hide -and -not $s.Mini -and -not $r.focused) { return 'window not in front' }
    ''
}

# ---- the summary ------------------------------------------------------------------------------------------

function Median($values) {
    $v = @($values | Where-Object { $null -ne $_ } | Sort-Object)
    if ($v.Count -eq 0) { return $null }
    if ($v.Count % 2) { $v[[int][math]::Floor($v.Count / 2)] } else { ($v[$v.Count / 2 - 1] + $v[$v.Count / 2]) / 2 }
}

# The valid runs of a scenario, as a table of columns, or $null.
function Get-Scenario($s) {
    $runs = @(1..$Runs | ForEach-Object {
        $name = "$($s.Name)-$_"
        if ((Test-Path (Join-Path $results "$name.json")) -and -not (Test-Run $name $s)) {
            Get-Content (Join-Path $results "$name.json") -Raw | ConvertFrom-Json
        }
    })
    if ($runs.Count -eq 0) { return $null }
    $col = {
        param($p)
        $v = @($runs | ForEach-Object { $_.$p } | Where-Object { $null -ne $_ } | Sort-Object)
        [pscustomobject]@{ med = (Median $v); min = $v[0]; max = $v[-1] }
    }
    [pscustomobject]@{ Name = $s.Name; n = $runs.Count; cpu = (& $col 'cpuPct'); priv = (& $col 'privMB'); ws = (& $col 'wsMB'); bar = (& $col 'playerBarMs') }
}

function Fmt($c, [int]$digits = 0) {
    if (-not $c) { return '-' }
    "{0} ({1}–{2})" -f [math]::Round($c.med, $digits), [math]::Round($c.min, $digits), [math]::Round($c.max, $digits)
}

# scenario - base, with a note when it is smaller than the spread of the base runs.
function Delta($c, $b, [int]$digits = 0, [double]$sign = 1) {
    if (-not $c -or -not $b) { return '-' }
    $d = $sign * ($c.med - $b.med)
    $text = ('{0:+0.#;-0.#;0}' -f [math]::Round($d, $digits)) -replace '^$', '0'
    if ($b.max -gt $b.min -and [math]::Abs($d) -lt ($b.max - $b.min)) { $text += ' (inside the noise)' }
    $text
}

function Write-Summary {
    $data = @{}
    foreach ($s in $scenarios) { $d = Get-Scenario $s; if ($d) { $data[$s.Name] = $d } }
    $base = $data['base']
    $md = [System.Collections.Generic.List[string]]::new()
    $md.Add("# Plugin measurements ($(Get-Date -Format 'yyyy-MM-dd'))")
    $md.Add('')
    $md.Add("Median (min–max) of $Runs runs, one fixed track, window 1280x800 in front. Delta = scenario − base. ""Inside the noise"": smaller than the spread (max − min) of the base runs.")
    $md.Add('')
    $md.Add('| Scenario | runs | CPU % | Δ CPU | private MB | Δ private | WS MB | Δ WS | to bar ms |')
    $md.Add('|---|---|---|---|---|---|---|---|---|')
    foreach ($s in $scenarios) {
        $d = $data[$s.Name]
        if (-not $d) { $md.Add("| $($s.Name) | 0 | - | - | - | - | - | - | - |"); continue }
        $md.Add("| $($d.Name) | $($d.n) | $(Fmt $d.cpu 1) | $(Delta $d.cpu $base.cpu 1) | $(Fmt $d.priv) | $(Delta $d.priv $base.priv) | $(Fmt $d.ws) | $(Delta $d.ws $base.ws) | $(Fmt $d.bar) |")
    }
    if ($base) {
        $md.Add('')
        $md.Add('## What each plugin costs (base − off)')
        $md.Add('')
        $md.Add('Positive: the plugin costs that much. Largest private memory first.')
        $md.Add('')
        $md.Add('| Plugin | CPU points | private MB | WS MB |')
        $md.Add('|---|---|---|---|')
        $rows = foreach ($id in (@('alloff') + $ids)) {
            $d = if ($id -eq 'alloff') { $data['alloff'] } else { $data["off-$id"] }
            if ($d) { [pscustomobject]@{ id = $(if ($id -eq 'alloff') { 'ALL plugins' } else { $id }); d = $d } }
        }
        foreach ($row in ($rows | Sort-Object { - ($base.priv.med - $_.d.priv.med) })) {
            $md.Add("| $($row.id) | $(Delta $row.d.cpu $base.cpu 1 -1) | $(Delta $row.d.priv $base.priv 0 -1) | $(Delta $row.d.ws $base.ws 0 -1) |")
        }
        if ($data['lyrics'] -and $data['lyrics-off']) {
            $md.Add('')
            $md.Add("Lyrics tab open, synced_lyrics on against off (cross-fade, scroll, word highlight): CPU $(Delta $data['lyrics'].cpu $data['lyrics-off'].cpu 1), private $(Delta $data['lyrics'].priv $data['lyrics-off'].priv), WS $(Delta $data['lyrics'].ws $data['lyrics-off'].ws).")
        }
    }
    $hour = Join-Path $results 'tray-1h.json'
    if (Test-Path $hour) {
        $h = Get-Content $hour -Raw | ConvertFrom-Json
        $md.Add('')
        $md.Add('## One hour in the tray')
        $md.Add('')
        $md.Add("WS at 5 min $($h.ws5MB) MB, at $($h.minutes) min $($h.wsEndMB) MB (x$($h.wsEndOverWs5)), no leak: $($h.noLeak), median CPU $($h.cpuMedianPct)%.")
    }
    $bad = foreach ($s in $scenarios) {
        foreach ($i in 1..$Runs) {
            $name = "$($s.Name)-$i"
            if (Test-Path (Join-Path $results "$name.json")) { $why = Test-Run $name $s; if ($why) { "- ${name}: $why" } }
        }
    }
    if ($bad) { $md.Add(''); $md.Add('## Runs left out'); $md.Add(''); $bad | ForEach-Object { $md.Add($_) } }
    Set-Content (Join-Path $results 'plugins-summary.md') $md
    Log "summary written: $(Join-Path $results 'plugins-summary.md')"
}

if ($SummaryOnly) { Write-Summary; return }

# ---- running ----------------------------------------------------------------------------------------------

if (-not $Url) { throw '-Url (the track to play in every run) is required' }
if (Get-Process -Name feathertune -ErrorAction SilentlyContinue) {
    throw 'feathertune is running (the installed copy shares the profile). Quit it from the tray first.'
}
$exe = Join-Path $root 'app\target\release\feathertune.exe'
if (-not (Test-Path $exe)) { throw "$exe not found: cargo build --release in app\ first" }
$newest = Get-ChildItem (Join-Path $root 'app\src'), (Join-Path $root 'app\injections') -Recurse -File | Sort-Object LastWriteTime | Select-Object -Last 1
if ($newest.LastWriteTime -gt (Get-Item $exe).LastWriteTime) { Log "warning: $($newest.Name) is newer than the release exe; build first if it matters" }

# Window position/state files and the play history change while the app runs; put them back at the end.
# A marker says the backup holds the originals; a run that was killed before restoring leaves both, and
# the next one must not back up the changed files over them.
$backup = Join-Path $results 'backup'
$marker = Join-Path $backup 'taken'
New-Item -ItemType Directory -Force $backup | Out-Null
$keep = @('desktop-lyrics.json', 'mini-player.json', 'stats\plays.jsonl')
if (-not (Test-Path $marker)) {
    foreach ($f in $keep) {
        $src = Join-Path $profileDir $f
        if (Test-Path $src) { Copy-Item $src (Join-Path $backup ($f -replace '\\', '_')) -Force }
    }
    Set-Content $marker (Get-Date -Format o)
}
# Every run starts with the floating lyrics and the mini player closed.
function Reset-UiState {
    foreach ($f in 'desktop-lyrics.json', 'mini-player.json') { Remove-Item (Join-Path $profileDir $f) -ErrorAction SilentlyContinue }
}
function Restore-State {
    foreach ($f in $keep) {
        $src = Join-Path $profileDir $f
        $dst = Join-Path $backup ($f -replace '\\', '_')
        if (Test-Path $dst) { Copy-Item $dst $src -Force; Remove-Item $dst } else { Remove-Item $src -ErrorAction SilentlyContinue }
    }
    Remove-Item $marker -ErrorAction SilentlyContinue
}
function Wait-Free {
    $n = 0
    while (Get-Process -Name feathertune -ErrorAction SilentlyContinue) {
        if ($n++ -eq 0) { Log 'feathertune is running; waiting for it to close' }
        if ($n -gt 60) { throw 'feathertune is still running after 30 min' }
        Start-Sleep 30
    }
}

$measure = Join-Path $root 'measure.ps1'
function Invoke-Scenario($s, [string]$name) {
    $a = @('--measure', $Url) + $s.Args
    $p = @{ Name = $name; ArgumentList = $a; Wait = $(if ($s.Wait) { $s.Wait } else { 45 }) }
    if ($s.Hide) { $p.HideAfter = $s.Hide }
    if ($s.Mini) { $p.MiniAfter = $s.Mini }
    $why = 'not run'
    for ($try = 1; $try -le 3; $try++) {
        Wait-Free
        Reset-UiState
        Remove-Item (Join-Path $results "$name.json") -ErrorAction SilentlyContinue
        try { & $measure @p | Out-Null } catch { Log "error in ${name}: $($_.Exception.Message)" }
        Start-Sleep 5
        $why = Test-Run $name $s
        if (-not $why) { return }
        Log "left out $name (try $try): $why"
    }
    Log "GAVE UP on ${name}: $why"
}

$total = $active.Count * $Runs
Log "start: $($active.Count) scenarios x $Runs runs = $total runs, about $([math]::Round($total * 72 / 3600, 1)) h$(if (-not $SkipHour -and 'tray-1h' -match $Only) { ", then $Minutes min in the tray" })"
try {
    # One discarded run: fills the lyrics cache and the audio facts of the track, warms the profile.
    # It must be good, or nothing after it can be (URL, sign-in, network).
    if (-not $NoWarmup) {
        Log 'warmup'
        Invoke-Scenario @{ Name = 'warmup'; Args = @() } 'warmup'
        if (Test-Run 'warmup' @{ Name = 'warmup'; Args = @() }) { throw 'the warmup run failed; see results\warmup.log' }
    }

    $done = 0
    for ($round = 1; $round -le $Runs; $round++) {
        $order = Get-Random -SetSeed (100 + $round) -InputObject $active -Count $active.Count
        foreach ($s in $order) {
            $name = "$($s.Name)-$round"
            $done++
            if ($Resume -and (Test-Path (Join-Path $results "$name.json")) -and -not (Test-Run $name $s)) { continue }
            Log "[$done/$total] $name"
            Invoke-Scenario $s $name
        }
    }

    if (-not $SkipHour -and 'tray-1h' -match $Only) {
        Log "tray-1h ($Minutes min)"
        Wait-Free
        Reset-UiState
        Remove-Item (Join-Path $results 'tray-1h.json') -ErrorAction SilentlyContinue
        & $measure -Name tray-1h -ArgumentList @('--measure', $Url) -HideAfter 25 -Minutes $Minutes | Out-Null
        $why = Test-Run 'tray-1h' @{ Name = 'tray-1h'; Args = @(); Hide = 25 } -Sampling
        if ($why) { Log "tray-1h is not good: $why" } else { Log 'tray-1h done' }
    }
} finally {
    Restore-State
    Write-Summary
}
