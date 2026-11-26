#!/usr/bin/env bash
# The update loop (run by hand or by Claude, no bot): each vendor/<name> pinned to a GitHub repo moves to that
# repo's newest default-branch commit (UPSTREAM and catalog.txt). The settings in
# each NEONBRIDGE.toml are re-checked by hand: a commit touching launch files is held.
set -u
changed=()
for d in vendor/*/; do
  n=$(basename "$d"); t="$d/NEONBRIDGE.toml"
  [ -f "$t" ] && [ -f "$d/UPSTREAM" ] || continue
  repo=$(sed -n 's#^upstream *= *"https://github.com/\([^/"]*/[^/"]*\)".*#\1#p' "$t" | head -1)
  [ -n "$repo" ] || continue
  new=$(gh api "repos/$repo/commits/HEAD" -q .sha 2>/dev/null) || { echo "$n: $repo unreachable"; continue; }
  old=$(head -1 "$d/UPSTREAM" | tr -d '[:space:]')
  [ ${#new} -eq 40 ] && [ "$new" != "$old" ] || continue
  # A commit touching the author's launch scripts, settings or README may change what
  # the toml should say: held for a hand check (listed in the run summary).
  if gh api "repos/$repo/compare/$old...$new" -q '.files[].filename' 2>/dev/null | grep -qiE '\.(ps1|cmd|bat|toml|json)$|readme|settings\.rs|apps/[^/]+/src/main\.rs'; then
    echo "$n: held for review (${new:0:7} changes launch files)"
    echo "- $n $repo ${old:0:7}..${new:0:7}" >> "${GITHUB_STEP_SUMMARY:-/dev/null}"
    continue
  fi
  printf '%s\n' "$new" > "$d/UPSTREAM"
  sed -i "/^about/s/${old:0:7}/${new:0:7}/" "$t"
  changed+=("$n ${new:0:7}")
done
# catalog.txt rebuilt from the tomls and pins (same layout as i95 --catalog).
python - <<'P'
import pathlib
out = ["# i95 runtime catalog: `=== <name> <upstream commit>` then that runtime's NEONBRIDGE.toml. Made by i95 --catalog."]
for d in sorted(pathlib.Path("vendor").iterdir()):
    t, u = d / "NEONBRIDGE.toml", d / "UPSTREAM"
    if not (t.is_file() and u.is_file()):
        continue
    c = u.read_text().splitlines()[0].strip()
    if len(c) != 40:
        continue
    out += [f"=== {d.name} {c}", t.read_text(encoding="utf-8").replace("\r\n", "\n").rstrip()]
pathlib.Path("catalog.txt").write_text("\n".join(out) + "\n", encoding="utf-8", newline="\n")
P
printf '%s\n' "${changed[@]:-}" | sed '/^$/d' > "${TMPDIR:-/tmp}/bumped.txt"
cat "${TMPDIR:-/tmp}/bumped.txt"
