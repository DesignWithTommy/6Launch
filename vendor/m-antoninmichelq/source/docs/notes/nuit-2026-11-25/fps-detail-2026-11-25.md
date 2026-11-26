# Détail fps (lignes brutes du log), 25/11

## 1/3 Desktop tout3 8006255, partie B chaude (run-20261125-085252-jouer-tout.log)

```
FPS data 1/3: Desktop tout3 (8006255), warm run B (run-20261125-085252-jouer-tout.log).
Window: the last 10 fps lines of the steady part, 08:56:12–08:56:59 local, player near the start. I didn't use the log's last 10 lines because they're a cutscene at 0.5–1.2 fps. Lines are raw up to "heap buffers … ms;"; the texture-slot/depth-twin tails are cut.
2026-11-25T07:56:12.071491Z  INFO victory_gpu::host: 5.7 fps, 176.9 ms/frame (59.5 preparing: copies 36.0 ms 129.3 MB, textures 3.6 ms 25.2 MB; 109.1 on the GPU; 83.0 copying back 60.2 MB), 4652 draws/frame, dropped 344 without a host pipeline (0 being made) and 0 indirect; heap buffers 1.0 passes 30.0 ms;
2026-11-25T07:56:17.269610Z  INFO victory_gpu::host: 6.0 fps, 165.5 ms/frame (58.9 preparing: copies 34.3 ms 122.4 MB, textures 3.3 ms 21.4 MB; 101.1 on the GPU; 79.8 copying back 58.1 MB), 4652 draws/frame, dropped 343 without a host pipeline (0 being made) and 0 indirect; heap buffers 1.0 passes 31.8 ms;
2026-11-25T07:56:22.603400Z  INFO victory_gpu::host: 6.1 fps, 163.5 ms/frame (58.2 preparing: copies 34.0 ms 123.4 MB, textures 3.2 ms 21.2 MB; 100.4 on the GPU; 79.8 copying back 58.3 MB), 4652 draws/frame, dropped 333 without a host pipeline (0 being made) and 0 indirect; heap buffers 1.0 passes 31.2 ms;
2026-11-25T07:56:27.943489Z  INFO victory_gpu::host: 6.1 fps, 165.2 ms/frame (58.3 preparing: copies 34.5 ms 124.2 MB, textures 3.5 ms 24.0 MB; 101.6 on the GPU; 79.4 copying back 58.6 MB), 4660 draws/frame, dropped 342 without a host pipeline (0 being made) and 0 indirect; heap buffers 1.0 passes 24.9 ms;
2026-11-25T07:56:33.149113Z  INFO victory_gpu::host: 6.0 fps, 166.9 ms/frame (58.6 preparing: copies 34.8 ms 122.8 MB, textures 3.5 ms 24.7 MB; 103.4 on the GPU; 78.8 copying back 56.6 MB), 4657 draws/frame, dropped 332 without a host pipeline (0 being made) and 0 indirect; heap buffers 1.0 passes 30.6 ms;
2026-11-25T07:56:38.400410Z  INFO victory_gpu::host: 6.0 fps, 167.3 ms/frame (58.1 preparing: copies 35.1 ms 123.7 MB, textures 3.2 ms 20.6 MB; 103.7 on the GPU; 80.3 copying back 57.8 MB), 4651 draws/frame, dropped 338 without a host pipeline (0 being made) and 0 indirect; heap buffers 1.0 passes 18.8 ms;
2026-11-25T07:56:43.608495Z  INFO victory_gpu::host: 5.9 fps, 168.1 ms/frame (56.9 preparing: copies 33.5 ms 127.9 MB, textures 3.3 ms 24.1 MB; 106.4 on the GPU; 81.5 copying back 59.5 MB), 4619 draws/frame, dropped 342 without a host pipeline (0 being made) and 0 indirect; heap buffers 1.0 passes 12.2 ms;
2026-11-25T07:56:49.110837Z  INFO victory_gpu::host: 6.1 fps, 164.3 ms/frame (57.4 preparing: copies 34.2 ms 127.2 MB, textures 3.1 ms 20.7 MB; 102.2 on the GPU; 78.5 copying back 59.0 MB), 4612 draws/frame, dropped 337 without a host pipeline (0 being made) and 0 indirect; heap buffers 1.0 passes 20.4 ms;
2026-11-25T07:56:54.627088Z  INFO victory_gpu::host: 6.0 fps, 167.0 ms/frame (58.9 preparing: copies 34.8 ms 123.5 MB, textures 3.5 ms 22.2 MB; 101.0 on the GPU; 78.9 copying back 58.2 MB), 4619 draws/frame, dropped 338 without a host pipeline (0 being made) and 0 indirect; heap buffers 1.0 passes 23.9 ms;
2026-11-25T07:56:59.938800Z  INFO victory_gpu::host: 4.0 fps, 250.7 ms/frame (95.5 preparing: copies 37.7 ms 148.6 MB, textures 36.3 ms 41.3 MB; 98.0 on the GPU; 56.0 copying back 58.4 MB), 4577 draws/frame, dropped 394 without a host pipeline (0 being made) and 0 indirect; heap buffers 1.0 passes 88.9 ms;
Last 3 card gather lines (raw):
2026-11-25T07:56:57.664567Z  INFO victory_gpu::host: card gather per frame (VICTORY_CARD_GATHER=1): 757 pieces (48.9 MB) merged from what the card copied out within their submission, 361 fetched after all (360 past a plan's room of 160, 1 of pieces merged from a newer fetch since)
2026-11-25T07:56:58.685214Z  INFO victory_gpu::host: card gather per frame (VICTORY_CARD_GATHER=1): 754 pieces (48.7 MB) merged from what the card copied out within their submission, 350 fetched after all (349 past a plan's room of 160, 1 of pieces merged from a newer fetch since)
2026-11-25T07:56:59.938689Z  INFO victory_gpu::host: card gather per frame (VICTORY_CARD_GATHER=1): 731 pieces (47.1 MB) merged from what the card copied out within their submission, 326 fetched after all (325 past a plan's room of 160, 1 of pieces merged from a newer fetch since)
Averages over the 10 lines:
- 175.5 ms/frame (the first 9 average about 167; the last one, 250.7, is the start of a new area)
- preparing 62.0 ms: copies 34.9 ms / 127.3 MB, textures 6.7 ms / 24.5 MB
- on the GPU 102.7 ms
- copying back 77.6 ms / 58.5 MB
- heap buffers passes 31.3 ms
- dropped 344.3
- being made 0
ASYNC: no VICTORY_ASYNC in the Desktop bat or the log. LATE_WAIT: the bat sets VICTORY_LATE_WAIT=3 (the log's "VICTORY_LATE_WAIT=1/2" tags are per-mode labels), so it's on.
tout5b B and C will follow when they end.
```

