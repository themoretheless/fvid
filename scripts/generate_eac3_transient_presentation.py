"""Owned expanded E-AC-3 transient witnesses; no external codec/network.

Per-component controls remain unity-gain and use exponent 10 so they never
clip before the independent test oracle applies scale/correction/common DRC.
"""
import json
from pathlib import Path
from generate_eac3_dependent import packet, COUNTS, SLOTS
from generate_eac3_presentation import PLANS, BASE, ORDER, split_movie
from generate_eac3_bsid import movie
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/eac3-transient-presentation'
def add_replacement_gap(files,cases):
 # The last replacement substream disappears for one programme, then returns.
 # Its transform state follows the pre-existing pause behaviour; its transient
 # history advances over the missing programme as zero, non-presented PCM.
 import struct
 from generate_eac3_presentation import element,uint,frame
 name='7.1-replacement-gap';rate=48000;blocks=3;width=8
 entries=[(7,True,None,BASE[7]),(2,False,0x0200,['Lrs','Rrs']),(3,False,0xe000,['L','C','R'])]
 components=[];programmes=[[],[],[]]
 for i,(layout,lfe,m,roles) in enumerate(entries):
  active=[0,2] if i==2 else [0,1,2];nf=COUNTS[layout]
  windows=[None]*nf;windows[0]=(300 if i==0 else 240,63 if i==0 else 127)
  if i>0:windows[-1]=(400,31)
  args=dict(blocks_override=blocks,divisor=1<<i)
  control,_=packet(layout,lfe,0,0,'mixed',None,False,**args)
  key=f'{name}-stream{i}'
  files[key+'-control.ec3']=control*len(active)
  files[key+'-control.mkv']=movie([control]*len(active),blocks=blocks,channels=nf+int(lfe),rate=rate)
  canonical=None
  for f in active:
   last=i==(1 if f==1 else 2)
   raw,_=packet(layout,lfe,max(0,i-1),0,'mixed',m,i>0,last=last,gains=None if last else ([96,None,None] if i==0 else None),transient_windows=windows,**args)
   programmes[f].append(raw)
   if canonical is None:canonical=raw
  files[key+'.ec3']=canonical
  slots=SLOTS[layout]+([3]if lfe else[])
  components.append(dict(raw=key+'.ec3',control_video=key+'-control.mkv',roles=roles,coded_to_native=[sorted(slots).index(s)for s in slots],active_frames=active,windows=[None if w is None else dict(location=w[0]*4,length=w[1])for w in windows]+([None]if lfe else[])))
 speakers=sorted({r for c in components for r in c['roles']},key=ORDER.index)
 files[name+'.ec3']=b''.join(p for programme in programmes for p in programme)
 video=uint(0xd7,1)+uint(0x73c5,1)+uint(0x83,1)+element(0x86,b'V_VP9')+element(0xe0,uint(0xb0,16)+uint(0xba,16))
 audio=uint(0xd7,2)+uint(0x73c5,2)+uint(0x83,2)+element(0x86,b'A_EAC3')+element(0xe1,element(0xb5,struct.pack('>d',rate))+uint(0x9f,width))
 vp9=frame(8,'420',16,16)
 clusters=b''.join(element(0x1f43b675,uint(0xe7,i*16)+element(0xa3,b'\x81\0\0\x80'+vp9)+b''.join(element(0xa3,b'\x82\0\0\x80'+raw)for raw in programme))for i,programme in enumerate(programmes))
 files[name+'.mkv']=element(0x1a45dfa3,element(0x4282,b'matroska'))+element(0x18538067,element(0x1549a966,uint(0x2ad7b1,1000000)+element(0x4489,struct.pack('>d',48)))+element(0x1654ae6b,element(0xae,video)+element(0xae,audio))+clusters)
 cases.append(dict(name=name,file=name+'.ec3',video=name+'.mkv',rate=rate,blocks=blocks,channels=width,wave_mask=0x63f,speakers=speakers,components=components,gains=[1,1,1],scale=1,mode='unity'))

def generate():
 files={};cases=[]
 for label,base_layout,lfe,blocks,deps,wave in PLANS:
  for rate in [48000,44100]:
   for mode in ['unity','vary','clip']:
    codes={'unity':None,'vary':[64,None,224],'clip':[96,None,None]}[mode]
    gains={'unity':[1,1,1],'vary':[4,4,.5],'clip':[8,8,8]}[mode]
    exponent=0 if mode=='clip' else 10
    name=f'{label}-{rate}-{mode}';parts=[];components=[]
    entries=[(base_layout,lfe,None,BASE[base_layout])]+[(layout,False,m,roles)for layout,m,roles in deps]
    for i,(layout,has_lfe,m,roles) in enumerate(entries):
     nf=COUNTS[layout];last=i==len(entries)-1
     windows=[None]*nf
     windows[0]=(300 if i==0 else 240,63 if i==0 else 127)
     if nf>1 and i>0:windows[-1]=(0,255) if mode=='vary' else (400,31)
     args=dict(blocks_override=blocks,divisor=1<<i,rate_code={48000:0,44100:1}[rate])
     raw,_=packet(layout,has_lfe,max(0,i-1),0,'mixed',m,i>0,last=last,exponent=exponent,gains=codes if last else ([96,None,None] if i==0 else None),transient_windows=windows,**args)
     control,_=packet(layout,has_lfe,0,0,'mixed',None,False,**args)
     key=f'{name}-stream{i}'
     files[key+'.ec3']=raw;files[key+'-control.ec3']=control*3
     files[key+'-control.mkv']=movie([control]*3,blocks=blocks,channels=nf+int(has_lfe),rate=rate)
     slots=SLOTS[layout]+([3]if has_lfe else[])
     components.append(dict(raw=key+'.ec3',control_video=key+'-control.mkv',roles=roles,coded_to_native=[sorted(slots).index(s)for s in slots],windows=[None if w is None else dict(location=w[0]*4,length=w[1])for w in windows]+([None]if has_lfe else[])))
     parts.append(raw)
    speakers=sorted({r for c in components for r in c['roles']},key=ORDER.index)
    files[name+'.ec3']=b''.join(parts)*3
    files[name+'.mkv']=split_movie(parts,blocks,len(speakers),rate)
    cases.append(dict(name=name,file=name+'.ec3',video=name+'.mkv',rate=rate,blocks=blocks,channels=len(speakers),wave_mask=wave,speakers=speakers,components=components,gains=gains,scale=2**(10-exponent),mode=mode))
 add_replacement_gap(files,cases)
 files['manifest.json']=(json.dumps(dict(cases=cases,expanded_transient_accepted=True),indent=2)+'\n').encode()
 return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
