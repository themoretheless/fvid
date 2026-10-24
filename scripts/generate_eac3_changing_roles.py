"""Owned changing dependent-channel maps, continuous transform/transient history."""
import json,struct
from pathlib import Path
from generate_eac3_dependent import packet,COUNTS,SLOTS
from generate_eac3_bsid import movie
from generate_eac3_presentation import ORDER,element,uint,frame
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/eac3-changing-roles'
def video(programmes,blocks,rate):
 v=uint(0xd7,1)+uint(0x73c5,1)+uint(0x83,1)+element(0x86,b'V_VP9')+element(0xe0,uint(0xb0,16)+uint(0xba,16))
 a=uint(0xd7,2)+uint(0x73c5,2)+uint(0x83,2)+element(0x86,b'A_EAC3')+element(0xe1,element(0xb5,struct.pack('>d',rate))+uint(0x9f,8))
 clusters=b''.join(element(0x1f43b675,uint(0xe7,i*blocks*256*1000//rate)+element(0xa3,b'\x81\0\0\x80'+frame(8,'420',16,16))+b''.join(element(0xa3,b'\x82\0\0\x80'+p)for p in parts))for i,parts in enumerate(programmes))
 return element(0x1a45dfa3,element(0x4282,b'matroska'))+element(0x18538067,element(0x1549a966,uint(0x2ad7b1,1000000)+element(0x4489,struct.pack('>d',3*blocks*256*1000/rate)))+element(0x1654ae6b,element(0xae,v)+element(0xae,a))+clusters)
def generate():
 files={};cases=[]
 for rate in [48000,44100]:
  for blocks in [3,6]:
   for transient in [False,True]:
    name=f'{rate}-{blocks}-{int(transient)}';programmes=[[],[],[]];parts=[]
    for i in range(3):
     layout=7 if i==0 else 2;lfe=i==0;nf=COUNTS[layout]
     windows=[None]*nf
     if transient and i:windows[0]=(0,255) if i==1 else (240,127)
     args=dict(allocated=not(i==0 and blocks==6),blocks_override=blocks,divisor=1<<i,rate_code={48000:0,44100:1}[rate])
     control,_=packet(layout,lfe,0,0,'mixed',None,False,**args)
     control_name=f'{name}-stream{i}-control.mkv'
     files[control_name]=movie([control]*3,blocks=blocks,channels=nf+int(lfe),rate=rate)
     roles=[]
     for f in range(3):
      rear=(i==1)==(f!=1)
      mapping=None if i==0 else (0x0200 if rear else 0xa000)
      roles.append(['L','C','R','Ls','Rs','LFE'] if i==0 else (['Lrs','Rrs']if rear else['L','R']))
      raw,_=packet(layout,lfe,max(0,i-1),0,'mixed',mapping,i>0,last=i==2,transient_windows=windows if transient else None,**args)
      programmes[f].append(raw)
     slots=SLOTS[layout]+([3]if lfe else[])
     parts.append(dict(control=control_name,roles=roles,coded_to_native=[sorted(slots).index(s)for s in slots],windows=[None if w is None else dict(location=w[0]*4,length=w[1])for w in windows]+([None]if lfe else[])))
    files[name+'.ec3']=b''.join(p for ps in programmes for p in ps)
    files[name+'.mkv']=video(programmes,blocks,rate)
    speakers=sorted({r for p in parts for roles in p['roles'] for r in roles},key=ORDER.index)
    cases.append(dict(name=name,rate=rate,blocks=blocks,transient=transient,components=parts,speakers=speakers))
    if blocks==6:
     from generate_eac3_mp4_presentation import rewrite
     from generate_eac3_aht_lfe import audiovisual
     from generate_eac3_playback import BitWriter
     bitrate=(sum(map(len,programmes[0]))*8*rate+1535999)//1536000
     w=BitWriter().put(bitrate,13).put(0,3).put({48000:0,44100:1}[rate],2).put(16,5).zeros(5).put(7,3).put(1,1).zeros(3).put(2,4).put(2,9)
     assert len(w.bits)==48
     config=bytes(sum(w.bits[i+j]<<(7-j)for j in range(8))for i in range(0,48,8))
     files[name+'.mp4']=rewrite(audiovisual([b''.join(ps)for ps in programmes]),config,rate)
     cases[-1]['mp4']=name+'.mp4'

 files['manifest.json']=(json.dumps(dict(accepted=True,cases=cases),indent=2)+'\n').encode()
 return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for name,data in generate().items():(OUT/name).write_bytes(data)
