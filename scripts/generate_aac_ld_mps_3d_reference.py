#!/usr/bin/env python3
"""Explicit original stereo arbitrary-downmix matrices plus Decimal temporal oracle."""
import pathlib,json,struct,hashlib,subprocess,sys,tempfile,zipfile,math
from aac_ld_mps_3d_decimal import inverse as stable_inverse,add as cadd,mul as cmul
from aac_ld_mps_3d_decimal import D,validate_reference
from decimal import Decimal as D,localcontext
from extract_aac_ld_mps_prepare_facts import load
ROOT=pathlib.Path(__file__).resolve().parents[1]
CORPUS=ROOT/'tests/fixtures/playback-errors/aac-ld-mps-3d'
def sha(b):return hashlib.sha256(b).hexdigest()
def main(archive):
 validate_reference();assert sys.byteorder=='little';facts,provenance=load(archive);assert facts==json.loads((ROOT/'tests/fixtures/playback-errors/aac-ld-mps-prepare/facts.json').read_text());m=json.loads((CORPUS/'manifest.json').read_text());records=bytearray();count=0
 seen=set();unique=[]
 for c in m['cases']:
  key=(c['tree'],c['bands'],c['variant'],c['slots'],c['pattern'])
  if key not in seen:seen.add(key);unique.append(c)
 m['cases']=unique
 for c in m['cases']:
  tree=c['tree'];assert tree in (2,3,4) and not any(c['cutoffs']);boxes=len(c['ott_bands'])
  for f in c['frames']:
   regions={r['id']:r for r in f['parameters']};slots=f['slots']+([c['slots']-1]if f['slots'][-1]!=c['slots']-1 else[])
   def value(id,ps,b):
    r=regions[id];grid=facts['dequant'+r['kind']];shift={'CLD':15,'ICC':0,'CPC':20}[r['kind']];return grid[r['values'][min(ps,1)][b-r['start']]+shift]
   for ps in range(len(slots)):
    for b in range(c['bands']):
     lfe=4 if tree==0 else 2 if tree==1 else 0 if tree in(2,3,4)else None;active=lfe is not None and b<c['ott_bands'][lfe]
     mode=0;x=y=0.;rho=1.;residual=False
     if c['ttt']:
      lo,hi,split=c['ttt'];high=b>=split;mode=hi if high else lo;kind='cpc'if mode<2 else'cld';x=value(f'ttt-{kind}:0:{int(high)}:0',ps,b);y=value(f'ttt-{kind}:0:{int(high)}:1',ps,b);rho=value(f'ttt-icc:0:{int(high)}',ps,b)if mode<2 else 1.;residual=b<c['cutoffs'][boxes]
     records.extend(struct.pack('<6I5d',tree,mode,residual,c['inversion'],active,boxes,x,y,rho,value('downmix-gain:0',ps,b),value('downmix-gain:1',ps,b)));records.extend(struct.pack('<Id',b,facts['surroundGainTable'][c['surround_index']]));records.extend(b''.join(struct.pack('<III',ch['left'][b],ch['right'][b],ch['phase'][b])for ch in c['hrtf']));count+=1
     for box in range(boxes):
      cld=value(f'ott-cld:{box}',ps,b)if b<c['ott_bands'][box]else 0.;icc=1. if box==lfe else value('ott-icc:all'if c['one_icc']else f'ott-icc:{box}',ps,b)
      records.extend(struct.pack('<ddI',cld,icc,b<c['cutoffs'][box]))
 records=bytearray(struct.pack('<I',count))+records
 with tempfile.TemporaryDirectory(prefix='fvid-assemble-reference-')as tmp:
  work=pathlib.Path(tmp);members={}
  with zipfile.ZipFile(archive)as z:
   for n in z.namelist():
    if n.startswith(('mp4spatialdec/src/','mp4spatialdec/include/','general/'))and n.endswith(('.c','.h')):
     assert '..'not in pathlib.Path(n).parts;b=z.read(n);dest=work/n;dest.parent.mkdir(parents=True,exist_ok=True);dest.write_bytes(b);members[n]=sha(b)
  src=work/'mp4spatialdec/src';(work/'libtsp.h').write_text('typedef struct AFILE AFILE;\n');driver=ROOT/'scripts/support/aac_ld_mps_3d_reference_probe.c';exe=work/'probe'
  args=['cc','-O2','-DLD_MPS','-ffunction-sections','-fdata-sections','-I',str(work),'-I',str(src),'-I',str(work/'mp4spatialdec/include'),str(driver)]+(['-Wl,-dead_strip']if sys.platform=='darwin'else['-Wl,--gc-sections'])
  result=subprocess.run(args+['-lm','-o',str(exe)],capture_output=True,timeout=60)
  if result.returncode:raise RuntimeError(result.stderr.decode())
  (work/'input.bin').write_bytes(records);subprocess.run([str(exe),str(work/'input.bin'),str(work/'output.bin')],check=True,timeout=60);raw=(work/'output.bin').read_bytes()
 at=0;expected=bytearray();cases=[];adjustments=0;regularized=0;rank_deficient=0;stock_nonfinite=0;stock_max_difference=0.;stable_branches=0
 with localcontext()as ctx:
  ctx.prec=80
  for c in m['cases']:
   tree=c['tree'];channels=5 if tree<2 else 6 if tree==2 else 2 if tree==7 else 8;inputs=1 if tree in(0,1,7)else 6 if tree in(5,6)else 3;outputs=2 if tree==7 else 6 if tree<=2 else 8;band_widths=[channels*inputs*2,outputs*channels,outputs*channels];previous=[[D(0)]*(c['bands']*w)for w in band_widths];frames=[]
   for f in c['frames']:
    slots=f['slots']+([c['slots']-1]if f['slots'][-1]!=c['slots']-1 else[]);smooth=f['smoothing']+([f['smoothing'][-1]]if len(slots)>2 else[]);saved=[[],[],[]];last=-1
    for ps,slot in enumerate(slots):
     endpoint=[[],[],[]]
     for band in range(c['bands']):
      n=sum(band_widths);values=list(struct.unpack_from('<'+'d'*n,raw,at));at+=n*8;mw=band_widths[0];dw=band_widths[1];m1=values[:mw];dry=values[mw:mw+dw];wet=values[mw+dw:]
      plain=list(struct.unpack_from('<'+'d'*mw,raw,at));at+=mw*8;pre=list(struct.unpack_from('<9d',raw,at));at+=72
      assert all(math.isfinite(v)for v in plain+pre+dry+wet)
      lo,hi,split=c['ttt'];mode=hi if band>=split else lo
      priority=c['inversion']and mode in(0,1,2,4)
      if not priority:
       regions={r['id']:r for r in f['parameters']}
       def cld(box):
        r=regions[f'ott-cld:{box}'];return facts['dequantCLD'][r['values'][min(ps,1)][band-r['start']]+15]
       indices=tuple((ch['left'][band],ch['right'][band],ch['phase'][band])for ch in c['hrtf'])
       q,reg,rank=stable_inverse([pre[i:i+3]for i in(0,3,6)],indices,(cld(1),cld(2)),facts['surroundGainTable'][c['surround_index']],band)
       stable_branches+=1;regularized+=int(reg);rank_deficient+=int(rank)
       corrected=plain.copy()
       for row in range(channels):
        for col in range(2):
         z=cadd(cmul((D.from_float(plain[row*6]),D.from_float(plain[row*6+1])),q[0][col]),cmul((D.from_float(plain[row*6+2]),D.from_float(plain[row*6+3])),q[1][col]))
         corrected[row*6+2*col]=float(z[0]);corrected[row*6+2*col+1]=float(z[1])
       for a,b in zip(m1,corrected):
        if math.isfinite(a):stock_max_difference=max(stock_max_difference,abs(a-b))
        else:stock_nonfinite+=1
       m1=corrected
      else:assert all(math.isfinite(v)for v in m1)
      if tree==7:
       for row in range(2):dry[row*2+1]=(1. if row==0 else-1.)if band<c['cutoffs'][0]else 0.;adjustments+=1
      planes=[m1[::2]+m1[1::2],dry,wet]
      for plane,terms in enumerate(planes):
       width=band_widths[plane];on=smooth[ps]['mask'][band]and band>=max(c['cutoffs']);delta=D(slot-last)/smooth[ps]['time']
       for col,value in enumerate(terms):
        value=D.from_float(value)
        if on:value=delta*value+(1-delta)*previous[plane][band*width+col]
        endpoint[plane].append(value)
     for plane in range(3):previous[plane]=endpoint[plane];saved[plane].append(endpoint[plane])
     last=slot
    frame=dict(slots=slots,offsets=[],widths=[c['bands']*w for w in band_widths])
    for plane in range(3):
     frame['offsets'].append(len(expected))
     for endpoint in saved[plane]:expected.extend(struct.pack('<'+'f'*len(endpoint),*[float(x)for x in endpoint]))
    frames.append(frame)
   cases.append(dict(name=c['name'],frames=frames))
 assert at==len(raw);(CORPUS/'expected.bin').write_bytes(expected);(CORPUS/'oracle.json').write_text(json.dumps(dict(scope='Original C stereo arbitrary-downmix raw matrices plus80-digit Decimal smoothing. Trees2/3/4 only,no residual audio.',cases=cases),separators=(',',':'))+'\n')
 report=dict(archive_sha256=provenance['archive_sha256'],reference_members=members,driver_sha256=sha(driver.read_bytes()),generator_sha256=sha(pathlib.Path(__file__).read_bytes()),input_sha256=sha(records),raw_reference_output_sha256=sha(raw),expected_sha256=sha(expected),band_endpoints=count,expected_scalar_endpoints=len(expected)//4,trees=[2,3,4],residual_audio=False,stable_inverse_branches=stable_branches,regularized_branches=regularized,rank_deficient_branches=rank_deficient,stock_float_nonfinite=stock_nonfinite,stock_float_max_difference=stock_max_difference,polar_reference_sha256=sha((ROOT/'scripts/aac_ld_mps_3d_decimal.py').read_bytes()),scope='Original C raw TTT/OTT matrices and M2;independent80-digit polar-factor3D inverse and smoothing for every3D branch. Unmodified standard-inverse-priority outputs retained. Symmetric/asymmetric fixtures. Stock float3D deviations/nonfinite results measured separately;not used as stable regularization truth. Endpoints stored as f32. Component evidence,not independent whole-codec PCM/movie/playback acceptance.')
 (ROOT/'docs/validation/aac-ld-mps-3d-reference-2026-10-31.json').write_text(json.dumps(report,indent=2)+'\n');print(count,len(expected),adjustments)
if __name__=='__main__':main(pathlib.Path(sys.argv[1]))