## 2/3 tout5b bd85106, partie B chaude (ALIAS_COPIES=0, DETILE=6)

```
FPS data 2/3: tout5b B, last 10 fps lines (raw, cut after "heap buffers … ms;"):
2026-11-25T09:48:38.269947Z  INFO victory_gpu::host: 4.5 fps, 219.9 ms/frame (65.7 preparing: copies 42.1 ms 167.0 MB, textures 3.8 ms 26.5 MB; 58.5 on the GPU; 50.8 copying back 55.1 MB), 4677 draws/frame, dropped 343 without a host pipeline (0 being made) and 0 indirect; heap buffers 1.0 passes 109.3 ms;
2026-11-25T09:48:43.710205Z  INFO victory_gpu::host: 4.6 fps, 216.5 ms/frame (66.2 preparing: copies 43.0 ms 168.4 MB, textures 3.5 ms 25.0 MB; 65.8 on the GPU; 50.5 copying back 55.8 MB), 4678 draws/frame, dropped 344 without a host pipeline (0 being made) and 0 indirect; heap buffers 1.0 passes 88.5 ms;
2026-11-25T09:48:49.160653Z  INFO victory_gpu::host: 4.5 fps, 220.6 ms/frame (65.4 preparing: copies 41.6 ms 164.3 MB, textures 3.5 ms 24.4 MB; 71.9 on the GPU; 52.7 copying back 55.7 MB), 4670 draws/frame, dropped 343 without a host pipeline (0 being made) and 0 indirect; heap buffers 1.0 passes 93.8 ms;
2026-11-25T09:48:54.594939Z  INFO victory_gpu::host: 4.6 fps, 216.7 ms/frame (65.1 preparing: copies 41.8 ms 166.9 MB, textures 3.6 ms 25.2 MB; 65.1 on the GPU; 51.8 copying back 55.1 MB), 4668 draws/frame, dropped 344 without a host pipeline (0 being made) and 0 indirect; heap buffers 1.0 passes 92.3 ms;
2026-11-25T09:49:00.045435Z  INFO victory_gpu::host: 4.7 fps, 213.0 ms/frame (64.9 preparing: copies 42.5 ms 168.9 MB, textures 2.1 ms 8.5 MB; 61.4 on the GPU; 52.2 copying back 54.7 MB), 4664 draws/frame, dropped 339 without a host pipeline (0 being made) and 0 indirect; heap buffers 1.0 passes 96.5 ms;
2026-11-25T09:49:05.470574Z  INFO victory_gpu::host: 4.7 fps, 213.6 ms/frame (62.8 preparing: copies 41.2 ms 169.3 MB, textures 2.3 ms 10.4 MB; 54.0 on the GPU; 51.4 copying back 55.6 MB), 4667 draws/frame, dropped 339 without a host pipeline (0 being made) and 0 indirect; heap buffers 1.0 passes 110.4 ms;
2026-11-25T09:49:10.982106Z  INFO victory_gpu::host: 4.6 fps, 216.8 ms/frame (64.2 preparing: copies 41.6 ms 165.5 MB, textures 2.1 ms 8.8 MB; 65.7 on the GPU; 50.2 copying back 53.3 MB), 4673 draws/frame, dropped 330 without a host pipeline (0 being made) and 0 indirect; heap buffers 1.0 passes 90.8 ms;
2026-11-25T09:49:16.531784Z  INFO victory_gpu::host: 4.5 fps, 223.5 ms/frame (65.3 preparing: copies 41.9 ms 166.3 MB, textures 3.6 ms 27.3 MB; 59.3 on the GPU; 50.9 copying back 54.8 MB), 4669 draws/frame, dropped 341 without a host pipeline (0 being made) and 0 indirect; heap buffers 1.0 passes 111.2 ms;
2026-11-25T09:49:21.953621Z  INFO victory_gpu::host: 4.6 fps, 217.5 ms/frame (66.0 preparing: copies 42.5 ms 167.2 MB, textures 3.5 ms 26.3 MB; 66.5 on the GPU; 49.8 copying back 53.9 MB), 4665 draws/frame, dropped 336 without a host pipeline (0 being made) and 0 indirect; heap buffers 1.0 passes 88.6 ms;
2026-11-25T09:49:27.433414Z  INFO victory_gpu::host: 4.3 fps, 230.3 ms/frame (70.0 preparing: copies 40.6 ms 165.9 MB, textures 9.8 ms 25.3 MB; 66.8 on the GPU; 49.8 copying back 54.3 MB), 4737 draws/frame, dropped 348 without a host pipeline (0 being made) and 0 indirect; heap buffers 1.2 passes 127.1 ms;
Last 3 gather lines (raw):
2026-11-25T09:49:29.599264Z  INFO victory_gpu::host: card gather per frame (VICTORY_CARD_GATHER=1): 696 pieces (44.8 MB) merged from what the card copied out within their submission, 270 fetched after all (269 past a plan's room of 160, 1 of pieces merged from a newer fetch since)
2026-11-25T09:49:30.692734Z  INFO victory_gpu::host: card gather per frame (VICTORY_CARD_GATHER=1): 694 pieces (44.4 MB) merged from what the card copied out within their submission, 275 fetched after all (274 past a plan's room of 160, 1 of pieces merged from a newer fetch since)
2026-11-25T09:49:31.786401Z  INFO victory_gpu::host: card gather per frame (VICTORY_CARD_GATHER=1): 713 pieces (45.9 MB) merged from what the card copied out within their submission, 271 fetched after all (270 past a plan's room of 160, 1 of pieces merged from a newer fetch since)
Averages vs tout3 B:
- ms/frame 218.8 (vs 175.5 / about 167)
- preparing 65.6: copies 41.9 ms / 167.0 MB (vs 34.9 / 127.3), textures 3.8 / 20.8
- on the GPU 63.5 (vs 102.7)
- copying back 51.0 / 54.8 MB (vs 77.6 / 58.5)
- HEAP BUFFERS PASSES 100.9 ms (vs 31.3); this is the big regression
- dropped 340.7, being made 0
Same LATE_WAIT=3 / no ASYNC as the Desktop. Note tout5b runs ALIAS_COPIES=0 and DETILE=6.
C (GATHER_PIECES=512) has been running since 10:49:39. Relay lines and D are next.
```

