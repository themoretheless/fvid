"""Offline source-frozen qualification of the FVID-0407 signed-CTS follow-up."""
from pathlib import Path
import subprocess,json,hashlib,os,re
q=Path(__file__).resolve().parent
sources=json.loads((q/'after-source.json').read_text())
env=os.environ.copy();env.update(CARGO_TARGET_DIR='/private/tmp/fvid-negative-composition-target-20261101',CARGO_BUILD_JOBS='2',CARGO_PROFILE_TEST_DEBUG='0',CARGO_PROFILE_DEV_DEBUG='0')
targets=['mp4','mp4_time_header_sizes','mp4_time_header_flags','mp4_open_ended_edit','mp4_implicit_fragment_base','mp4_layered_seek_coverage','mp4_concat_selected_clock','mp4_seek_io_regressions','mp4_negative_composition_endpoint','mp4_presentation_endpoint','mp4_nondisplay_endpoint','mp4_decode_only_seek','mp4_edit_window_eof','mp4_wide_decode_clock']
commands={'integration':['cargo','test','--locked','--offline','--no-default-features','--features','player','--no-fail-fast']+sum((['--test',n]for n in targets),[]),'root-mp4':['cargo','test','--locked','--offline','--no-default-features','--features','player','--lib','mp4','--','--test-threads=2'],'core-check':['cargo','check','--locked','--offline','--no-default-features'],'player-check':['cargo','check','--locked','--offline','--no-default-features','--features','player']}
results={}
for k,cmd in commands.items():
 for n,h in sources.items():assert hashlib.sha256(Path(n).read_bytes()).hexdigest()==h
 with(q/(k+'.log')).open('wb')as f:r=subprocess.run(cmd,stdout=f,stderr=subprocess.STDOUT,env=env)
 text=(q/(k+'.log')).read_text();counts=re.findall(r'test result: (?:ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored;',text)
 results[k]=dict(command=cmd,exit_code=r.returncode,passed=sum(int(x[0])for x in counts),failed=sum(int(x[1])for x in counts),ignored=sum(int(x[2])for x in counts));(q/'qualification-results.json').write_text(json.dumps(results,indent=2)+'\n');print(k,results[k]['exit_code'],results[k]['passed'],flush=True)
 assert r.returncode==0
