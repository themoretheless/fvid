"""Replay paired Cargo input-watch measurements; preserves current scripts unless an improvement is confirmed."""
import json,os,pathlib,subprocess,time,hashlib,traceback,tempfile
ROOT=pathlib.Path(__file__).resolve().parents[2]
OUT=pathlib.Path(tempfile.mkdtemp(prefix='fvid-build-watch-measurement-'))
SCRIPTS={'root':ROOT/'build.rs','media':ROOT/'crates/fvid-media/build.rs'}
INITIAL={k:p.read_text() for k,p in SCRIPTS.items()}
DIRECTIVE='    println!("cargo:rerun-if-changed=build.rs");\n'
ORIGINAL={k:s.replace(DIRECTIVE,'',1) for k,s in INITIAL.items()}
assert all('cargo:rerun-if-changed=' not in s for s in ORIGINAL.values()), 'Additional file inputs require a new benchmark baseline'
FIXED={k:s.replace('fn main() {\n','fn main() {\n'+DIRECTIVE,1) for k,s in ORIGINAL.items()}
PROBE='    std::io::Write::write_all(&mut std::fs::OpenOptions::new().create(true).append(true).open(std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("fvid_build_watch_counter")).unwrap(), b"x").unwrap();\n'
EXPECTED=dict(INITIAL)
ENV=dict(os.environ,CARGO_TARGET_DIR=os.environ.get('CARGO_TARGET_DIR',str(pathlib.Path(tempfile.gettempdir())/'fvid-improvements-target')))
META=json.loads(subprocess.check_output(['cargo','metadata','--locked','--offline','--no-deps','--format-version=1'],cwd=ROOT,env=ENV))
IDS={p['id']:k for p in META['packages'] for k,path in SCRIPTS.items() if pathlib.Path(p['manifest_path'])==path.parent/'Cargo.toml'}
assert set(IDS.values())=={'root','media'}
NONCE=str(time.time_ns())
MARKERS=[ROOT/'docs/improvements'/('build-watch-probe-'+NONCE+'.md'),ROOT/'crates/fvid-media/tests'/('build_watch_probe_'+NONCE+'.txt')]
REPORT={'original':ORIGINAL,'fixed':FIXED,'runs':[],'pairs':[],'status':'running'}
WINNER=False

def write_variant(values,instrument):
    for k,p in SCRIPTS.items():
        assert p.read_text()==EXPECTED[k],f'concurrent change in {p}'
        text=values[k].replace('fn main() {\n','fn main() {\n'+PROBE,1) if instrument else values[k]
        p.write_text(text);EXPECTED[k]=text

def markers(label):
    for path in MARKERS:path.write_text('FVID build-watch probe '+NONCE+' '+label+'\n')

def run(label):
    print(json.dumps({'starting':label}),flush=True)
    log=OUT/(label+'.jsonl');err=OUT/(label+'.stderr')
    start=time.monotonic()
    with log.open('w') as stdout,err.open('w') as stderr:
        result=subprocess.run(['cargo','check','--locked','--offline','--no-default-features','--lib','--message-format=json'],cwd=ROOT,env=ENV,stdout=stdout,stderr=stderr)
    elapsed=time.monotonic()-start
    events=[json.loads(line) for line in log.read_text().splitlines() if line.startswith('{')]
    dirs={IDS[e['package_id']]:e['out_dir'] for e in events if e.get('reason')=='build-script-executed' and e.get('package_id') in IDS}
    fresh={IDS[e['package_id']]:e['fresh'] for e in events if e.get('reason')=='compiler-artifact' and e.get('package_id') in IDS and 'lib' in e.get('target',{}).get('kind',[])}
    data={'label':label,'seconds':elapsed,'exit_code':result.returncode,'out_dirs':dirs,'library_fresh':fresh,'stdout':str(log),'stderr':str(err)}
    REPORT['runs'].append(data);(OUT/'report.json').write_text(json.dumps(REPORT,indent=2)+'\n')
    print(json.dumps(data),flush=True)
    assert result.returncode==0,f'Cargo failed: {label}'
    return data

def counts(dirs):return {k:(pathlib.Path(p)/'fvid_build_watch_counter').stat().st_size for k,p in dirs.items()}

def measure(label,values):
    write_variant(values,True);warm=run(label+'-warm');assert set(warm['out_dirs'])=={'root','media'}
    before=counts(warm['out_dirs']);time.sleep(1.1);markers(label)
    result=run(label+'-probe');after=counts(warm['out_dirs']);result['script_invocations']={k:after[k]-before[k] for k in before}
    print(json.dumps({'measurement':result}),flush=True)
    return result

try:
    markers('initial')
    for index in range(2):
        before=measure(f'before-{index}',ORIGINAL)
        after=measure(f'after-{index}',FIXED)
        pair={'before':before,'after':after};REPORT['pairs'].append(pair)
        (OUT/'report.json').write_text(json.dumps(REPORT,indent=2)+'\n')
    WINNER=all(all(pair['before']['script_invocations'][k]>=1 and pair['after']['script_invocations'][k]==0 for k in SCRIPTS) and pair['after']['seconds']<pair['before']['seconds']*.8 for pair in REPORT['pairs'])
    write_variant(FIXED if WINNER else INITIAL,False)
    warm=run('final-uninstrumented-warm')
    if WINNER:
        mtimes={k:(pathlib.Path(p).parent/'output').stat().st_mtime_ns for k,p in warm['out_dirs'].items()}
        time.sleep(1.1);markers('final');probe=run('final-uninstrumented-probe')
        now={k:(pathlib.Path(p).parent/'output').stat().st_mtime_ns for k,p in warm['out_dirs'].items()}
        assert mtimes==now,'uninstrumented scripts reran after unrelated marker change'
        assert probe['library_fresh']=={'root':True,'media':True},probe['library_fresh']
    REPORT['status']='measured improvement' if WINNER else 'rejected and reverted'
except BaseException as error:
    WINNER=False;REPORT['status']='failed';REPORT['error']=str(error);traceback.print_exc()
finally:
    for k,p in SCRIPTS.items():
        current=p.read_text()
        if current==EXPECTED[k]:
            desired=FIXED[k] if WINNER else INITIAL[k]
            if current!=desired:p.write_text(desired)
        else:p.write_text(current.replace(PROBE,'',1))
    for path in MARKERS:
        if path.exists() and path.read_text().startswith('FVID build-watch probe '+NONCE):path.unlink()
    REPORT['final_hashes']={k:hashlib.sha256(p.read_bytes()).hexdigest() for k,p in SCRIPTS.items()}
    (OUT/'report.json').write_text(json.dumps(REPORT,indent=2)+'\n')
    print(json.dumps({'status':REPORT['status'],'report':str(OUT/'report.json')}),flush=True)
