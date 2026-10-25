"""Owned alternate-BSI metadata with retained authored AC-3 tone payloads."""
import json
from pathlib import Path
from generate_ac3_rf_compression import crc_frame,authored,mp4_movie
from generate_eac3_bsid import movie
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'tests/fixtures/playback-errors/ac3-alternate-bsi'
def replace(raw,bsid,mix,info,extra=b'',*,frame_bytes=2560):
 bits=''.join(f'{x:08b}'for x in raw);layout=int(bits[48:51],2)
 at=51+2*int(layout&1!=0 and layout!=1)+2*int(layout&4!=0)+2*int(layout==2)+1
 for _ in range(2 if layout==0 else 1):
  at+=5
  for width in [8,8,7]:
   present=int(bits[at]);at+=1+width*present
 at+=2;start=at
 for _ in range(2):
  present=int(bits[at]);at+=1+14*present
 oldadd=at;present=int(bits[at]);at+=1
 if present:
  length=int(bits[at:at+6],2)+1;at+=6+length*8
 meta='0'if mix is None else '1'+''.join(f'{v:0{w}b}'for v,w in zip(mix,[2,3,3,3,3]))
 meta+='0'if info is None else '1'+''.join(f'{v:0{w}b}'for v,w in zip(info,[2,2,1,8,1]))
 meta+=('1'+f'{len(extra)-1:06b}'+''.join(f'{v:08b}'for v in extra))if extra else'0'
 data=bits[:start]+meta+bits[at:-16]
 data=data[:34]+f'{36:06b}{bsid:05b}'+data[45:]
 assert len(data.rstrip('0'))<=frame_bytes*8-16
 data=data[:frame_bytes*8-16].ljust(frame_bytes*8,'0')
 return crc_frame(bytes(int(data[i:i+8],2)for i in range(0,len(data),8)))
def generate():
 files={};cases=[]
 raw=(ROOT/'tests/fixtures/audio/ac3-51.ac3').read_bytes();assert raw[:2]==b'\x0b\x77'and raw[4]>>6==0
 source=[raw[i*1792:(i+1)*1792]for i in range(3)]
 families=[('51',source,7,True,6),('stereo',[authored(2,False,0,None,None,control=True)]*3,2,False,2),('dual',[authored(0,False,0,None,None,control=True)]*3,0,False,2),('mono-lfe',[authored(1,True,0,None,None,control=True)]*3,1,True,2)]
 for family,source,layout,lfe,channels in families:
  files[family+'-control.ac3']=b''.join(source)
  for c,s in ([(c,s)for c in range(8)for s in range(8)]if family=='51'else[(0,7),(7,0)]):
   for bsid in ([6,8]if family=='51'and c==s==0 else[6]):
    mixes=[[(c+s)%4,7-c,7-s,c,s],[(c+s+1)%4,c,s,(c+3)%8,(s+5)%8],None]
    info=[[(c+s)%4,((c+s)//2)%4,1,0,1],None,[0,0,0,0,0]]
    packets=[replace(p,bsid,m,i,b'\x5a\xa5'if j==1 else b'')for j,(p,m,i)in enumerate(zip(source,mixes,info))]
    key=f'{family}-bsid{bsid}-c{c}-s{s}'
    files[key+'.ac3']=b''.join(packets)
    files[key+'.mkv']=movie(packets,channels=channels,codec='A_AC3')
    if family=='51'and (c,s)in[(0,0),(0,7),(7,0),(7,7)]:files[key+'.mp4']=mp4_movie(packets,layout,lfe,0,bsid=bsid)
    cases.append(dict(name=key,control=family+'-control.ac3',layout=layout,lfe=lfe,channels=channels,bsid=bsid,mixes=mixes,info=info,frames=[1280,1536,1536],basic_centre=0.595 if layout==7 else 0.707,basic_surround=0.5 if layout==7 else 0.707,mp4=key+'.mp4'in files))
 files['manifest.json']=(json.dumps(dict(cases=cases,provenance='Repository-authored short 5.1 synthetic tone and authored mono/stereo/dual spectra, BSI rewritten with both CRC regions repaired. Neutral VP9 companions. No private data, external process, encoder or network.'),indent=2)+'\n').encode()
 return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
