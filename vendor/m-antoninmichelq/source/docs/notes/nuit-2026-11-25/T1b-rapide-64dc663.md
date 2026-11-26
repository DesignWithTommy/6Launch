# T1b "Jouer rapide (essai)" 64dc663 + WIDE_TEXTURES=1 + CARD_GATHER=1 : PASSED (03:03-03:15 Paris)
Log on the PC: _work\runs\run-20261125-030335-jouer-rapide.log (timestamps in the log are UTC)
- World ~128 s after the first log line (5 Enter presses: profile, Continuer)
- Private memory max 21.18 GB. memory: lines 8.4 GB -> 18.3 GB (world load) -> peak 21.7 GB at 02:06:37 -> flat 21.0-21.08 GB until 02:15:39. PC 5.6-6.6 GB free, ~15.4 GB commit left, card ~6490/6884 MB. No climb.
- memory, ours: host copies 4237 alive / 3511 MB -> peak 4820 / 5264 MB -> flat ~4542 / 4404 MB (windows outside buffers 846 / 3484 MB); host textures 13175 / 1863 MB peak -> ~6880 / 1516 MB; lists ~765; scratch 212 chunks
- fps after world avg 5.51 (min 0.9, max 6.3, last 10 avg 6.12); settled 5.5-6.1 fps, 158-184 ms/frame, ~4540-4930 draws, ~440 dropped/frame
- being made: 34 just before world, then 0. "host pipelines all made made=0 loaded=2738 started_early=2740 waited_for_memory=2 tried_again=0"
- textures waiting: 20885 at world -> 14618 (+36 s) -> 9629 -> 5611 (+71 s) -> plateau ~2650 (never below 200); queue: ~2650 slots, 38-75 due, 18 waiting for memory, ~1700 uploaded in part; periodic re-due bursts of 1710-1732 (oldest written 3109-4181 frames ago)
- 39f0e9ae refused up front (VICTORY_NO_COMPILE), first bind frame 3283; no RAM climb
- card gather: 407-411 pieces (25.9-26.2 MB) merged/frame from in-submission copies, 508-527 still fetched after
- refused: only mesh f5e50eb2 + pixel ce3f0432 (0x80070057). No OOM, no short-of-memory, no tried-again
- WARN: ~100 alias WARNs; 0x1c8/0x218 unimplemented 16+13; 4 depth bounds; misc singles
- End: clean Alt+F4
