# Map tour (feat/map-tour, fork AntoninMichelq/VIctory, a3754b4 on prison-stack-a 0dbc516)

Goal: visit many places of the map, let each stream in fully, so every shader/pipeline
gets translated + compiled + cached (pipeline library and early-start list already exist in
victory-gpu pipeline/library.rs, so the tour fills them for free), and log what is wrong per place.

## How
- VICTORY_TOUR=grid:<step m>:<rings> (spiral around the start) or a file of `x y z [name]` lines.
- Player found in memory: scan writable RAM for 4x4 placement matrices, walk W 6 s, hold 3 s,
  walk S 6 s; keep matrices that followed (player + physics + camera). Then their position
  (bytes 48..60) is rewritten every 15 ms at each place = player hangs at the point
  (VICTORY_TOUR_HEIGHT m above start z; Leonida is flat).
- Settled = >= MIN s, draws/frame within 5% over 3 x 5 s windows, no pipeline waiting; or MAX s.
- Then: 1 picture + 3 quarter-turn pictures (right stick L), line in tour-results.csv
  (draws/frame, fps, textures waiting, pipelines waiting, private MB, frame number).
- tour-state.txt: resume after a crash; the crashed point is written as "crashed" and skipped.
- Needs a diagnostics build (`cargo build --release -p victory --features diagnostics`), and
  VICTORY_CAPTURE for pictures. Launcher draft: Tour-de-la-map.bat (copy to _work).

## Known limits / next
- Hovering only (no landing): ground-level LODs may not all load. Next: per-point z from file.
- If the scan finds 0 or >64 matrices it retries 4x (60 s apart) then gives up (log "tour:").
- Rebuild of the bat on the latest Jouer base when other threads finish.
