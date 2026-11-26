#!/usr/bin/env bash
# The update loop's step: each vendor/<name> pinned to a GitHub repo moves to that
# repo's newest default-branch commit (UPSTREAM and catalog.txt). The settings in
# each NEONBRIDGE.toml are re-checked against the author's scripts by hand.
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
  printf '%s\n' "$new" > "$d/UPSTREAM"
  sed -i "s/^=== $n $old\$/=== $n $new/" catalog.txt
  changed+=("$n ${new:0:7}")
done
printf '%s\n' "${changed[@]:-}" | sed '/^$/d' > /tmp/bumped.txt
cat /tmp/bumped.txt