## tout3 B : relais, copies, attentes (même fenêtre)

```
FPS data: tout3 B relay/copies/settle lines, window 08:56:12–08:56:59 (run-20261125-085252-jouer-tout.log), raw.
Relay (last 5):
2026-11-25T07:56:55.636728Z  INFO victory_gpu::host: the relay per frame (VICTORY_RELAY=1): 33.5 flights landed in 14.8 landings by the thread that copies back, beside the submitting thread; that thread waited 52.9 ms for the card with nothing to land and 58.3 ms for fetches of card copies' pieces, and was 81.8 ms landing in all (the frame line's "copying back" is its time; "settles" above are waits of other threads for it)
2026-11-25T07:56:56.643280Z  INFO victory_gpu::host: the relay per frame (VICTORY_RELAY=1): 37.8 flights landed in 16.3 landings by the thread that copies back, beside the submitting thread; that thread waited 55.7 ms for the card with nothing to land and 57.5 ms for fetches of card copies' pieces, and was 80.9 ms landing in all (the frame line's "copying back" is its time; "settles" above are waits of other threads for it)
2026-11-25T07:56:57.664564Z  INFO victory_gpu::host: the relay per frame (VICTORY_RELAY=1): 35.3 flights landed in 15.3 landings by the thread that copies back, beside the submitting thread; that thread waited 52.5 ms for the card with nothing to land and 60.2 ms for fetches of card copies' pieces, and was 84.6 ms landing in all (the frame line's "copying back" is its time; "settles" above are waits of other threads for it)
2026-11-25T07:56:58.685207Z  INFO victory_gpu::host: the relay per frame (VICTORY_RELAY=1): 32.6 flights landed in 15.4 landings by the thread that copies back, beside the submitting thread; that thread waited 63.3 ms for the card with nothing to land and 46.9 ms for fetches of card copies' pieces, and was 71.1 ms landing in all (the frame line's "copying back" is its time; "settles" above are waits of other threads for it)
2026-11-25T07:56:59.938686Z  INFO victory_gpu::host: the relay per frame (VICTORY_RELAY=1): 30.4 flights landed in 17.4 landings by the thread that copies back, beside the submitting thread; that thread waited 84.3 ms for the card with nothing to land and 35.5 ms for fetches of card copies' pieces, and was 57.7 ms landing in all (the frame line's "copying back" is its time; "settles" above are waits of other threads for it)
Host copies in the card's memory (last 3):
2026-11-25T07:56:49.110765Z  INFO victory_gpu::host: host copies in the card's memory: 97 copies 881 MB, their bytes from before 0 MB; per frame: 87.3 MB sent up in 698 ranges, 15902 pieces compared on the card, 352 of them fetched (22.9 MB) with 3.3 more waits for the card (57.1 ms)
2026-11-25T07:56:54.627014Z  INFO victory_gpu::host: host copies in the card's memory: 97 copies 881 MB, their bytes from before 0 MB; per frame: 83.1 MB sent up in 583 ranges, 15917 pieces compared on the card, 343 of them fetched (22.3 MB) with 3.1 more waits for the card (57.1 ms)
2026-11-25T07:56:59.938727Z  INFO victory_gpu::host: host copies in the card's memory: 97 copies 881 MB, their bytes from before 0 MB; per frame: 89.8 MB sent up in 269 ranges, 15916 pieces compared on the card, 326 of them fetched (21.2 MB) with 3.0 more waits for the card (35.5 ms)
Why/settle lines (last 2 pairs):
2026-11-25T07:56:58.685195Z  INFO victory_gpu::host: submissions sent ahead per frame: 31.8 in 3.4 settles (0.0 flights each time), 104.1 ms waiting for the card in them; settled for: a synchronous submission or present 0.0, the card had run them 0.0, a wait for a memory value 0.0, title memory changed under shader writes 2.4, title memory read 1.0 (by a buffer copy 0.0, a fill of upload memory 1.0, a texture 0.0), a step of ours 0.0, 128 lists in the air 0.0, bottom-of-pipe writes (VICTORY_LATE_WAIT_WRITES=0) 0.0, a blocking fence wait 0.0; 15.0 waits for memory values met by a flight's bottom-of-pipe write, 18.4 fence signals set behind flights
2026-11-25T07:56:58.685203Z  INFO victory_gpu::host: of the waiting for the card in settles, per frame, by what settled: a synchronous submission or present 0.0 ms, the card had run them 0.0, a wait for a memory value 0.0, title memory changed under shader writes 92.5, title memory read 11.5, a step of ours 0.0, lists in the air 0.0, bottom-of-pipe writes 0.0, a blocking fence wait 0.0
2026-11-25T07:56:59.938675Z  INFO victory_gpu::host: submissions sent ahead per frame: 30.4 in 4.0 settles (0.0 flights each time), 98.0 ms waiting for the card in them; settled for: a synchronous submission or present 0.0, the card had run them 0.0, a wait for a memory value 0.0, title memory changed under shader writes 3.0, title memory read 1.0 (by a buffer copy 0.0, a fill of upload memory 1.0, a texture 0.0), a step of ours 0.0, 128 lists in the air 0.0, bottom-of-pipe writes (VICTORY_LATE_WAIT_WRITES=0) 0.0, a blocking fence wait 0.0; 13.6 waits for memory values met by a flight's bottom-of-pipe write, 16.6 fence signals set behind flights
2026-11-25T07:56:59.938682Z  INFO victory_gpu::host: of the waiting for the card in settles, per frame, by what settled: a synchronous submission or present 0.0 ms, the card had run them 0.0, a wait for a memory value 0.0, title memory changed under shader writes 73.3, title memory read 24.7, a step of ours 0.0, lists in the air 0.0, bottom-of-pipe writes 0.0, a blocking fence wait 0.0
Summary: about 100 ms per frame of settle waits, 75–95 ms of them for "title memory changed under shader writes". The relay thread waits about 50–85 ms for the card and 35–60 ms for piece fetches.
Run D (diag GPU timing) is being queued after C.
```

