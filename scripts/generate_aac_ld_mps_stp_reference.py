#!/usr/bin/env python3
"""Explicit component oracle generation using an audited ISO STP extraction.

The final active-bin formula is unchanged. Three reference-only UB removals:
* Skip bins0..11 in the final mix (wet already merged and zero there).
* Initialize absent tree7 C/LFE indices to -1.
* Process only the two populated dry groups for trees5/6.
This is an adapted reference, not an unmodified/full normative decoder oracle.
"""
import hashlib,json,math,struct,subprocess,sys,tempfile,zipfile
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'tests/fixtures/playback-errors/aac-ld-mps-stp'
def sha(b):return hashlib.sha256(b).hexdigest()
def main(archive):
 raw=Path(archive).read_bytes()
 assert sha(raw)=='b6b27e2f9af6ee8f950af8d7074afc467e66d93372328c0ec55282fff71d1f83'
 with zipfile.ZipFile(archive)as z:
  original=z.read('mp4spatialdec/src/sac_TPprocess.c');mapper=z.read('mp4spatialdec/src/sac_calcM1andM2.c').decode()
 source=original.decode().replace('\r\n','\n');source=source[source.index('#define BP_SIZE'):source.index('\nvoid tpProcess(')]
 old='int i_LF, i_RF, i_C, i_LFE, i_LS, i_RS, i_AL, i_AR;';assert source.count(old)==1;source=source.replace(old,'int i_LF, i_RF, i_C=-1, i_LFE=-1, i_LS, i_RS, i_AL, i_AR;')
 old='for (n=0; n<HP_SIZE-3+10; n++)';assert source.count(old)==1;source=source.replace(old,'for (n=12; n<HP_SIZE-3+10; n++)')
 mapping=mapper[mapper.index('static const int row2channelSTP'):mapper.index('static const int row2channelGES')]
 prefix='''#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#define LD_MPS 1
#define MAX_INPUT_CHANNELS 8
#define MAX_OUTPUT_CHANNELS 8
#define MAX_NUM_QMF_BANDS 64
#define MAX_M2_INPUT 8
#define ABS_THR (1e-9f*32768*32768)
typedef struct {int treeConfig,numInputChannels,numOutputChannels,qmfBands,hybridBands;
int tempShapeEnableChannelSTP[8];float hybOutputRealDry[8][32][64],hybOutputImagDry[8][32][64],hybOutputRealWet[8][32][64],hybOutputImagWet[8][32][64];} spatialDec;
'''+mapping+'''int SpatialDecGetChannelIndex(spatialDec*s,int row){return row2channelSTP[s->treeConfig][row];}
'''
 driver=r'''
static uint32_t u32(void){uint32_t n;if(fread(&n,4,1,stdin)!=1)exit(3);return n;}
static float number(void){float n;if(fread(&n,4,1,stdin)!=1)exit(4);return n;}
int main(void){static spatialDec s;int block,ch,t,b,k;
 s.treeConfig=u32();s.qmfBands=s.hybridBands=u32();s.numOutputChannels=u32();
 s.numInputChannels=(s.treeConfig<2||s.treeConfig==7)?1:2;
 int blocks=u32();for(block=0;block<blocks;block++){
  int slots=u32();for(ch=0;ch<8;ch++)s.tempShapeEnableChannelSTP[ch]=u32();
  for(k=0;k<2;k++)for(t=0;t<slots;t++)for(ch=0;ch<s.numOutputChannels;ch++)for(b=0;b<s.qmfBands;b++){
   float re=number()*32768.f,im=number()*32768.f;
   if(k==0){s.hybOutputRealDry[ch][t][b]=re;s.hybOutputImagDry[ch][t][b]=im;}
   else{s.hybOutputRealWet[ch][t][b]=re;s.hybOutputImagWet[ch][t][b]=im;}
  }
  for(t=0;t<slots;t++){
   for(ch=0;ch<s.numOutputChannels;ch++)for(b=0;b<12;b++){
    s.hybOutputRealDry[ch][t][b]+=s.hybOutputRealWet[ch][t][b];s.hybOutputImagDry[ch][t][b]+=s.hybOutputImagWet[ch][t][b];
    s.hybOutputRealWet[ch][t][b]=s.hybOutputImagWet[ch][t][b]=0;
   }
   subbandTP(&s,t);
   for(ch=0;ch<s.numOutputChannels;ch++)for(b=0;b<s.qmfBands;b++){
    float re=s.hybOutputRealDry[ch][t][b]/32768.f,im=s.hybOutputImagDry[ch][t][b]/32768.f;
    fwrite(&re,4,1,stdout);fwrite(&im,4,1,stdout);
   }
  }
 }return 0;}
'''
 samples=bytearray();expected=bytearray();cache={};cases=[]
 with tempfile.TemporaryDirectory(prefix='fvid-stp-reference-')as tmp:
  work=Path(tmp);(work/'probe.c').write_text(prefix+source+driver)
  subprocess.run(['cc','-O1','-ffp-contract=off','-fsanitize=undefined,bounds','-fno-sanitize-recover=all',str(work/'probe.c'),'-lm','-o',str(work/'probe')],check=True)
  for tree in range(8):
   outputs=[6,6,6,8,8,8,8,2][tree];count=[5,5,4,6,6,4,4,2][tree]
   for bands in(32,64):
    blocks=[];payload=bytearray(struct.pack('<4I',tree,bands,outputs,8))
    for frame,slots in enumerate((1,3,8,16,7,32,32,5)):
     flags=None if frame in(0,2,6)else[(ch+frame)%3!=1 for ch in range(count)]
     payload.extend(struct.pack('<9I',slots,*((flags or[False]*count)+[False]*(8-count))))
     key=(outputs,bands,frame);at=cache.get(key,len(samples));fresh=key not in cache;cache[key]=at;signal=bytearray()
     for kind in range(2):
      for t in range(slots):
       for ch in range(outputs):
        for b in range(bands):
         active=not(frame==2 or frame==3 and b<35 or frame==4 and b>=6)
         phase=(frame*83+t*13+ch*7+b*3)*.17
         z=complex(math.sin(phase),math.cos(phase*1.13))*.007*(1+(frame+ch)%3)if active else 0j
         if kind:z*=(-1 if frame==5 else .4)
         signal.extend(struct.pack('<ff',z.real,z.imag))
     if fresh:samples.extend(signal)
     payload.extend(signal);blocks.append(dict(offset=at,slots=slots,flags=flags,expected_offset=len(expected)+sum(x['slots']for x in blocks)*outputs*bands*8))
    result=subprocess.run([str(work/'probe')],input=payload,capture_output=True,check=True,timeout=30).stdout
    assert len(result)==104*outputs*bands*8;expected.extend(result);cases.append(dict(tree=tree,bands=bands,outputs=outputs,blocks=blocks))
 OUT.mkdir(exist_ok=True)
 (OUT/'component-input.f32le').write_bytes(samples);(OUT/'component-expected.f32le').write_bytes(expected)
 (OUT/'component.json').write_text(json.dumps(dict(scope=__doc__,cases=cases),separators=(',',':'))+'\n')
 report=dict(archive_sha256=sha(raw),original_member_sha256=sha(original),adapted_probe_sha256=sha((prefix+source+driver).encode()),generator_sha256=sha(Path(__file__).read_bytes()),input_sha256=sha(samples),expected_sha256=sha(expected),programmes=len(cases),scalar_values=len(expected)//4,scope=__doc__)
 (ROOT/'docs/validation/aac-ld-mps-stp-reference-2026-10-30.json').write_text(json.dumps(report,indent=2)+'\n');print(len(cases),'programmes',len(expected)//4,'scalar values')
if __name__=='__main__':main(sys.argv[1])
