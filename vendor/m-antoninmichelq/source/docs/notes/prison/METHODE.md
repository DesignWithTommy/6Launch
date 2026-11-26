# Prison scene, end to end, offline first (user request 2026-11-24 21:51 Paris)

Goal: make the prison scene fully work (buildings, textures, lighting, speed) before widening.
Launch the game only when a build shows something new. Everything else is offline on the PC.

## Phase 1 (now): the scene's whole shader set, translated offline
- Build: fork branch victory-stack-d13z6p 3193172 (= lumière 2 candidate 28e9be6 + 34487b2 dump + dump-all).
- One capture run (user OK once), diag exe, same env as Jouer plus:
  VICTORY_SHADER_DUMP=C:\Users\4nto\Desktop\VIctory\_work\prison\shaders
  VICTORY_SHADER_HASHES=all            (every shader the title creates -> <hash>.bin + index.txt "<hash> <stage>")
  VICTORY_CAPTURE=C:\Users\4nto\Desktop\VIctory\_work\prison
  VICTORY_CAPTURE_AFTER_WORLD=360,420  (prison; leave the window open ~10 min)
  VICTORY_DUMP_TARGETS=1, VICTORY_DUMP_FULL=1, VICTORY_INDIRECT_STATS=1
- Offline, no game (PC, any time, no OK needed):
  victory translate _work\prison\shaders --validate --pipelines --host-lanes 32-64 --list > _work\prison\translate-<commit>.txt
  = translator + DXIL validator + real driver pipeline creation on the RX 7600, per shader, with failure reasons.
  A translator fix is checked by rebuilding victory.exe and rerunning this line, then diffing the reports.
- Files stay local on the PC (never published). The cloud threads get the reports (text) and, when needed, staged .bin files.

## Phase 2 (next): per-draw frame trace + single-draw replay
- Frame trace (to write): at one captured frame, per draw/dispatch in order: pipeline hashes per stage, root view addresses+sizes,
  bound targets, indirect args. Output _work\prison\frame-<n>.jsonl.
- Replay (to write): re-run one draw/dispatch from the trace against captured buffer ranges (dumped once with the trace),
  write the output target to PNG, so a translator fix can be checked against the scene without the game.
- Owners: materials/lighting thread (draw.rs knowledge) for trace+replay format; Sync thread runs the capture and the PC side.

## Threads using it
- Buildings: cmsg_01XYoPEzLL1bj63HP9pniGvsXNyNECxZ8q3AHi1qmrMLh6
- Lighting/textures: cmsg_01XYoPEzLL1bj63HP9pniGvsEUsQ1QkLneSZ7oCrk82Bmg
- Characters: cmsg_01XYoPEzLL1bj63HP9pniGvsWiRCxc4ARkbVZv3irzfS15