## tout3 B : rafraîchissements alias (WARN)

```
tout3 B, WARN "a host copy refreshed from title memory that another copy of it is written in", kind "a flight's shaders write it": 74 lines in the run. It's logged every 1000 (seen=N001). Last 5 raw:
2026-11-25T08:02:51.840157Z  WARN victory_gpu::host: a host copy refreshed from title memory that another copy of it is written in kind="a flight's shaders write it" seen=58001 reader=0xda1800000+0x400000 range=0xda1870000+0xfffec writer=-
2026-11-25T08:03:12.960238Z  WARN victory_gpu::host: a host copy refreshed from title memory that another copy of it is written in kind="a flight's shaders write it" seen=59001 reader=0xdb9c00000+0x400000 range=0xdb9c10000+0x60000 writer=-
2026-11-25T08:03:26.197053Z  WARN victory_gpu::host: a host copy refreshed from title memory that another copy of it is written in kind="a flight's shaders write it" seen=60001 reader=0xdba000000+0x400000 range=0xdba008e00+0x47200 writer=-
2026-11-25T08:03:39.249529Z  WARN victory_gpu::host: a host copy refreshed from title memory that another copy of it is written in kind="a flight's shaders write it" seen=61001 reader=0xd60800000+0x400000 range=0xd60980000+0xeffd8 writer=-
2026-11-25T08:04:10.458728Z  WARN victory_gpu::host: a host copy refreshed from title memory that another copy of it is written in kind="a flight's shaders write it" seen=62001 reader=0xdb9c00000+0x400000 range=0xdb9c10000+0x60000 writer=-
seen= growth (local time):
- 08:56:12 seen=5001 → 08:56:55 seen=14001: 9,000 in 43 s ≈ 210/s. At about 6 fps (≈ 260 frames) that's ≈ 35 per frame in steady play at the save start.
- 08:58:02 seen=20001 → 09:04:10 seen=62001: 42,000 in 368 s ≈ 114/s. That stretch is the moving/cutscene part at 1–5 fps, so ≈ 25–35 per frame.
- Estimate: about 30 (25–35) refreshes per frame. Frequent readers: 0xd49401c20+0x8000 (21 of the logged lines), 0xd914cf620+0x400 (8), 0xdb9c00000+0x400000 (6). writer=- in all.
Run G is being added after D and before tout3q2.
```

## 3/3 tout5b C : B + CARD_GATHER_PIECES=512

