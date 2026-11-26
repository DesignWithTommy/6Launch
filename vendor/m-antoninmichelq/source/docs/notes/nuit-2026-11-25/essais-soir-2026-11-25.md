# Evening trials, 2026-11-25 (HANDOFF section 7)

**Desktop Jouer is unchanged.** It is still Peau3 b74ac64 (_work\victory-play-peau3.exe). Nothing was promoted tonight.
- Env: STAGING_POOL_MB=1024, VRAM_HYSTERESIS=85,75, CARD_BUFFERS_MB=512, TEXTURE_HALF=2, AMBIENT=off.
- Cache: tout3q2.

All runs below are warm, with Peau3 env, and the fps figures are over the last 10 lines.

## Results
- **Peau3 control** (run-20261125-195202-jouer-ctl): 6.50 fps, GPU 70.5 ms.
  - The chrome shirt and the missing window glass vary from run to run, even on Peau3.
- **Lean math** (perf/lean-math 2c143b9):
  - L7: 6.68 fps (+4%), GPU 70.7 ms.
  - L31: 6.93 fps (+8%), GPU 61.6 ms. It showed a chrome shirt, but Peau3 shows that too.
  - Every new LEAN value needs a cold run of about 1,800 pipelines (10+ min).
  - 4237bce fixes the -0 from bit 1.
  - LEAN 97 was never run alone.
- **Grouped trials** (fork branches, all with LEAN=127 and FLAT_INSTANCES=e732a8b4):
  - Branches:
    - essai/groupe-base = 9beab0a: lean 4237bce + ledger 7068acc + flat tiles 142aa83e.
    - essai/groupe = f5a52e9: groupe-base + loading-sharp 2cde495 + overlay aeebeec.
    - essai/groupe-overlay = 0bdc220: groupe + the overlay height fix bbc847a.
  - Speed: GPU 53-57 ms instead of 70, about 7 fps when steady.
  - Image: NPCs show chrome patches and black silhouettes, and a guard's head floats with no body in the common room. **Not promotable.**
  - DISPATCH_PEEK group counts:
    - 39f0e9ae: 2040×1×1 groups (×32 threads).
    - 5cf00dc3: 240×135×1 groups.
- **J-sans-sondes** (Peau3 + NO_COMPILE=39f0e9ae): the chrome stays and gets even brighter, so the probes are NOT the cause.
- **Textures given up at w+300:**

  | Run | Count |
  |---|---|
  | Peau3, window scene | 16.4k |
  | Common room | about 33k |
  | wait-grey | 65.6k |

  The chrome and black areas follow this count.
- **fix/wait-grey 89e0bb6** (built on fix/evict-grey 056a9b5, Peau3 env, run-20261125-204839-jouer-evictgrey):
  - Fixed: the shirt is matte at the window.
  - Still wrong:
    - posters are solid black;
    - the face and neck are black in the A102 corridor;
    - NPCs in the cells are still chrome.
  - Other numbers:
    - The given-up count doubled.
    - The log has no "grey" line, so it can't confirm the stand-in was used.
    - 4.90 fps, in a heavy scene.
    - 21.8 GB peak.
  - **Not clean, not promoted.**

## To do (NOT run tonight)
- Split the grouped trial (all on essai/groupe-overlay):
  - A: LEAN unset.
  - B: LEAN=127 + HOIST_CLAMPS=7.
  - C: LEAN=97.
- D: a diagnostic run with GPU_TIMING + DRED on the best mix, for the e732 time and a new top 20.
- Textures, the real fix: evict less, either least-recently-sampled first or by keeping the small mips. Also find the views that stay null.
- Ledger 7068acc: a 10-minute torso check.
- fx d510ee4 trials F0-F2 (the user has not OK'd them).
