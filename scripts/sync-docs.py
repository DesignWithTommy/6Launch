"""Keeps docs/CREDITS.md and the website (index.html) in step with vendor/ and STANDINGS.md.

Run by scripts/bump-upstreams.sh. Credits: adds any vendored runtime whose upstream
is not yet credited. Website: rewrites the standings from STANDINGS.md.
"""
import html, pathlib, re

root = pathlib.Path(__file__).resolve().parent.parent


def licence(d):
    p = d / "LICENSE"
    t = p.read_text(encoding="utf-8", errors="replace")[:3000] if p.exists() else ""
    if "GNU GENERAL PUBLIC LICENSE" in t.upper():
        v = "GPL-3.0" if "Version 3" in t else "GPL-2.0" if "Version 2" in t else "GPL"
        return v + ("-or-later" if "or (at your option) any later version" in t else "")
    for k, v in (("MIT License", "MIT"), ("Apache License", "Apache-2.0")):
        if k in t:
            return v
    return "its own licence (see vendor LICENSE)"


cr = root / "docs/CREDITS.md"
c = cr.read_text(encoding="utf-8")
c = c.replace("An hourly update moves", "A regular update (every 15 minutes) moves")
c = re.sub(r"Ranks come from \[vuyp\].*?\(src/build\.py rank_key\)\.",
           "Ranks are i95's own, with [PortWatch VI](https://portwatch-vi.xyz/)'s scoring formula (scripts/rank.py); "
           "thanks to [vuyp](https://github.com/vuyp)'s [GTA VI PC race tracker](https://vuyp.github.io/gta6-pc-race/) for the leads.", c)
add = []
for d in sorted((root / "vendor").iterdir()):
    t = d / "NEONBRIDGE.toml"
    if not t.exists():
        continue
    m = re.search(r'^upstream = "https://github\.com/([^/"]+)/([^/"]+)', t.read_text(encoding="utf-8"), re.M)
    if not m or m.group(1) == "DesignWithTommy":
        continue
    owner, repo = m.groups()
    if f"github.com/{owner}/{repo}".lower() in c.lower():
        continue
    add.append(f"- [{repo}](https://github.com/{owner}/{repo}) by {owner}, {licence(d)} (vendor/{d.name})")
if add:
    c = c.rstrip("\n") + "\n" + "\n".join(add) + "\n"
cr.write_text(c, encoding="utf-8", newline="\n")

ix = root / "docs/index.html"
h = ix.read_text(encoding="utf-8")


def md(t):
    t = html.escape(t.strip(), quote=False)
    t = re.sub(r"\[([^\]]+)\]\((https?://[^)\s]+)\)", r'<a href="\2" rel="noopener">\1</a>', t)
    t = re.sub(r"\*\*(.+?)\*\*", r"<b>\1</b>", t)
    t = re.sub(r"`([^`]+)`", r"<code>\1</code>", t)
    return t


def cells(l):
    return [c.strip() for c in l.strip().strip("|").split("|")]


lines = (root / "STANDINGS.md").read_text(encoding="utf-8").split("\n")
full = {}
for l in lines:
    m = re.match(r"^\| \[[^\]]+\]\((https://github\.com/[^)]+?)/?\) \|", l)
    if m and not re.match(r"^\| \d+ \|", l):
        c = cells(l)
        if len(c) >= 4:
            full[m.group(1).lower()] = (c[1], c[2], c[3])
rows = []
for l in lines:
    if not re.match(r"^\| \d+ \| [\d.]+ \| \[", l):
        continue
    c = cells(l)
    if len(c) < 6:
        continue
    r, sc, rt, gpu, rep, ours = c[:6]
    m = re.match(r"\[([^\]]+)\]\((https://github\.com/[^)]+?)/?\)", rt)
    if not m:
        continue
    name, url = m.groups()
    fork, long, ours_long = full.get(url.lower(), ("", rep, ours))
    gk = "nvidia" if "nvidia" in gpu.lower() else "amd" if "amd" in gpu.lower() else "any"
    tested = "" if ours.strip() in ("—", "-", "") else " tested"
    detail = f'<p>{md(long)}</p>' + (f'<p class="ours"><b>On our RTX 4060:</b> {md(ours_long)}</p>' if tested else "") + (f'<p class="fork">Based on {md(fork)}</p>' if fork else "")
    rows.append(
        f'<details class="rt{tested}" data-gpu="{gk}" data-name="{html.escape(name.lower())}">'
        f'<summary><span class="rank">{r}</span><span class="name">{html.escape(name)}</span>'
        f'<span class="gpu">{md(gpu)}</span><span class="score">{sc}</span>'
        f'<span class="rep">{md(rep)}</span></summary>'
        f'<div class="more">{detail}<a class="src" href="{url}" rel="noopener">Source on GitHub &rarr;</a></div></details>')
block = "\n".join(rows)
h = re.sub(r"(<!--standings:start-->).*?(<!--standings:end-->)", lambda m: m.group(1) + "\n" + block + "\n" + m.group(2), h, count=1, flags=re.S)
h = re.sub(r'(<span id="count">)\d*(</span>)', lambda m: f"{m.group(1)}{len(rows)}{m.group(2)}", h)
ix.write_text(h, encoding="utf-8", newline="\n")
print(f"sync-docs: {len(add)} credits added, {len(rows)} runtimes on the site")
