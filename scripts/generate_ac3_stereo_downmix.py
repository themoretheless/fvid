"""Owned AC-3 Lt/Rt/LoRo/automatic scalar-matrix witnesses."""
import json
from pathlib import Path
from generate_ac3_alternate_bsi import replace
from generate_ac3_rf_compression import crc_frame
from generate_eac3_playback import BitWriter
from generate_eac3_aht import spectrum,conventional
from generate_eac3_bsid import movie
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'tests/fixtures/playback-errors/ac3-stereo-downmix'

def source_frame(layout):
 nf=[2,1,2,3,3,4,4,5][layout];assert 3<=nf<=4
 w=BitWriter().put(0x0b77,16).zeros(16).put(2,2).put(36,6).put(8,5).zeros(3).put(layout,3)
 if layout&1:w.zeros(2)
 if layout&4:w.zeros(2)
 w.put(0,1).put(31,5).zeros(8)
 for b in range(6):
  w.zeros(2*nf+1).put(int(b==0),1)
  if b==0:w.put(0,1)
  for _ in range(nf):w.put(3 if b==0 else 0,2)
  if b==0:
   w.zeros(6*nf)
   for _ in range(nf):
    w.put(10,4)
    for _ in range(6):w.put(62,7)
    w.zeros(2)
  w.put(int(b==0),1)
  if b==0:w.put(3,2).put(3,2).put(3,2).put(3,2).put(7,3)
  w.put(int(b==0),1)
  if b==0:
   w.put(63,6)
   for _ in range(nf):w.put(15,4).put(4,3)
  w.zeros(2)
  for ch in range(nf):
   for i in range(73):
    v=round(conventional([round(v/16)for v in spectrum(i,ch,'mixed')],b))
    w.put(v&65535,16)
 return crc_frame(w.to_bytes(3840))

def generate():
 raw=(ROOT/'tests/fixtures/audio/ac3-51.ac3').read_bytes()
 families=[('51',[raw[i*1792:(i+1)*1792]for i in range(3)],7,True,6,48000,2560,.595,.5)]
 families +=[(f'layout{layout}',[source_frame(layout)]*3,layout,False,[2,1,2,3,3,4,4,5][layout],32000,3840,.707,.707)for layout in [3,4,5,6]]
 files={};cases=[]
 for family,source,layout,lfe,channels,rate,size,cg,sg in families:
  files[family+'-control.ac3']=b''.join(source)
  pairs=[(c,s)for c in range(8)for s in range(8)]if layout in [5,7]else[(c,c)for c in range(8)]
  for c,s in pairs:
   for bsid in ([6,8]if family=='51'and c==s==0 else[6]):
    # All four preference codes, and an absent-word fallback, across the matrix.
    mixes=[[(c+s)%4,c,s,7-c,7-s],[(c+s+1)%4,(c+3)%8,(s+5)%8,c,s],None]
    packets=[replace(p,bsid,m,None,frame_bytes=size)for p,m in zip(source,mixes)]
    name=f'{family}-b{bsid}-c{c}-s{s}'
    files[name+'.ac3']=b''.join(packets)
    files[name+'.mkv']=movie(packets,channels=channels,rate=rate,codec='A_AC3')
    cases.append(dict(name=name,control=family+'-control.ac3',layout=layout,lfe=lfe,channels=channels,rate=rate,bsid=bsid,mixes=mixes,basic_centre=cg,basic_surround=sg,frame_bytes=size))
 files['manifest.json']=(json.dumps(dict(cases=cases,provenance='Owned A/52 spectra for modes 3/0,2/1,3/1,2/2 plus existing repository-authored 5.1 tone. Independent scalar matrices in tests, authored neutral VP9. No private media, process, external encoder or network.'),indent=2)+'\n').encode()
 return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
