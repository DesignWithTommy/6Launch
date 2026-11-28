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
# Line and strategy tags per runtime (analysis.tsv, from ClaudeTheftAuto's docs/RUNTIMES.md).
ana = {}
ap = root / "analysis.tsv"
if ap.exists():
    for l in ap.read_text(encoding="utf-8").split("\n"):
        if l and not l.startswith("#"):
            p = (l.split("\t") + ["", ""])[:3]
            ana[p[0].strip().lower()] = (p[1].strip(), p[2].split())
TAGS = {
    "LOOP": ("Faster copy loop", "Cheaper per-frame syncing of the CPU shadow to the GPU: chunk hashes, dirty spans, fewer compares."),
    "INPLACE": ("Zero-copy", "The GPU reads the game's memory directly instead of a copy. The biggest gains, paid for in page faults and PCIe reads."),
    "CARDCMP": ("Compare on the GPU", "Changed data is found on the graphics card instead of the CPU. The best single change on record: 11 to 21.8 fps."),
    "THREADS": ("More threads", "Translation and submission split across threads. Hides CPU latency without cutting the work."),
    "GPUCUT": ("Less GPU work", "Series S mode, lower render scale, DLSS or FSR, frame generation, ray tracing off. Helps once the copy loop is fixed."),
    "VRAMFIT": ("Fits small cards", "Mip caps, tiled levels and eviction to fit 8 GB of VRAM or less."),
    "RAMFIT": ("Fits 16 GB RAM", "Trimmed and capped memory shadows; most runtimes still use 23 to 30 GB and page on 16 GB."),
    "PICFIX": ("Picture fixes", "Lighting, UI, NaN and ray-miss fixes: correctness, not speed."),
    "SLOTS": ("Descriptor slots", "Bigger dynamic descriptor heaps, so NVIDIA cards stop silently dropping draws."),
    "OWN": ("Own engine", "Not a VIctory fork at heart: its own layer or engine."),
    "NOMESH": ("No mesh shaders", "Runs on cards without mesh shaders, such as the RX 580."),
}
rows = []
stats = {}
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
    key = url.lower().removeprefix("https://github.com/")
    line, tags = ana.get(key, ("", []))
    for t in tags:
        st = stats.setdefault(t, [0, None, None])
        st[0] += 1
        if st[1] is None:
            st[1], st[2] = name, sc
    chips = "".join(f'<span class="tag" title="{html.escape(TAGS[t][1])}">{html.escape(TAGS[t][0])}</span>' for t in tags if t in TAGS)
    lineage = f'<p class="fork">Line: {html.escape(line)}</p>' if line else ""
    detail = (f'<p class="tags">{chips}</p>' if chips else "") + lineage + f'<p>{md(long)}</p>' + (f'<p class="ours"><b>On our RTX 4060:</b> {md(ours_long)}</p>' if tested else "") + (f'<p class="fork">Based on {md(fork)}</p>' if fork else "")
    rows.append(
        f'<details class="rt{tested}" data-gpu="{gk}" data-tags="{" ".join(tags)}" data-name="{html.escape(name.lower())}">'
        f'<summary><span class="rank">{r}</span><span class="name">{html.escape(name)}</span>'
        f'<span class="gpu">{md(gpu)}</span><span class="score">{sc}</span>'
        f'<span class="rep">{md(rep)}</span></summary>'
        f'<div class="more">{detail}<a class="src" href="{url}" rel="noopener">Source on GitHub &rarr;</a></div></details>')
block = "\n".join(rows)
h = re.sub(r"(<!--standings:start-->).*?(<!--standings:end-->)", lambda m: m.group(1) + "\n" + block + "\n" + m.group(2), h, count=1, flags=re.S)
cards = []
for t, (label, desc) in TAGS.items():
    n, best, bs = stats.get(t, [0, None, None])
    if not n:
        continue
    cards.append(f'<button class="strat" data-t="{t}"><b>{html.escape(label)}</b><span class="n">{n} runtime{"s" if n != 1 else ""}</span>'
                 f'<p>{html.escape(desc)}</p><span class="best">Best ranked: {html.escape(best)} ({bs})</span></button>')
h = re.sub(r"(<!--strategies:start-->).*?(<!--strategies:end-->)", lambda m: m.group(1) + "\n" + "\n".join(cards) + "\n" + m.group(2), h, count=1, flags=re.S)
h = re.sub(r'(<span id="count">)\d*(</span>)', lambda m: f"{m.group(1)}{len(rows)}{m.group(2)}", h)
ix.write_text(h, encoding="utf-8", newline="\n")
print(f"sync-docs: {len(add)} credits added, {len(rows)} runtimes on the site")
