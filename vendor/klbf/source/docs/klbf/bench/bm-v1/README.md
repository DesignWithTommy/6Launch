# BM-v1 results

Accepted results for [Bayside Morning v1](../../BAYSIDE-MORNING-V1.md): for each run, its
`KLBF-Bench-BM-v1-<serial>.json`, unchanged, and an entry in `submissions.json`:
`{"<serial>": {"login": "<submitter's GitHub login>", "card": "<card PNG URL attached to the issue>", "repo": "<project repo URL from the form>"}}`.
Add `"reference": true` for our own run on the KLBF Lab test machine.
The player on the card must be that login or `anonymous`. After adding one, run
`python -I tools/bench-board.py` to regenerate the board and the README's latest-run line.

<!-- Seed: our own run goes here once it exists. -->
