#!/usr/bin/env python3
"""Optional original apply/decor/merge chain on saved assembly endpoints."""
import pathlib,json,hashlib,struct,subprocess,sys,tempfile,zipfile
from extract_aac_ld_mps_prepare_facts import load
ROOT=pathlib.Path(__file__).resolve().parents[1]
OUT=ROOT/'tests/fixtures/playback-errors/aac-ld-mps-pcm'
def sha(b):return hashlib.sha256(b).hexdigest()
def main(archive):
 assert sys.byteorder=='little';facts,provenance=load(archive);assert facts==json.loads((ROOT/'tests/fixtures/playback-errors/aac-ld-mps-prepare/facts.json').read_text())
 base=ROOT/'tests/fixtures/playback-errors/aac-ld-mps-assemble';m=json.loads((base/'manifest.json').read_text());oracle=json.loads((base/'oracle.json').read_text());endpoints=(base/'expected.bin').read_bytes();records=bytearray();cases=[];output_offset=0
 for index,(c,o)in enumerate(zip(m['cases'],oracle['cases'])):
  if c['variant']not in(0,5):continue
  tree=c['tree'];bands=c['bands'];bins=32 if c['variant']==0 else 64;boxes=len(c['cutoffs']);outputs=2 if tree==7 else 6 if tree<=2 else 8
  modes=[(c['ttt'][1]if b>=c['ttt'][2]else c['ttt'][0])for b in range(bands)]if c['ttt']else[1]*bands
  for mode in (0,1):
   sur=(tree+c['variant']+bands)%5 if tree!=7 else 0;lf=(tree*2+bands)%5 if tree!=7 else 0;clip=(tree+bands+mode)%8
   config_blob=(base/'configs.bin').read_bytes()[c['config_offset']:c['config_offset']+c['config_bytes']];wire=''.join(format(v,'08b')for v in config_blob)
   def setfield(w,at,width,value):return w[:at]+format(value,f'0{width}b')+w[at+width:]
   if tree!=7:wire=setfield(wire,20,3,sur);wire=setfield(wire,23,3,lf);wire=setfield(wire,26,3,clip)
   else:wire=setfield(wire,19,3,clip)
   config_hex=int(wire,2).to_bytes(len(config_blob),'big').hex()
   records.extend(struct.pack('<5I',tree,bands,bins,mode,boxes))
   records.extend(struct.pack('<3d',facts['clipGainTable'][clip],facts['surroundGainTable'][sur],facts['lfeGainTable'][lf]))
   for xs in(c['cutoffs'],modes,facts[f'QMF{bands}'][:bins]):records.extend(struct.pack('<'+'I'*len(xs),*xs))
   frames=[]
   for f,of in zip(c['frames'],o['frames']):
    sets=len(of['slots']);records.extend(struct.pack('<II',sets,c['slots']));records.extend(struct.pack('<'+'I'*sets,*of['slots']))
    for plane in range(3):
     offset=of['offsets'][plane];n=of['widths'][plane]*sets*4;records.extend(endpoints[offset:offset+n])
    frames.append(dict(offset=output_offset));output_offset+=c['slots']*outputs*bins*4
   cases.append(dict(name=c['name']+f'-mode{mode}',base_case=index,mode=mode,bins=bins,outputs=outputs,config=config_hex,gain_indices=[clip,sur,lf],frames=frames))
 records=bytearray(struct.pack('<I',len(cases)))+records
 with tempfile.TemporaryDirectory(prefix='fvid-qmf-reference-')as tmp:
  work=pathlib.Path(tmp);members={}
  with zipfile.ZipFile(archive)as z:
   for n in z.namelist():
    if n.startswith(('mp4spatialdec/src/','mp4spatialdec/include/','general/'))and n.endswith(('.c','.h')):
     assert '..'not in pathlib.Path(n).parts;b=z.read(n);dest=work/n;dest.parent.mkdir(parents=True,exist_ok=True);dest.write_bytes(b);members[n]=sha(b)
  src=work/'mp4spatialdec/src';(work/'libtsp.h').write_text('typedef struct AFILE AFILE;\n');driver=ROOT/'scripts/support/aac_ld_mps_pcm_reference_probe.c';exe=work/'probe'
  args=['cc','-O2','-DLD_MPS','-ffunction-sections','-fdata-sections','-I',str(work),'-I',str(src),'-I',str(work/'mp4spatialdec/include'),str(driver)]+[str(src/n)for n in('sac_calcM1andM2.c','sac_process.c','sac_decor.c','sac_hybfilter.c')]+(['-Wl,-dead_strip']if sys.platform=='darwin'else['-Wl,--gc-sections'])
  outputs={}
  (work/'input.bin').write_bytes(records)
  (work/'precise.c').write_text('#include "sac_sbrconst.h"\n#undef PI\n#define PI 3.141592653589793238462643383279502884\n#include "sac_polyphase.c"\n')
  for control in ('original','precise'):
   source=src/'sac_polyphase.c'if control=='original'else work/'precise.c'
   result=subprocess.run(args+[str(source),'-lm','-pthread','-o',str(exe)],capture_output=True,timeout=60)
   if result.returncode:raise RuntimeError(result.stderr.decode())
   subprocess.run([str(exe),str(work/'input.bin'),str(work/'output.bin')],check=True,timeout=60);outputs[control]=(work/'output.bin').read_bytes()
  expected=outputs['precise']
 assert len(expected)==output_offset
 OUT.mkdir(parents=True,exist_ok=True);(OUT/'expected.bin').write_bytes(expected);(OUT/'original.bin').write_bytes(outputs['original']);(OUT/'manifest.json').write_text(json.dumps(dict(scope='Controlled ordinary PCM component chain; not original frontend layouts, AAC transport or movie acceptance.',cases=cases),separators=(',',':'))+'\n')
 report=dict(archive_sha256=provenance['archive_sha256'],reference_members=members,driver_sha256=sha(driver.read_bytes()),generator_sha256=sha(pathlib.Path(__file__).read_bytes()),qmf_precision_controls='Original PI macro retained in original.bin; separate translation unit overrides only PI with a double-precision constant for expected.bin. Archived sources unchanged.',assembly_endpoints_sha256=sha(endpoints),input_sha256=sha(records),output_sha256=sha(expected),sequences=len(cases),frames=len(cases)*3,scalar_outputs=len(expected)//4,original_output_sha256=sha(outputs['original']),original_precise_max_error=max(abs(a[0]-b[0])for a,b in zip(struct.iter_unpack('<f',outputs['original']),struct.iter_unpack('<f',expected))),scope='Analysis/synthesis plus unmodified original M1 application, decorrelation, W merge and M2 application on controlled saved endpoints; both real/complex filter modes. Padded W geometry follows the qualified matrix interface, not the original frontend channel-count selection. HRTF branch is unselected and abort-guarded. Known 212 semantic fields come from the documented assembly oracle. AAC/LD-SBR transport integration, active temporal shaping and movie playback acceptance remain open.',units='Normalized synthetic samples scaled by32768 for original C, outputs divided back; source samples are dyadic and exactly representable in float.')
 (ROOT/'docs/validation/aac-ld-mps-pcm-reference-2026-10-30.json').write_text(json.dumps(report,indent=2)+'\n');print(len(cases),len(expected))
if __name__=='__main__':main(pathlib.Path(sys.argv[1]))
