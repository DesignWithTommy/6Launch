#!/usr/bin/env bash
# The update loop's step: each vendor/<name> pinned to a GitHub repo moves to that
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
  sed -i "s/^=== $n $old\$/=== $n $new/" catalog.txt
  changed+=("$n ${new:0:7}")
done
printf '%s\n' "${changed[@]:-}" | sed '/^$/d' > /tmp/bumped.txt
cat /tmp/bumped.txt