```
tout5b C (B + CARD_GATHER_PIECES=512, warm cache tout5b), 12 min, Alt+F4 at 11:01:46, exited at 11:01:54. No AV, no device removed.
- World at 10:51:51 (+132 s).
- Last 10: fps 4.63 (B 4.56). ms per frame 216.8 (B 218.8).
- Preparing 69.2: copies 40.9 ms / 169.9 MB, textures 8.5 / 21.4.
- On the GPU 60.5 (B 63.5).
- COPYING BACK 22.0 ms / 55.9 MB (B 51.0): halved.
- Heap buffers passes 96.1 (B 100.9). Dropped 340.8, being made 0.
- Gather (raw, last):
2026-11-25T10:01:46.525803Z  INFO victory_gpu::host: card gather per frame (VICTORY_CARD_GATHER=1): 999 pieces (64.4 MB) merged from what the card copied out within their submission, 0 fetched after all (0 past a plan's room of 512, 0 of pieces merged from a newer fetch since)
  B had about 700 merged and 270 fetched after (past 160). C merges about 990, with 0 fetched after.
- Memory max 22,042 MB (B 21,626), last 21,013.
- Your rule: fps ≥ B ✓, memory ≤ B + 300 MB ✗ (+416 MB). So 512 is DROPPED by the rule, and E runs without it. Borderline: the copy-back saving is real, but fps only gains +0.07 because heap passes (~96 ms) dominate.
- Last 10 fps lines (raw, cut at "heap buffers … ms;"):
2026-11-25T10:00:56.523013Z  INFO victory_gpu::host: 4.8 fps, 210.4 ms/frame (65.5 preparing: copies 42.0 ms 172.0 MB, textures 3.8 ms 26.4 MB; 56.4 on the GPU; 22.6 copying back 56.8 MB), 4580 draws/frame, dropped 340 without a host pipeline (0 being made) and 0 indirect; heap buffers 0.8 passes 74.5 ms;
2026-11-25T10:01:01.823725Z  INFO victory_gpu::host: 4.8 fps, 208.4 ms/frame (64.7 preparing: copies 40.9 ms 169.0 MB, textures 3.5 ms 25.3 MB; 59.1 on the GPU; 22.1 copying back 54.7 MB), 4656 draws/frame, dropped 336 without a host pipeline (0 being made) and 0 indirect; heap buffers 1.0 passes 91.0 ms;
2026-11-25T10:01:07.260737Z  INFO victory_gpu::host: 4.1 fps, 243.1 ms/frame (92.7 preparing: copies 39.5 ms 169.4 MB, textures 33.5 ms 28.7 MB; 60.8 on the GPU; 21.0 copying back 56.8 MB), 4658 draws/frame, dropped 338 without a host pipeline (0 being made) and 0 indirect; heap buffers 1.0 passes 114.6 ms;
2026-11-25T10:01:12.595170Z  INFO victory_gpu::host: 4.7 fps, 213.6 ms/frame (67.0 preparing: copies 40.6 ms 169.6 MB, textures 6.2 ms 23.7 MB; 61.9 on the GPU; 22.3 copying back 55.9 MB), 4651 draws/frame, dropped 348 without a host pipeline (0 being made) and 0 indirect; heap buffers 1.0 passes 92.5 ms;
2026-11-25T10:01:17.865842Z  INFO victory_gpu::host: 4.6 fps, 219.6 ms/frame (70.3 preparing: copies 40.8 ms 169.6 MB, textures 10.0 ms 25.1 MB; 55.4 on the GPU; 21.9 copying back 55.6 MB), 4656 draws/frame, dropped 350 without a host pipeline (0 being made) and 0 indirect; heap buffers 1.0 passes 111.5 ms;
2026-11-25T10:01:23.236057Z  INFO victory_gpu::host: 4.8 fps, 206.3 ms/frame (63.7 preparing: copies 41.4 ms 170.1 MB, textures 2.4 ms 9.8 MB; 55.6 on the GPU; 23.0 copying back 55.9 MB), 4660 draws/frame, dropped 344 without a host pipeline (0 being made) and 0 indirect; heap buffers 1.0 passes 89.8 ms;
2026-11-25T10:01:28.543613Z  INFO victory_gpu::host: 4.8 fps, 209.9 ms/frame (64.3 preparing: copies 41.3 ms 169.3 MB, textures 2.6 ms 11.4 MB; 59.2 on the GPU; 21.7 copying back 56.1 MB), 4659 draws/frame, dropped 345 without a host pipeline (0 being made) and 0 indirect; heap buffers 1.0 passes 90.6 ms;
2026-11-25T10:01:33.779574Z  INFO victory_gpu::host: 4.8 fps, 208.3 ms/frame (64.0 preparing: copies 41.6 ms 171.1 MB, textures 2.4 ms 9.5 MB; 57.9 on the GPU; 22.3 copying back 55.7 MB), 4650 draws/frame, dropped 334 without a host pipeline (0 being made) and 0 indirect; heap buffers 1.0 passes 91.5 ms;
2026-11-25T10:01:39.133564Z  INFO victory_gpu::host: 4.3 fps, 232.2 ms/frame (76.4 preparing: copies 40.1 ms 169.9 MB, textures 16.4 ms 27.3 MB; 75.7 on the GPU; 21.7 copying back 56.5 MB), 4659 draws/frame, dropped 339 without a host pipeline (0 being made) and 0 indirect; heap buffers 1.0 passes 94.2 ms;
2026-11-25T10:01:44.446131Z  INFO victory_gpu::host: 4.6 fps, 215.9 ms/frame (63.5 preparing: copies 40.9 ms 169.1 MB, textures 3.9 ms 26.6 MB; 63.2 on the GPU; 21.2 copying back 55.1 MB), 4652 draws/frame, dropped 334 without a host pipeline (0 being made) and 0 indirect; heap buffers 1.0 passes 110.6 ms;
- Shots: same cell window, player in full backlit silhouette. No purple; lines can't be judged. Gallery version 4: shots/tout5b-c-11min.jpg, shots/tout5b-c-6min.jpg.
- D's diag build started at 11:01:55.
```

## D : temps carte par commande (tout5b diag, GPU_TIMING + DRED)

