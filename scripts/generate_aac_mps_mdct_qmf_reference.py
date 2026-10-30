#!/usr/bin/env python3
"""Explicit ISO MDCT-to-QMF extraction and owned synthetic component generation.
Reference compilation is separate from ordinary tests. Numeric window facts are
transcribed at their reference f32 realization; no reference code enters Rust.
"""
import hashlib,json,math,re,struct,subprocess,sys,tempfile,zipfile
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1];OUT=ROOT/'tests/fixtures/playback-errors/aac-mps-mdct-qmf'
def sha(b):return hashlib.sha256(b).hexdigest()
def main(archive):
 raw=Path(archive).read_bytes();assert sha(raw)=='b6b27e2f9af6ee8f950af8d7074afc467e66d93372328c0ec55282fff71d1f83'
 with zipfile.ZipFile(archive)as z:
  code=z.read('mp4spatialdec/src/sac_mdct2qmf.c');tables=z.read('mp4spatialdec/src/sac_mdct2qmf_wf_tables.c')
 source=code.decode();source=source[source.index('static void'):];table=tables.decode();table=table[table.index('const float'):]
 facts='// Numeric residual frequency windows from the pinned ISO reference.\n// Factual f32 table realizations only; generated explicitly, never by tests.\n'
 sizes=[]
 for m,body in re.findall(r'const float wf_(\d+)\[\d+\]\s*=\s*\{(.*?)\};',table,re.S):
  values=[struct.unpack('<f',struct.pack('<f',float(v)))[0]for v in re.findall(r'[-+]?\d+\.\d+(?:[eE][-+]?\d+)?',body)];m=int(m);assert len(values)==m;sizes.append(m)
  facts+=f'const W{m}: [f64; {m}] = [\n'+''.join('    '+repr(v)+',\n'for v in values)+'];\n'
 facts+='pub(super) fn window(n: usize) -> Option<&\'static [f64]> {\n    Some(match n {\n'+''.join(f'        {n} => &W{n},\n'for n in sizes)+'        _ => return None,\n    })\n}\n'
 (ROOT/'crates/fvid-media/src/owned_aac/aac_mps_mdct_qmf_facts.rs').write_text(facts)
 prefix='''#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#define PI 3.14159265358979323846
#define MAX_TIME_SLOTS 72
#define MAX_NUM_QMF_BANDS 128
#define MAX_RESIDUAL_CHANNELS 10
#define AAC_FRAME_LENGTH 1024
#define AAC_SHORT_FRAME_LENGTH 128
#define MDCT_LENGTH_LO 1024
#define MDCT_LENGTH_HI 2048
#define MDCT_LENGTH_SF 384
#define MAX_UPD_QMF 36
#define UPD_QMF_15 15
#define UPD_QMF_16 16
#define UPD_QMF_18 18
#define UPD_QMF_24 24
#define UPD_QMF_30 30
#define UPD_QMF_32 32
#define ONLYLONG_WINDOW 0
#define LONGSTART_WINDOW 1
#define EIGHTSHORT_WINDOW 2
#define LONGSTOP_WINDOW 3
#define SPACE_MDCT2QMF_ERROR_NO_ERROR 0
#define SPACE_MDCT2QMF_ERROR_INVALID_WINDOW_TYPE 1
#define SPACE_MDCT2QMF_ERROR_INVALID_QMF_UPDATE 2
typedef int SPACE_MDCT2QMF_ERROR;
typedef int BLOCKTYPE;
typedef struct {double fi,beta;} Twiddle;
Twiddle *twiddleExtern=NULL;
typedef struct {float qmfResidualReal[10][144][128],qmfResidualImag[10][144][128];} spatialDec;
'''
 driver=r'''
static uint32_t read_u32(void){uint32_t n;if(fread(&n,4,1,stdin)!=1)exit(3);return n;}
int main(void){static float re[144][128],im[144][128];float input[2048];
 int update=read_u32(),frames=read_u32(),count=read_u32(),slots=update*frames;
 for(int f=0;f<count;f++){
  int seq=read_u32();for(int b=0;b<frames;b++){
   int n=seq==2&&(update==18||update==24||update==30)?2048:1024;
   for(int i=0;i<2048;i++)input[i]=0;
   if(fread(input,4,n,stdin)!=(size_t)n)return 4;
   if(SPACE_MDCT2QMF_Process(update,input,re,im,seq,b*update)!=0)return 5;
  }
  for(int t=0;t<slots;t++)for(int b=0;b<64;b++){
   float r=re[t][b]/32768.f,i=im[t][b]/32768.f;fwrite(&r,4,1,stdout);fwrite(&i,4,1,stdout);
   re[t][b]=re[t+slots][b];im[t][b]=im[t+slots][b];re[t+slots][b]=im[t+slots][b]=0;
  }
 }return 0;}
'''
 inputs=bytearray();expected=bytearray();cases=[];sequences=[0,0,1,2,2,3,0,1,2,3,0,0]
 with tempfile.TemporaryDirectory(prefix='fvid-residual-qmf-')as tmp:
  w=Path(tmp);(w/'probe.c').write_text(prefix+table+source+driver)
  subprocess.run(['cc','-O1','-ffp-contract=off','-fsanitize=undefined,bounds','-fno-sanitize-recover=all',str(w/'probe.c'),'-lm','-o',str(w/'probe')],check=True)
  for update,frames in[(15,1),(15,2),(16,1),(16,2),(18,1),(24,1),(30,1),(32,1)]:
   for mode in('dense','silent','impulse'):
    payload=bytearray(struct.pack('<3I',update,frames,len(sequences)));blocks=[]
    for f,seq in enumerate(sequences):
     payload.extend(struct.pack('<I',seq));parts=[]
     for b in range(frames):
      spectra=[]
      for extra in range(2 if seq==2 and update in(18,24,30)else 1):
       values=[]
       for i in range(1024):
        value=0. if mode=='silent'or f==6 else 512*math.sin((i*3+f*17+b*11+extra*31)*.013)*math.cos(i*.037)if mode=='dense'else 1024. if i in(0,127,128,767,768,895,896,959,960,1023)else 0.
        values.append(value)
       data=struct.pack('<1024f',*values);spectra.append(dict(offset=len(inputs)));inputs.extend(data);payload.extend(data)
      parts.append(dict(first=spectra[0],extra=spectra[1]if len(spectra)>1 else None))
     blocks.append(dict(sequence=seq,parts=parts,expected_offset=len(expected)+f*update*frames*64*8))
    result=subprocess.run([str(w/'probe')],input=payload,capture_output=True,check=True,timeout=60).stdout
    assert len(result)==len(sequences)*update*frames*64*8;expected.extend(result);cases.append(dict(update=update,frames=frames,mode=mode,blocks=blocks))
 OUT.mkdir(exist_ok=True);(OUT/'input.f32le').write_bytes(inputs);(OUT/'expected.f32le').write_bytes(expected);(OUT/'manifest.json').write_text(json.dumps(dict(scope='Unmodified ISO conversion functions, default phase (twiddleExtern=NULL), full-complex64-bin output and overlap-buffer shifts. Not native LD phase or playback qualification.',cases=cases),separators=(',',':'))+'\n')
 report=dict(archive_sha256=sha(raw),source_sha256=sha(code),window_table_sha256=sha(tables),extracted_probe_sha256=sha((prefix+table+source+driver).encode()),generator_sha256=sha(Path(__file__).read_bytes()),facts_sha256=sha(facts.encode()),input_sha256=sha(inputs),expected_sha256=sha(expected),programmes=len(cases),scalar_values=len(expected)//4,scope='Unmodified ISO function extraction with original numeric tables. Default phase only. Ordinary tests use frozen outputs and owned Rust.')
 (ROOT/'docs/validation/aac-mps-mdct-qmf-reference-2026-10-30.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report,indent=2))
if __name__=='__main__':main(sys.argv[1])
