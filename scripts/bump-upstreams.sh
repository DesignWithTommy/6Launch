#!/usr/bin/env bash
# The update loop (run by hand or by Claude, no bot): each vendor/<name> pinned to a GitHub repo moves to that
# repo's newest default-branch commit (UPSTREAM and catalog.txt);
# every runtime moves, launch-file changes included; nothing is held.
set -u
changed=()
for d in vendor/*/; do
  n=$(basename "$d"); t="$d/NEONBRIDGE.toml"
  [ -f "$t" ] && [ -f "$d/UPSTREAM" ] || continue
  repo=$(sed -n 's#^upstream *= *"https://github.com/\([^/"]*/[^/"]*\)/*".*#\1#p' "$t" | head -1); repo=${repo%.git}   # trailing / or .git dropped, as fetch.rs repo() does
  [ -n "$repo" ] || continue
  new=$(gh api "repos/$repo/commits/HEAD" -q .sha 2>/dev/null) || { echo "$n: $repo unreachable"; continue; }
  old=$(head -1 "$d/UPSTREAM" | tr -d '[:space:]')
  [ ${#new} -eq 40 ] && [ "$new" != "$old" ] || continue
  printf '%s\n' "$new" > "$d/UPSTREAM"
  sed -i "/^about/s/${old:0:7}/${new:0:7}/" "$t"
  changed+=("$n ${new:0:7}")
done
# catalog.txt rebuilt from the tomls and pins (same layout as i95 --catalog).
# A failed rebuild fails the script: the UPSTREAM files above would be out of step with catalog.txt.
python - <<'P' || { echo "catalog.txt rebuild failed; UPSTREAM files changed but catalog.txt was not rewritten" >&2; exit 1; }
import pathlib
out = ["# i95 runtime catalog: `=== <name> <upstream commit>` then that runtime's NEONBRIDGE.toml. Made by i95 --catalog."]
for d in sorted(pathlib.Path("vendor").iterdir()):
    t, u = d / "NEONBRIDGE.toml", d / "UPSTREAM"
    if not (t.is_file() and u.is_file()):
        continue
    lines = u.read_text().splitlines()
    c = lines[0].strip() if lines else ""
    if len(c) != 40 or any(ch not in "0123456789abcdefABCDEF" for ch in c):
        continue
    out += [f"=== {d.name} {c}", t.read_text(encoding="utf-8").replace("\r\n", "\n").rstrip()]
pathlib.Path("catalog.txt").write_text("\n".join(out) + "\n", encoding="utf-8", newline="\n")
P
printf '%s\n' "${changed[@]:-}" | sed '/^$/d' > "${TMPDIR:-/tmp}/bumped.txt"
T=$(mktemp -d); if gh api repos/4lwayz/portwatch-vi/contents/data/tracker.json -H "Accept: application/vnd.github.raw" > "$T/pw.json" 2>/dev/null; then python -I scripts/rank.py "$T/pw.json" "$T/ranks.json" > /dev/null && python -I scripts/rank-write.py "$T/ranks.json" >&2; fi; rm -rf "$T"
python -I scripts/sync-docs.py >&2
# Flag runtimes the README table does not link (matched on the toml's upstream URL, case-insensitive).
for d in vendor/*/; do
  u=$(grep -m1 '^upstream' "$d/NEONBRIDGE.toml" 2>/dev/null | grep -o 'https://github.com/[^"]*'); u=${u%/}; u=${u%.git}
  [ -n "$u" ] && ! grep -qiF "($u)" README.md && echo "README missing: $(basename "$d") $u" >&2
done
cat "${TMPDIR:-/tmp}/bumped.txt"
