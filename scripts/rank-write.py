"""Writes scripts/rank.py's ranks into vendor/*/NEONBRIDGE.toml and reorders README's table. Run by bump-upstreams.sh."""
import json, re, os, sys
os.chdir(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
ranks = json.load(open(sys.argv[1]))
byrepo = {}
for r, s, src, n, u in ranks:
    byrepo[u.lower()] = (r, s)
    p = f'vendor/{n}/NEONBRIDGE.toml'
    t = open(p, encoding='utf-8').read()
    sc = ('%g' % s) if s != int(s) else '%.1f' % s
    how = "its own assessment" if src == 'PortWatch' else "i95's assessment from its author's report"
    line = f'rank = {r}   # i95 rank: score {sc} by PortWatch VI\'s formula ({how}; scripts/rank.py)'
    if re.search(r'^rank = ', t, re.M):
        t2 = re.sub(r'^rank = (\d+)', lambda m: f'rank = {r}', t, count=1, flags=re.M)
    else:
        t2 = re.sub(r'^(gpu = .*\n)', lambda m: m.group(1) + line + '\n', t, count=1, flags=re.M)
    if t2 != t:
        open(p, 'w', encoding='utf-8', newline='\n').write(t2)
new = {}
md = open('README.md', encoding='utf-8').read().split('\n')
rows, start, end = [], None, None
for i, l in enumerate(md):
    if re.match(r'^\| \d+ \| [\d.]+ \| \[', l):
        start = i if start is None else start
        end = i
        rows.append(l)
for k, v in new.items():
    r, s = byrepo[k]
    rows.append(f'| {r} | {s:g} | {v}')
out = []
for l in rows:
    m = re.search(r'\]\(https://github\.com/([^)]+?)/?\)', l)
    k = m.group(1).lower() if m else None
    if k in byrepo:
        r, s = byrepo[k]
        l = re.sub(r'^\| \d+ \| [\d.]+ \|', f'| {r} | {s:g} |', l)
    out.append((int(re.match(r'^\| (\d+)', l).group(1)), l))
out.sort(key=lambda x: x[0])
md[start:end + 1] = [l for _, l in out]
open('README.md', 'w', encoding='utf-8', newline='\n').write('\n'.join(md))
print(len(out), 'rows')
