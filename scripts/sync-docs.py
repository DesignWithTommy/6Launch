"""Keeps docs/CREDITS.md and the website (index.html) in step with vendor/ and README.md.

Run by scripts/bump-upstreams.sh. Credits: adds any vendored runtime whose upstream
is not yet credited. Website: rewrites the ranked runtime list from README's table.
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
names = []
for l in (root / "README.md").read_text(encoding="utf-8").split("\n"):
    m = re.match(r"^\| (\d+) \| [\d.]+ \| \[([^\]]+)\]", l)
    if m:
        names.append((int(m.group(1)), m.group(2)))
items = "".join(f"<li>{html.escape(n)}</li>" for _, n in sorted(names))
h = re.sub(r"(<h2>Runtimes in the launcher</h2>.*?<ol>\n?)<li>.*?(\n?</ol>)", lambda m: m.group(1) + items + m.group(2), h, count=1, flags=re.S)
ix.write_text(h, encoding="utf-8", newline="\n")
print(f"sync-docs: {len(add)} credits added, {len(names)} runtimes on the site")