```
Run D (diag tout5b bd85106 = 2C7CA5D6C10A40D3, GPU_TIMING + DRED, warm cache tout5b) is done. World at 11:04:51 (7 Enter presses), Alt+F4 120 s later at 11:06:52, exited at 11:07:00. No crash; peak 21.43 GB. No DRED output (nothing removed).
Card time by command, last block (11:06:49 local), raw, top 20:
the card's time by command (timing run), all 860 kinds together: 102.4 ms a frame; by first word: indirect 62.8 ms in 2439, dispatch 32.1 ms in 3677, mesh 3.7 ms in 294, alias 2.6 ms in 73, draw 0.7 ms in 50, indexed 0.2 ms in 12,
12.01 ms a frame in 1.0 commands a frame, the longest 12.98 ms: indirect [vertex e732a8b4bea4c0dd, pixel 456f02b52910aabc]
8.65 ms a frame in 1.0 commands a frame, the longest 8.86 ms: dispatch [compute 39f0e9aeb156a61c]
8.56 ms a frame in 200.0 commands a frame, the longest 0.30 ms: indirect [mesh ef5409053e14aa82, pixel 94d281151ca58b4f]
3.43 ms a frame in 1.0 commands a frame, the longest 3.52 ms: indirect [compute 919bafe017d59512]
3.24 ms a frame in 1.0 commands a frame, the longest 3.41 ms: indirect [compute bba90b2f0ff2eada]
2.62 ms a frame in 73.1 commands a frame, the longest 0.31 ms: alias copy
2.47 ms a frame in 151.0 commands a frame, the longest 0.31 ms: indirect [mesh b7a81f5b96f4ca93, pixel 43e24028d4a1faef]
1.83 ms a frame in 4.0 commands a frame, the longest 1.89 ms: mesh [mesh 04b9be3756e8f41c, pixel 1af4a50c7fb40c97]
1.70 ms a frame in 31.0 commands a frame, the longest 0.45 ms: indirect [mesh 5bd95fdec765c0d6, pixel 398971d1222d8f95]
1.30 ms a frame in 69.3 commands a frame, the longest 0.06 ms: indirect [mesh df4b2c7798c14c90, pixel d9fcb8a4425fc59b]
1.23 ms a frame in 1.0 commands a frame, the longest 1.32 ms: indirect [compute 940c2635dc1312f4]
1.21 ms a frame in 113.0 commands a frame, the longest 0.19 ms: indirect [mesh 30332cf87c4e0eae]
1.19 ms a frame in 1.0 commands a frame, the longest 1.22 ms: dispatch [compute fd46520032208bed]
1.13 ms a frame in 10.0 commands a frame, the longest 0.59 ms: indirect [mesh f841ad74ec6e7367, pixel 6695d7f5e3f6d351]
1.09 ms a frame in 1.0 commands a frame, the longest 1.16 ms: dispatch [compute 8cb27ccf2d50c606]
1.08 ms a frame in 1.0 commands a frame, the longest 1.16 ms: dispatch [compute 34fb51b8a0312e82]
1.06 ms a frame in 29.0 commands a frame, the longest 0.35 ms: indirect [mesh 1c451759513efb4c, pixel 43fba4644854c38e]
1.02 ms a frame in 17.0 commands a frame, the longest 0.20 ms: indirect [mesh 093c64546c55554e, pixel 818a9a0ec6a53abb]
0.94 ms a frame in 41.0 commands a frame, the longest 0.08 ms: indirect [mesh cf43bdbfc75c9296, pixel 573d1857e903fab5]
0.93 ms a frame in 53.0 commands a frame, the longest 0.27 ms: indirect [mesh 235e56869c1ea883, pixel 62f761fb777fc811]
0.90 ms a frame in 1.0 commands a frame, the longest 0.97 ms: indirect [compute 2dc364df5d3278b8]
Slow host list lines (only one in the run, 11:04:13), raw, first 7 of its commands:
2026-11-25T10:04:13.250701Z  WARN victory_gpu::host: slow host list ms=55 commands=8
… slow list command, 0.1 ms: dispatch 86x1x1 [compute 55679a0547649d0b] …
… slow list command, 0.0 ms: dispatch 37x1x1 [compute 91b10a73e48b2192] …
… slow list command, 0.0 ms: dispatch 9x1x1 [compute c1488f485ad606fb] …
… slow list command, 0.0 ms: dispatch 1x1x1 [compute 24317e785c8688e3] …
… slow list command, 0.0 ms: dispatch 123x1x1 [compute fdd56d97a806bcca] …
… slow list command, 0.0 ms: dispatch 213x1x1 [compute 17af15c4015cb4b6] …
Read: about 102 ms of card time per frame. Top items: one indirect fullscreen-like pass (VS e732a8b4bea4c0dd / PS 456f02b52910aabc) at 12 ms, the probes compute 39f0e9aeb156a61c at 8.7 ms, and a mesh with 200 draws at 8.6 ms.
G has been running since 11:07:00. H was switched to 6b5e9d9 with H1 (ALIAS_FLIGHT_COPIES=0, alias copies on) and H2 (both on), 3 min after world each. Order: G, H1, H2, then tout3q2 A/B, E/F, RT. Bats are written and CRLF-checked.
```

## G : Desktop tout3 + ALIAS_SETTLE=0

```
Run G (Desktop tout3 8006255 + VICTORY_ALIAS_SETTLE=0, warm cache tout3). The cache was WARM: library loaded 331,124 B, "all made made=0 loaded=34".
- World at 11:08:37. Alt+F4 300 s after world (11:13:46), exited at 11:13:52 ("window closed"). No AV, no device removed.
- fps: average 5.07 (includes the start), LAST 10 = 6.74 (tout3 B steady about 6.0).
- Last-10 averages vs tout3 B (B's steady window in brackets):
  - ms/frame 148.4 [≈167 / 175.5]
  - preparing 63.9: copies 36.3 ms / 122.1 MB, textures 6.3 / 19.6
  - on the GPU 69.5 [102.7]
  - copying back 88.6 / 61.4 MB [77.6 / 58.5]
  - heap passes 21.5 [31.3]
  - dropped 339.7, being made 0
- Settles (raw, last):
2026-11-25T10:13:47.352538Z  INFO victory_gpu::host: submissions sent ahead per frame: 29.4 in 2.9 settles (0.0 flights each time), 65.8 ms waiting for the card in them; settled for: … title memory changed under shader writes 0.1, title memory read 1.4 (by a buffer copy 0.4, a fill of upload memory 1.0, a texture 0.0) …
2026-11-25T10:13:47.352545Z  INFO victory_gpu::host: of the waiting for the card in settles, per frame, by what settled: … title memory changed under shader writes 4.7, title memory read 58.6, a step of ours 0.0, lists in the air 2.5 …
  So "title memory changed under shader writes" is 4.7 ms [B 73–95 ms] and total settle wait 65.8 ms [B ≈100]. Most of what's left is now "title memory read" (58.6 ms).
- Memory max 21.3 GB, last 19.9 GB.
- SHOTS (cell window, player from behind, gallery version 5: shots/tout3-g-6min.jpg, shots/tout3-g-4min.jpg):
  - Hair and skin are fine, lighting is fine, no purple.
  - The triangle-edge net is still on the sleeves, as expected on tout3.
  - POSSIBLE ARTIFACT: in the 6-min shot, a small solid BLACK HEXAGON shape on the shirt, lower centre (about x 740, y 810 of 1834×1032), not present in tout3 B/T17 at the same spot. In the 4-min shot that area is out of view.
  - On the right wall, an orange speckled patch (probably a lit wall texture).
  - No flicker can be judged from 2 stills.
- H build (6b5e9d9) started at 11:13:52, then H1/H2.
```

## H1/H2 : nuit/30fps-b 6b5e9d9

