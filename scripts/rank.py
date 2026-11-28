# Usage: gh api repos/4lwayz/portwatch-vi/contents/data/tracker.json -H "Accept: application/vnd.github.raw" > pw.json; python -I scripts/rank.py pw.json ranks.json
# i95's own ranking: PortWatch VI's scoring formula (assets/js/scoring.js) over every runtime.
import json,sys,re,glob
d=json.load(open(sys.argv[1],encoding='utf-8'))
TICK={'loads':10,'boots':20,'menu':30,'world':40,'playable':50}
ST={'unknown':0,'often':.25,'sometimes':.55,'rarely':.8,'stable':1}
c=lambda v:min(1,max(0,float(v))) if isinstance(v,(int,float)) else 0
lv=lambda v:1 if v is True else 0 if v in (False,None) else c(v)
def score(g):
    s=prev=0; reached=None
    for k in TICK:
        v=lv(g.get(k))
        if v>=1: s=prev=TICK[k]; reached=k; continue
        if v>0: s=max(s,prev+v*(TICK[k]-prev))
        break
    st=g.get('stability'); st=c(st) if isinstance(st,(int,float)) else ST.get(st,0)
    q=.35*c(g.get('story'))+.25*c(g.get('render'))+.2*c((g.get('fps') or 0)/30)+.2*st
    return round(50+q*50 if reached=='playable' else s,1)
pw={e['repo'].lower():e for e in d['efforts'] if e.get('repo') and (e.get('gta') or {}).get('loads')}
P=lambda f,s=0,r=.25,t=0:dict(loads=1,boots=1,menu=1,world=1,playable=1,fps=f,story=s,render=r,stability=t)
W=lambda:dict(loads=1,boots=1,menu=1,world=1)
OURS=dict(vicetay=P(21.1,.1,.5,.8),anyxboxx=P(24,.1,.25),klbf=P(14,.1,.25,.8),victoryperf=P(10),scarlettport=P(7.5),
 amd=P(2.5,0,.25,.25),flamingo=P(8.6,0,.25,.55),ka1qux=P(10.5,0,.25,.55),vilocity=P(29,0,.25,.55),cblvictory=P(13.5),
 unified=P(2.5,0,.25,.8),vinividivici=P(6,0,.25,.8),vibrport=P(22),rufflab=P(6.5,0,.25,.8),darkworld=P(3),
 united=P(10,0,.25,.25),rdna=P(21,0,.25,.8),mavis9070xt=P(18.9,0,.25,.55),cgbj=P(25),naixue=P(11,.1),amdr13=P(4),
 ccs03=P(5,.1),lnx=P(5.5),victory8gb=P(0,.25,.25,1),peppa=P(23.5,0,.25,.55),leonida=P(10),sleyfork=P(20.6),
 scarlett3070=P(18.5),foxhound=P(18.5),sm86=P(28.5),slaktarn=P(22),viscosity=P(17.9),darkvictory=P(10),m1lk=P(4),
 minivi=P(0),victory4gb=P(6.75),neptune=P(14,0,.25,.55),kitty6=P(20.5,0,.25,.25),karamelo=P(11),coolfox12=P(7.7),
 apexvi=P(7,0,.25,.55),nvidiart=P(21.9),nvidia8gb=P(9),nvidiafg=P(11),razda4a=P(19.3,0,.25,.55),grimkaaah=P(24.3,0,.25,.25),pilotebox=P(21),victorious=P(26,0,.25,.55),danzi=P(11,.1,.25,.55),mandyport=P(7),lowstutter=P(0),amd4k=P(14.5),hydra=P(19),viport=P(0),rorodev=P(8),lite=P(16),el1xr=P(24.4,0,.25,.55),skornuta=P(10.5),bigcheese=P(17),mangrove=P(2.5,0,.25,.25))
rows=[]
for f in glob.glob('vendor/*/NEONBRIDGE.toml'):
    n=f.replace('\\','/').split('/')[-2]; t=open(f,encoding='utf-8').read()
    if re.search(r'^pin = true',t,re.M): continue
    u=re.search(r'^upstream = "https://github.com/([^"]+)"',t,re.M).group(1)
    e=pw.get(u.lower()); g=e['gta'] if e else OURS.get(n)
    if g is None: print('NO DATA',n,file=sys.stderr); continue
    rows.append((score(g),'PortWatch' if e else 'i95',n,u))
rows.sort(key=lambda r:(-r[0],r[2]))
json.dump([[i+1,*r] for i,r in enumerate(rows)],open(sys.argv[2],'w'))
for i,r in enumerate(rows): print(i+1,*r)
