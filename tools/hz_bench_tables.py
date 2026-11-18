"""Tables from a rotation-bench run (tools/hz_rotation.py --out DIR): per job and tier the kills in 30 s, basic and
skill hits (the hit hook's damage, before the critical), damage taken, lowest HP and SP at the end. A seeded run
(`--seeds`, folders j*_t*_sN) gets a row per seed and, in the last table, the seeds' mean with each seed's kills.

    python -X utf8 -I tools/hz_bench_tables.py DIR
"""
import re,os,collections,sys
D=sys.argv[1]
JOB=['워리어','로그','나이트','건너','총검사','정령사']
SK={0:{4:'강타',5:'충격',6:'돌격',7:'외침',8:'광기',9:'회전베기'},1:{4:'기습',5:'독수',6:'연격',7:'신속',8:'참격',9:'분신'},2:{4:'방패치기',5:'회복',6:'방패돌격',7:'반격',8:'방패밀기',9:'전력방어'},3:{4:'근접사격',5:'조준사격',6:'연사',7:'난사',8:'트릭샷',9:'저격'},4:{4:'포격',5:'돌진',6:'돌진(폭발)'},5:{4:'맹약',5:'동화',6:'정령(?)',7:'정령(?)'}}
def runs(j,t):
    seeded=sorted((n for n in os.listdir(D) if n.startswith(f"j{j}_t{t}_s")),key=lambda n:int(n.rsplit('_s',1)[1])) if os.path.isdir(D) else []
    return seeded or [f"j{j}_t{t}"]
def run(name):
    d=os.path.join(D,name)
    log=open(os.path.join(d,'log.txt'),encoding='utf-8',errors='replace').read().splitlines()
    f=0; start=8400; end=8400+30*80
    per=collections.defaultdict(list); taken=[]; kills=0; hp=[]; sp=None
    for l in log:
        m=re.search(r'\[probe\] f=(\d+)',l)
        if m: f=int(m[1])
        if not(start<=f<=end): continue
        m=re.search(r'bench hit (-?\d+) -> (-?\d+) action (\d+)',l)
        if m: per[int(m[3])].append(int(m[2]))
        m=re.search(r'bench taken (-?\d+)',l)
        if m: taken.append(-int(m[1]))
        if 'bench kill' in l: kills+=1
        m=re.search(r'stats HP (\d+)/(\d+)\s+SP (\d+)/(\d+)',l)
        if m: hp.append(int(m[1])); sp=(int(m[3]),int(m[4]))
    return per,taken,kills,hp,sp
print("| 직업 | 단계 | 처치 | 타격 수(기본/스킬) | 기본 한 타(훅, 크리 전) | 스킬 한 타 | 받은 피해 합(최대 한 대) | HP 최저 | 끝 SP |")
print("|---|---|---|---|---|---|---|---|---|")
grid={}
for t in (1,2,3,4):
    for j in range(6):
      for name in runs(j,t):
        try: per,taken,kills,hp,sp=run(name)
        except FileNotFoundError: continue
        grid.setdefault((t,j),[]).append(kills)
        seed=name.rsplit('_s',1)[1] if '_s' in name else ''
        basic=[v for a,vs in per.items() if a<4 for v in vs]; skills={a:vs for a,vs in per.items() if a>=4}
        sk=' · '.join(f"{SK[j].get(a,a)} {round(sum(v)/len(v))}×{len(v)}" for a,v in sorted(skills.items()))
        print(f"| {JOB[j]}{' s'+seed if seed else ''} | T{t} | **{kills}** | {len(basic)}/{sum(len(v) for v in skills.values())} | {round(sum(basic)/len(basic)) if basic else '-'} | {sk} | {sum(taken)} ({max(taken,default=0)}) | {min(hp) if hp else '-'} | {sp[0] if sp else '-'}/{sp[1] if sp else '-'} |")
print()
print("| 단계 | "+" | ".join(JOB)+" |"); print("|---|"+"---|"*6)
for t in (1,2,3,4):
    cell=lambda v: '-' if not v else str(v[0]) if len(v)==1 else f"{sum(v)/len(v):.1f} ({'/'.join(map(str,v))})"
    print(f"| T{t} | "+" | ".join(cell(grid.get((t,j))) for j in range(6))+" |")