```
H (nuit/30fps-b 6b5e9d9) results:
- cargo test -p victory-gpu --release: 147 passed, 1 FAILED, 3 ignored. Failure: descriptor::host::tests::dynamic_allocation_finds_contiguous_range_crossing_search_cursor panicked at crates\victory-gpu\src\descriptor\host.rs:3616:9, "assertion left == right failed".
- Release build OK (victory-play-30fps = 4EF4D61B5D84BC5D), runs done anyway.
H1 (ALIAS_FLIGHT_COPIES=0, alias copies ON, cache tout5b): the CACHE MISSED. "Being made" was still about 1,040 at the end, so H1's numbers are a cold compile and not comparable. Either the cache key changed or alias-on changes pipelines.
- 1.93 fps, 533 ms per frame, heap passes 132.
- Settles: 27.6 ms; "title memory changed under shader writes" 8.1, "title memory read" 19.5.
- Catch-up: "texture catch-up per frame: 0.7 passes, 7.4 ms; textures picked by index uploaded per frame 42.0".
- No device removed; memory max 21.9 GB. Shots are a blank loading screen (not added).
H2 (both steps, warm after H1):
- Last 10: fps 5.03, ms/frame 199.4.
- Preparing 66.8: copies 42.0 ms / 166.9 MB, textures 3.9 / 19.3.
- On the GPU 25.6; copying back 59.3 / 59.7 MB; HEAP PASSES 94.8 (target ≤ 40 ✗); dropped 341.5, being made 0.
- Raw lines:
2026-11-25T10:25:44.386999Z  INFO victory_gpu::host: texture catch-up per frame: 1.2 passes, 6.4 ms; textures picked by index uploaded per frame 13.3
2026-11-25T10:25:44.386976Z  INFO victory_gpu::host: card gather per frame (VICTORY_CARD_GATHER=1): 750 pieces (48.4 MB) merged from what the card copied out within their submission, 345 fetched after all (344 past a plan's room of 160, 1 of pieces merged from a newer fetch since)
2026-11-25T10:25:26.836596Z  INFO victory_gpu::host: card copies from flights' copies into copies of the same title memory, in place of a settle (VICTORY_ALIAS_FLIGHT_COPIES=0 settles) copies=10014 mb=174
2026-11-25T10:25:44.386966Z  INFO victory_gpu::host: of the waiting for the card in settles, per frame, by what settled: … title memory changed under shader writes 1.3, title memory read 25.7 …
Settles 27.0 ms in 1.5 settles (tout3 B ≈100 ms).
- Memory max 23.1 GB (high), last 22.3. No device removed, no AV.
- Shots: no triangle net on the shirt, no purple, but two concerns:
  (a) the player's head shows NO HAIR, a bald black silhouette with only an ear outline. tout3 (G) at the same spot shows the lit ponytail. This matches the tout5b B/C shots, so it looks like a tout5b-line regression, not just backlight.
  (b) thin bright-red horizontal streaks across a yard table (about y 600–630).
- Gallery version 6: shots/30fps-h2-4min.jpg, shots/30fps-h2-3min.jpg.
Next: tout3q2 build, then A (cold) and B, then E/F, then RT.
```

## I1 : perf/30fps-pipeline 893a56b

```
I1 (893a56b = 6b5e9d9 + logs, H2 env, warm tout5b), 180 s after world, closed by Alt+F4 at about 12:15:45. No AV, no device removed; being made 0 at the end.
Last 10 averages:
- ms/frame 371.3, fps 3.24 (WORSE than H2's 199 ms / 5.03)
- preparing 107.7: copies 46.5 ms / 151.0 MB, textures 38.8 ms / 36.6 MB
- on the GPU 37.0
- copying back 47.0 / 44.7 MB
- heap passes 153.2
- dropped 342.4
- memory max 21.9 GB (the overlay showed 22.9 Go)
Heap split, raw (last 3):
2026-11-25T11:15:42.655677Z  INFO victory_gpu::host: heap passes per frame: 80.2 ms on buffer views, 2.2 ms on textures picked by index (11.0 uploaded); texture catch-up 1.0 passes, 9.1 ms
2026-11-25T11:15:43.662879Z  INFO victory_gpu::host: heap passes per frame: 99.5 ms on buffer views, 5.1 ms on textures picked by index (15.4 uploaded); texture catch-up 1.2 passes, 10.8 ms
2026-11-25T11:15:44.791738Z  INFO victory_gpu::host: heap passes per frame: 111.7 ms on buffer views, 29.7 ms on textures picked by index (46.0 uploaded); texture catch-up 1.5 passes, 33.0 ms
Buffer views dominate (80–112 ms), so I2 (UNWATCHED_FRAMES=4) is running, started at 12:15:53.
Fills, raw (last 3):
2026-11-25T11:15:42.655632Z  INFO victory_gpu::host: fills of upload memory that met flights' writes per frame, by what asked: root constants (not aligned) 0.0, constants in tables 1.0, texture copies from buffers 0.0
2026-11-25T11:15:43.662834Z  INFO victory_gpu::host: fills of upload memory that met flights' writes per frame, by what asked: root constants (not aligned) 0.0, constants in tables 1.0, texture copies from buffers 0.0
2026-11-25T11:15:44.791692Z  INFO victory_gpu::host: fills of upload memory that met flights' writes per frame, by what asked: root constants (not aligned) 0.0, constants in tables 0.5, texture copies from buffers 0.0
Settles: 32.5 in 2.0 settles, 45.3 ms waiting. "title memory changed under shader writes" 0.0, "title memory read" 45.3.
Shots (gallery version 9: shots/30fps-i1-4min.jpg, shots/30fps-i1-3min.jpg):
- The save now starts elsewhere: a dark corridor with the hint "Approchez-vous d'une codétenue…" (the user played earlier, so the start position moved).
- Player: bald, almost black silhouette, the same look the user screenshotted. That's the tout5b-line character regression, not the Desktop build.
Next: I3 (CARD_ROOM 80), then the K builds and K run, then J1–J3, then RT.
```

## I2 : I1 + UNWATCHED_FRAMES=4

