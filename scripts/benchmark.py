#!/usr/bin/env python3
"""Deterministic FFmpeg differential checks and warm-cache CLI benchmarks."""
import datetime,hashlib,json,pathlib,platform,random,re,statistics,subprocess,time,os,tempfile
ROOT=pathlib.Path(__file__).resolve().parents[1]; DATA=ROOT/'benchmarks/data'; DATA.mkdir(exist_ok=True)
BIN=str(ROOT/'target/release/fvid')
def run(cmd,**kw): return subprocess.run(cmd,check=True,stderr=subprocess.PIPE,**kw)
def make(w,h,n,fmt):
 p=DATA/f'{w}x{h}-{n}-{fmt}.y4m'
 if not p.exists(): run(['ffmpeg','-v','error','-f','lavfi','-i',f'testsrc2=size={w}x{h}:rate=30','-frames:v',str(n),'-pix_fmt',fmt,'-strict','-1',str(p)])
 return p

def commands(src,crop,hf,vf):
 args=[];filters=[]
 if crop:
  x,y,w,h=crop;args+=['--crop',f'{x}:{y}:{w}:{h}'];filters+=[f'crop={w}:{h}:{x}:{y}']
 if hf: args+=['--hflip'];filters+=['hflip']
 if vf: args+=['--vflip'];filters+=['vflip']
 ff=['ffmpeg','-nostdin','-v','error','-i',str(src),'-an','-sn']
 if filters: ff+=['-vf',','.join(filters)]
 output=['-c:v','rawvideo','-pix_fmt',src.stem.split('-')[-1],'-strict','-1','-f','yuv4mpegpipe','-']
 return {'fvid':[BIN,str(src),'-']+args,'ffmpeg_default':ff+output,'ffmpeg_1thread':ff[:1]+['-filter_threads','1','-threads','1']+ff[1:]+['-threads','1']+output}

def payload(path):
 with open(path,'rb') as f:
  header=f.readline();w=int(re.search(rb' W(\d+)',header)[1]);h=int(re.search(rb' H(\d+)',header)[1]);c=re.search(rb' C([^\s]+)',header)[1]
  n=w*h*(3 if c.startswith(b'444') else 2) if not c.startswith(b'420') else w*h*3//2
  digest=hashlib.sha256();count=0
  while marker:=f.readline():
   assert marker.startswith(b'FRAME');frame=f.read(n);assert len(frame)==n;digest.update(frame);count+=1
  return {'width':w,'height':h,'frames':count,'sha256':digest.hexdigest()}

checks=[]
for fmt in ['yuv420p','yuv422p','yuv444p']:
 src=make(640,360,7,fmt)
 for name,crop,hf,vf in [('copy',None,False,False),('hflip',None,True,False),('vflip',None,False,True),('fused',(32,16,512,288),True,True)]:
  signatures={}
  for engine,cmd in commands(src,crop,hf,vf).items():
   out=DATA/'check.y4m'
   with open(out,'wb') as f: run(cmd,stdout=f)
   signatures[engine]=payload(out)
  assert len({json.dumps(v,sort_keys=True) for v in signatures.values()})==1,(fmt,name,signatures)
  checks.append({'format':fmt,'case':name,'signature':signatures['fvid']})
print('Differential correctness: 12 cases passed',flush=True)

results=[];rng=random.Random(451)
for w,h,n in [(1280,720,180),(1920,1080,120),(3840,2160,60)]:
 src=make(w,h,n,'yuv420p')
 for name,crop,hf,vf in [('copy',None,False,False),('hflip',None,True,False),('vflip',None,False,True),('fused',(w//8//2*2,h//8//2*2,w//2,h//2),True,True)]:
  cmds=commands(src,crop,hf,vf);samples={engine:[] for engine in cmds};rss={engine:[] for engine in cmds}
  for cmd in cmds.values(): run(cmd,stdout=subprocess.DEVNULL)
  for round_ in range(7):
   order=list(cmds);rng.shuffle(order)
   for engine in order:
    with tempfile.TemporaryFile() as err:
     start=time.perf_counter_ns();p=subprocess.Popen(cmds[engine],stdout=subprocess.DEVNULL,stderr=err)
     _,status,usage=os.wait4(p.pid,0);p.returncode=os.waitstatus_to_exitcode(status);elapsed=(time.perf_counter_ns()-start)/1e9
     if p.returncode: err.seek(0);raise RuntimeError(err.read().decode())
    samples[engine].append(elapsed)
    rss[engine].append(usage.ru_maxrss)
  result={'resolution':f'{w}x{h}','frames':n,'case':name,'commands':cmds,'seconds':samples,'max_rss_bytes':rss,'median_seconds':{k:statistics.median(v) for k,v in samples.items()},'fps':{k:n/statistics.median(v) for k,v in samples.items()}}
  results.append(result);(ROOT/'benchmarks/partial-results.json').write_text(json.dumps(results,indent=2));print(result['resolution'],name,{k:round(v,4) for k,v in result['median_seconds'].items()},flush=True)
report={'created_utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),'platform':platform.platform(),'machine':platform.machine(),'cpu':(subprocess.run(['sysctl','-n','machdep.cpu.brand_string'],capture_output=True,text=True).stdout.strip() or 'unavailable in sandbox'),'ffmpeg':subprocess.check_output(['ffmpeg','-version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'method':'Warm cache; local input file; Y4M serialization to OS null sink; includes process startup; per-process RSS from wait4; 1 warmup and 7 seeded randomized rounds per case; no fsync; CPU only; raw frames; no codec performance claims. RSS is platform-specific macOS bytes.','correctness':checks,'results':results}
(ROOT/'benchmarks/results.json').write_text(json.dumps(report,indent=2))
print('Saved benchmarks/results.json',flush=True)
