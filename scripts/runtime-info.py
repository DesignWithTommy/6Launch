"""Writes `updated = "<date>"` (the date of the upstream commit in vendor/<name>/UPSTREAM)
and `stars = <n>` (the repo's GitHub stars) into each vendor/<name>/NEONBRIDGE.toml, under
its `upstream =` line, so the catalog carries them to the launcher's details pane.
Run after bump-upstreams.sh, before i95 --catalog: python -I scripts/runtime-info.py (needs gh)."""
import json, os, re, subprocess
from datetime import datetime

root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
cache_file = os.path.join(root, "target", "runtime-info.json")
try:
    cache = json.load(open(cache_file, encoding="utf-8"))
except Exception:
    cache = {}


def gh(path, jq):
    out = subprocess.run(["gh", "api", path, "--jq", jq], capture_output=True, text=True)
    return out.stdout.strip() if out.returncode == 0 else None


changed = 0
for name in sorted(os.listdir(os.path.join(root, "vendor"))):
    d = os.path.join(root, "vendor", name)
    try:
        toml = open(os.path.join(d, "NEONBRIDGE.toml"), encoding="utf-8").read()
        sha = open(os.path.join(d, "UPSTREAM"), encoding="utf-8").read().split()[0]
    except Exception:
        continue
    m = re.search(r'^upstream = "https://github\.com/([^"/]+/[^"/]+?)(?:\.git)?"[^\n]*\n', toml, re.M)
    if not m:
        continue
    repo = m.group(1)
    key = f"{repo}@{sha}@time"
    if key not in cache:
        date = gh(f"repos/{repo}/commits/{sha}", ".commit.committer.date")
        if date:
            # Local time of the commit, to the minute.
            cache[key] = datetime.fromisoformat(date.replace("Z", "+00:00")).astimezone().strftime("%Y-%m-%d %H:%M")
    date = cache.get(key)
    stars = gh(f"repos/{repo}", ".stargazers_count")
    new = toml
    # A value gh could not give (offline, rate limit) keeps its old line.
    keep = {k: line for line, k in re.findall(r'^((updated|stars) = [^\n]*\n)', new, re.M)}
    new = re.sub(r'^(updated|stars) = [^\n]*\n', "", new, flags=re.M)
    m = re.search(r'^upstream = [^\n]*\n', new, re.M)
    add = ""
    if not date and "updated" in keep:
        add += keep["updated"]
    elif date:
        add += f'updated = "{date}"   # the pinned upstream commit\'s date (scripts/runtime-info.py)\n'
    if stars and stars.isdigit():
        add += f"stars = {stars}   # GitHub stars when last checked\n"
    elif "stars" in keep:
        add += keep["stars"]
    new = new[: m.end()] + add + new[m.end():]
    if new != toml:
        open(os.path.join(d, "NEONBRIDGE.toml"), "w", encoding="utf-8", newline="\n").write(new)
        changed += 1

os.makedirs(os.path.dirname(cache_file), exist_ok=True)
json.dump(cache, open(cache_file, "w", encoding="utf-8"))
print(f"runtime info: {changed} tomls changed")
