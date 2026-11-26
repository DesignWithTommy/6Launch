# Measurements

Build bc090416 (`cargo build --profile diag -p victory --features diagnostics`), 2026-11-26,
on the test PC in the README. The benchmark used revision `7d62e536`.
The only change between that revision and `bc090416` is in the README. Each session: `scripts\play.ps1` with the
RTX 4090 settings, full screen, past the start screen into the prologue, then
every minute a capture, the camera turned, a walk, and a route (run, turn,
look around, step aside) with another capture.

Frame columns use recorded in-world samples with over 1,000 draws. Memory and card columns are samples every 20 s over the whole
session, loading included.

| Profile | world lines | fps median | p10 | p90 | 1% low | ms/frame | preparing | copies ms / MB | textures ms / MB | GPU ms | copying back ms / MB | draws | dropped draws | captures |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| Series S | 98 | 21.5 | 18.4 | 24.0 | 7.0 | 46.5 | 32.3 | 12.4 / 82.0 | 0.5 / 31.1 | 0.1 | 12.3 / 29.5 | 3830.5 | 78.0 | 41 |

| Profile | samples | game memory median / max (GB) | working set max (GB) | card memory max (GB) | card load median (%) | power median (W) | temperature max (C) | clock median (MHz) | CPU seconds per minute |
|---|---|---|---|---|---|---|---|---|---|
| Series S | 31 | 26.4 / 27.9 | 29.6 | 11.6 | 85.0 | 134.2 | 59.4 | 2760.0 | 532 |

CPU seconds per minute: the game's CPU time over all its threads; 60 is one core busy all the time.
