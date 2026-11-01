from pathlib import Path
import subprocess,json,statistics,hashlib,datetime
q=Path(__file__).parent;target=Path('/private/tmp/fvid-clock-description-target-20261101');fixtures=Path('tests/fixtures/playback-errors/mp4-clock-description-scan');names={'primary':'frames262144-unused32.mp4','control':'frames262144-unused0.mp4'};source=Path('src/playback/audio/mp4_clock_recovery.rs');frozen=hashlib.sha256(source.read_bytes()).hexdigest();results={};raw={}
for kind,name in names.items():
    rounds=[]
    for round in range(7):
        result={};order=['baseline','candidate']if round%2==0 else['candidate','baseline']
        for version in order:
            assert hashlib.sha256(source.read_bytes()).hexdigest()==frozen
            r=subprocess.run([str(target/(version+'-open')),str(fixtures/name),'9'],check=True,text=True,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
            result[version]=json.loads(r.stdout)
        assert result['baseline']['fingerprint']==result['candidate']['fingerprint']
        result['order']=order;result['utc']=datetime.datetime.now(datetime.timezone.utc).isoformat();result['load']=subprocess.check_output(['uptime'],text=True).strip();rounds.append(result)
    raw[kind]=rounds
    before=statistics.median(r['baseline']['median_ns']for r in rounds);after=statistics.median(r['candidate']['median_ns']for r in rounds)
    results[kind]={'before_ns':before,'after_ns':after,'reduction_percent':100*(before-after)/before,'wins':sum(r['candidate']['median_ns']<r['baseline']['median_ns']for r in rounds),'identical_fingerprints':True}
(q/'measurement-pairs.json').write_text(json.dumps(raw,indent=2)+'\n')
p=results['primary'];c=results['control'];accepted=p['reduction_percent']>=10 and p['wins']>=6 and (c['after_ns']<=c['before_ns']*1.1 or c['after_ns']-c['before_ns']<=100000)
results.update(criteria_passed=accepted,aggregation='median of seven per-process medians; nine measured opens per process after two warmups',production_sha256=frozen,binary_sha256={version:hashlib.sha256((target/(version+'-open')).read_bytes()).hexdigest()for version in ['baseline','candidate']})
(q/'measurement-result.json').write_text(json.dumps(results,indent=2)+'\n');print(json.dumps(results,indent=2),flush=True)
if not accepted:raise SystemExit('predeclared criteria failed; candidate must not be accepted')
