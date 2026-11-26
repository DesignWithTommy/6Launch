# Launches GTA VI in the fxceless VIctory fork with the settings that measured
# best on this PC (RTX 4070 Ti, Ryzen 7 5800X, 2026-11-25).
#
# The two console modes, measured 2026-11-25 on this PC:
#   Series X (default): the game renders 2560x1440 with full-size textures.
#     Prison, settled: about 128 ms a frame (7.8 fps).
#     Open world (the boat of "Gone Fishing"): 148-155 ms (6.5-6.8 fps), 12 % of
#     frames over 250 ms: the 12 GB card is full and texture copies are thrown
#     away and made again all the time.
#   Series S (-SeriesS): the game renders 1280x720 and upscales; menus and prompts show.
#     Open world, driving the boat: 122 ms (8.2 fps), 0.3 % of frames over 250 ms,
#     7.3 GB of the card in use. The better choice outside.
#     Prison: slower than Series X on this build (about 145 ms).
#
# Usage:  .
un-gta6.ps1            play as a Series X
#         .
un-gta6.ps1 -SeriesS   play as a Series S (try this for the open world)
#         .
un-gta6.ps1 -Rebuild   rebuild the emulator first
# Close other programs that use the graphics card first (browser, overlays): they
# held about 2.5 GB of the card's 12 GB during testing.
param([switch]$Rebuild, [switch]$SeriesS, [switch]$Diag)

Set-Location $PSScriptRoot
# Two builds. The lean one (default) has no diagnostics counters: about 10 % faster
# afloat in "Gone Fishing" (111 against 123 ms a frame, Series S, warm caches). The
# diagnostics one (-Diag) writes the frame-time lines. Each has its own build folder
# and its own shader cache: the first launch of a build translates shaders for a few
# minutes and stays slow until its pipelines are made (one whole run here).
if ($Diag) {
    $exe = "$PSScriptRoot\target\release\victory.exe"
    if ($Rebuild -or -not (Test-Path $exe)) {
        cargo build --release -p victory --features diagnostics
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    }
} else {
    $exe = "$PSScriptRoot\target-lean\release\victory.exe"
    if ($Rebuild -or -not (Test-Path $exe)) {
        $env:CARGO_TARGET_DIR = "$PSScriptRoot\target-lean"
        cargo build --release -p victory --features keys
        $code = $LASTEXITCODE
        Remove-Item Env:CARGO_TARGET_DIR
        if ($code -ne 0) { exit $code }
    }
}

$env:VICTORY_LATE_WAIT = "3"
$env:VICTORY_WIDE_BACK = "1"
$env:VICTORY_CARD_BUFFERS_MIN_MB = "1"
$env:VICTORY_CARD_BUFFERS_MB = "1024"
$env:VICTORY_ASYNC_SUBMIT = "1"
$env:VICTORY_QUEUE2 = "1"
$env:VICTORY_INDIRECT_WINDOW = "1"   # if the GPU device is lost, remove this one first
$env:VICTORY_TEXTURE_NO_WAIT = "1"
# An experiment (fork PR #868): what shaders wrote to copies of write-combining
# title memory is not fetched back. It assumes the game's CPU code never reads
# that memory. 132 -> 116 ms here. If something looks wrong or stale, remove it.
$env:VICTORY_SKIP_WC_BACK = "1"
# Fork commits 3973179 and 465b94f. Together, same scene, one after the other:
# 159 -> 133 ms in Series X (6.2 -> 7.6 fps), submissions per frame 29 -> 9.
$env:VICTORY_JOIN_PARTS = "1"
$env:VICTORY_BACK_OWN_VIEW = "1"
$env:VICTORY_INDIRECT_GATHER = "1"
# Lighting stand-in (Geckomacabre's, after Flamingo and Belobug): while the game's
# ray-tracing structures are not built, every ray misses instead of walking
# memory that holds no structure. Same frame time at the boat (118 ms with,
# 123 without); lit interiors in the opening scenes.
$env:VICTORY_BVH_MISS = "1"
# VICTORY_AMBIENT (default 0.6) lights shaded places with a constant while the
# game's ray-traced light does not run; "off" gives the game's own output.
# Measured here and left off: VICTORY_ASYNC_PRESENT (frames arrive in pairs),
# VICTORY_COPY_PLAIN, VICTORY_WIDE_REFRESH, VICTORY_MERGE_GAP (no real gain).

# Where the game is: the full path of game_scarlett_master.exe, on the first
# line of game-path.txt beside this script (a file of your own; git ignores it).
$pathFile = Join-Path $PSScriptRoot "game-path.txt"
if (-not (Test-Path $pathFile)) {
    Write-Host "Put the full path of game_scarlett_master.exe in $pathFile and run this again." -ForegroundColor Yellow
    exit 1
}
$game = (Get-Content $pathFile -TotalCount 1).Trim().Trim('"')

$console = if ($SeriesS) { "series-s" } else { "series-x" }
& $exe run --console $console --execute $game