```
I2 (I1 + VICTORY_UNWATCHED_FRAMES=4), 180 s after world, exited 12:20:32 ("window closed"). No AV, no device removed.
Last 10 averages:
- ms/frame 211.7, fps 4.76 (I1: 371 / 3.24; H2: 199 / 5.03)
- preparing 72.5: copies 42.2 / 168.8 MB, textures 8.9 / 21.3
- on the GPU 26.8
- copying back 56.5 / 58.5 MB
- heap passes 109.8
- dropped 341, being made 0
- memory max 22.1 GB
Heap split, raw (last 3):
2026-11-25T11:20:22.861355Z  INFO victory_gpu::host: heap passes per frame: 114.4 ms on buffer views, 1.2 ms on textures picked by index (11.8 uploaded); texture catch-up 1.0 passes, 6.5 ms
2026-11-25T11:20:23.867297Z  INFO victory_gpu::host: heap passes per frame: 88.0 ms on buffer views, 2.2 ms on textures picked by index (10.4 uploaded); texture catch-up 1.2 passes, 9.8 ms
2026-11-25T11:20:24.958989Z  INFO victory_gpu::host: heap passes per frame: 98.5 ms on buffer views, 3.2 ms on textures picked by index (17.2 uploaded); texture catch-up 0.8 passes, 6.3 ms
Settles: 30.4 in 1.6 settles, 35.2 ms waiting.
UNWATCHED_FRAMES=4 did NOT cut the buffer-view time (88–114 ms vs I1's 80–112). I1's slower numbers came from texture catch-up and upload in that window (textures 38.8 ms), not buffer views.
I3 (CARD_ROOM_PERCENT=80) is next, then the K builds/run, J1–J3, RT.
```

## K : perf/30fps-tout3 1e91b32

```
K report (1e91b32, warm tout3q2, Desktop env). Launched 12:28:21, world 12:29:20, Alt+F4 at +221 s, clean exit. Device removed 0, amdxc 0. This run finished before J1 launched (12:32:29), so it was not affected.
- Last 10 fps lines: 4.79 fps, 224.0 ms/frame. tout3q2 B was 5.79 / 173.
  - The first 5 of those 10 have texture spikes: 3.0–4.6 fps, textures 53–185 ms.
  - The last 5 are steady at 5.7–6.0 fps, 167–177 ms.
- Averages: preparing 110.0 (copies 39.5 ms / 120.4 MB, textures 42.8 ms / 39.2 MB), GPU 62.0, copying back 77.8 ms / 51.1 MB, heap passes 67.1 ms, dropped 348, being made 0.
- Memory max 21,175 MB.
- fills raw, last 3, all identical: "fills of upload memory that met flights' writes per frame, by what asked: root constants (not aligned) 0.0, constants in tables 1.0, texture copies from buffers 0.0" (11:32:21 / :22 / :23).
- flights raw: "card copies from flights' copies into copies of the same title memory, in place of a settle (VICTORY_ALIAS_FLIGHT_COPIES=0 settles) copies=10018 mb=404" (11:32:12; the earlier line was copies=3 mb=0 at 11:29:17).
- Settles: "submissions sent ahead per frame: 30.9 in 2.6 settles (0.0 flights each time), 78.7 ms waiting for the card in them". By what settled: title memory read 67.8 ms, the rest 0.0.
- Relay: 80.5 ms waiting with nothing to land, 47.2 ms for fetches of card copies' pieces.
- Card gather (last): 684 pieces (44.2 MB) merged, 224 fetched after all (223 past a plan's room of 160).
- Visual (3-min shot, gallery v11 shots/30fps-k-3min.jpg; the run stopped before the 4-min shot): hair/ponytail present, face profile fine, shirt seam lines normal, no line web.
  - NEW: several floating black rectangles at the top right, over the bright exterior (about 5 opaque dark quads).
  - I have not seen these on tout3q2 B in the same scene. Possible regression from the flight-copy or alias change.
Not promoted.
```

## J1 : K diag, GPU_TIMING

```
J1 report (K diag 1e91b32, warm tout3q2, GPU_TIMING=1 + DRED; the only game running).
- Timeline: launched 12:46:04, world 12:47:35, stopped at world + 120 s, clean exit 12:49:44.
- Stability: device removed 0, AV 0.
- Performance (timing run, so slow):
  - Last 10 fps lines: 2.23 fps, 492.4 ms/frame.
  - GPU 185.4 ms; copies 41.5 ms / 160.7 MB; textures 72.6 ms / 69.5 MB; copying back 19.8 ms / 67.7 MB.
  - Heap passes 220.2 ms, dropped 801.
- Memory max 21,319 MB.
- Card time by kind, raw: "all 1403 kinds together: 128.0 ms a frame; by first word: indirect 80.1 ms in 6102, dispatch 29.8 ms in 3586, mesh 9.4 ms in 984, draw 4.9 ms in 438, alias 2.1 ms in 78, texture 0.9 ms in 371, indexed 0.4 ms in 68, copy 0.3 ms in 7, read 0.0 ms in 2"
- Top 10 commands, raw (ms a frame / commands a frame / longest):
  8.67 / 1.0 / 9.14 indirect [vertex e732a8b4bea4c0dd, pixel 456f02b52910aabc]
  8.33 / 1.0 / 9.55 dispatch [compute 39f0e9aeb156a61c]
  6.81 / 885.2 / 0.25 indirect [mesh 9f1f95875944d82a, pixel f710ed8f1038035e]
  4.84 / 192.1 / 0.18 indirect [mesh ef5409053e14aa82, pixel 94d281151ca58b4f]
  4.76 / 432.3 / 0.32 indirect [mesh db875f50a166bd91]
  4.70 / 180.4 / 0.40 mesh [mesh f0c7efd2aceb2aaa, pixel 7d85c9cab3c80721]
  4.03 / 1.0 / 6.46 indirect [compute 7a071f906f98311b]
  2.15 / 1.0 / 3.84 dispatch [compute 5cf00dc39042edaf]
  2.11 / 78.3 / 0.41 alias copy
  2.07 / 1.0 / 2.37 draw [vertex 7affbd5bbaa830f3, pixel 6318f669e4f17d65]
- Shot (gallery v13, shots/30fps-j1-3min.jpg): female ponytail, lit hair, the shirt red with glossy streaks (same red as I4b, so the red shirt itself is normal). No black rectangles visible in this view.
Queue: J2 was NOT launched. chain21 printed "##### J2" in the same second as J1's "exited" and was stopped before it launched anything. The cold opt30k cache is untouched. chain23 is running alone: Desk launched 12:50:06 → J2 → J3 → RT.
```
