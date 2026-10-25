"""Owned dependent-channel reconstruction and coded-order witnesses."""
import json
from pathlib import Path
from generate_eac3_playback import BitWriter,patch_crc
from generate_eac3_aht import spectrum,conventional
from generate_eac3_bsid import movie
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/eac3-dependent'
COUNTS=[2,1,2,3,3,4,4,5]
# Coded channels' positions in native WAVE PCM from the independent control.
SLOTS=[[0,1],[2],[0,1],[0,2,1],[0,1,4],[0,2,1,4],[0,1,4,5],[0,2,1,4,5]]
MAPS=[0xa000,0x4000,0xa000,0xe000,0xa100,0xe100,0xb800,0xf800]

def packet(layout,lfe,identity,mask,basis,channel_map,dependent,*,last=True,blocks_override=None,gains=None,divisor=1,exponent=10,rate_code=0,allocated=True,block_allocation=None,transient_windows=None,lfe_mix_code=None,mixing_metadata=False,silent_planes=(),gains_second=None,compression=None,compression_second=None,compact=False,stereo_mix=None):
 nf=COUNTS[layout];blocks=blocks_override or (6 if nf<=2 else 3)
 if block_allocation is not None:
  assert len(block_allocation)==blocks and not mask, 'block SNR witnesses require conventional planes'
 w=BitWriter().put(0x0b77,16).put(int(dependent),2).put(identity if dependent else 0,3).put(2047,11)
 present=(dependent and last) or compression is not None
 w.put(rate_code,2).put(3 if blocks==6 else 2,2).put(layout,3).put(int(lfe),1).put(16,5).put(31,5).put(int(present),1)
 if present:w.put(0 if compression is None else compression,8)
 if layout==0:
  w.put(31,5).put(int(compression_second is not None),1)
  if compression_second is not None:w.put(compression_second,8)
 if dependent:
  w.put(int(channel_map is not None),1)
  if channel_map is not None:w.put(channel_map,16)
 mix=mixing_metadata or lfe_mix_code is not None or stereo_mix is not None
 if lfe_mix_code is not None:assert lfe and 0<=lfe_mix_code<=31
 w.put(int(mix),1)
 if mix:
  pref,lc,ls,rc,rs=stereo_mix if stereo_mix is not None else (0,4,4,4,4)
  assert 0<=pref<4 and all(0<=v<8 for v in [lc,ls,rc,rs])
  if layout>2:w.put(pref,2)
  if layout>2 and layout&1:w.put(lc,3).put(rc,3)
  if layout&4:w.put(ls,3).put(rs,3)
  if lfe:
   w.put(int(lfe_mix_code is not None),1)
   if lfe_mix_code is not None:w.put(lfe_mix_code,5)
  if not dependent:
   w.zeros(1+int(layout==0)).put(0,1).zeros(2)
   if layout<2:w.zeros(1+int(layout==0))
   w.put(0,1)
 w.put(0,1) # infomdate
 if not dependent and blocks!=6:w.put(0,1)
 w.put(0,1) # addbsi
 if blocks==6:w.put(1,1).put(int(dependent),1)
 w.put(1 if block_allocation is not None else 0,2)
 if allocated and block_allocation is None:w.put(int(transient_windows is not None),1).zeros(7)
 else:w.put(int(transient_windows is not None),1).put(0,1).put(1,1).zeros(5) # explicitly disable dither in every block
 if layout>1:w.zeros(blocks)
 for b in range(blocks):
  for _ in range(nf):w.put(3 if b==0 else 0,2)
 if lfe:w.put(1,1).zeros(blocks-1)
 if not dependent:
  if blocks==6:w.zeros(5*nf)
  else:w.put(0,1)
 if blocks==6 and dependent:
  for ch in range(nf+int(lfe)):w.put((mask>>ch)&1,1)
 if block_allocation is None:w.put(63 if allocated else 0,6).put(15 if allocated else 0,4)
 if transient_windows is not None:
  assert len(transient_windows)==nf
  for window in transient_windows:
   w.put(int(window is not None),1)
   if window is not None:w.put(window[0],10).put(window[1],8)
 w.put(0,1)
 for b in range(blocks):
  if not allocated or block_allocation is not None:w.zeros(nf)
  code=None if gains is None else gains[b]
  for programme in range(2 if layout==0 else 1):
   word=gains_second[b] if programme==1 and gains_second is not None else code
   w.put(int(word is not None),1)
   if word is not None:w.put(word,8)
  w.put(0,1) # SPX
  if b==0:
   if layout==2:w.zeros(4)
   w.zeros(6*nf)
   for _ in range(nf):
    w.put(exponent,4)
    for _ in range(6):w.put(62,7)
    w.zeros(2)
   if lfe:w.put(exponent,4).put(62,7).put(62,7)
  elif layout==2:w.put(0,1)
  if block_allocation is not None:
   if b:w.put(1,1)
   w.put(63 if block_allocation[b] else 0,6).put(15 if block_allocation[b] else 0,4)
  if not dependent:w.put(0,1)
  for ch in range(nf+int(lfe)):
   aht=dependent and mask&(1<<ch)
   if aht:
    if b:continue
    w.put(0,2)
   for i in range(7 if ch==nf else 73):
    values=[round(v/divisor)for v in spectrum(i,ch,basis)]
    if ch in silent_planes:values=[0]*6
    active_allocation=block_allocation[b] if block_allocation is not None else allocated
    if active_allocation:
     if aht:
      for q in values:w.put(q&65535,16)
     else:w.put(round(conventional(values,b))&65535,16)
 if compact:
  size=((len(w.bits)+64+15)//16)*2
  assert size<=4096
  raw=bytearray(w.to_bytes(size));words=size//2-1
  raw[2]=(raw[2]&0xf8)|(words>>8);raw[3]=words&255
  return patch_crc(raw),blocks
 return patch_crc(w.to_bytes(4096)),blocks

def generate():
 files={};cases=[]
 for layout,nf in enumerate(COUNTS):
  for lfe in [False,True]:
   slots=SLOTS[layout]+([3]if lfe else[]);order=[sorted(slots).index(s)for s in slots]
   for identity in [0,7]:
    for mapped in [False,True]:
     channel_map=(MAPS[layout]|int(lfe))if mapped else None
     for mask in (range(1<<(nf+int(lfe)))if nf<=2 else[0]):
      for basis in [None,3,'mixed']:
       name=f'l{layout}-f{int(lfe)}-i{identity}-c{int(mapped)}-m{mask}-{basis}'
       raw,blocks=packet(layout,lfe,identity,mask,basis,channel_map,True)
       control,_=packet(layout,lfe,0,0,basis,None,False)
       l1=max(sum(abs(conventional(spectrum(i,ch,basis),b)-round(conventional(spectrum(i,ch,basis),b)))for i in range(7 if ch==nf else 73))for ch in range(nf+int(lfe))for b in range(blocks))
       files[name+'.ec3']=raw*3;files[name+'-control.ec3']=control*3
       # Full valid substream ordering: dependent ID 7 is extracted from eight
       # dependent streams following the independent programme.
       dependencies=[packet(layout,lfe,j,mask,basis,channel_map,True,last=j==identity)[0]for j in range(identity+1)]
       files[name+'.mkv']=movie([control+b''.join(dependencies)]*3,blocks=blocks,channels=nf+int(lfe))
       files[name+'-control.mkv']=movie([control]*3,blocks=blocks,channels=nf+int(lfe))
       cases.append(dict(name=name,file=name+'.ec3',control=name+'-control.ec3',video=name+'.mkv',control_video=name+'-control.mkv',layout=layout,lfe=lfe,identity=identity,channel_map=channel_map,channels=nf+int(lfe),blocks=blocks,mask=mask,order=order,quantization_bound=4*l1/(32768*1024),exact=basis in [None,3]))
 for bit in range(16):
  pair=bit in [5,6,9,10,11,13];layout=2 if pair else 1;nf=COUNTS[layout]
  channel_map=1<<(15-bit);name=f'location-{bit}'
  raw,blocks=packet(layout,False,0,(1<<nf)-1,3,channel_map,True)
  control,_=packet(layout,False,0,0,3,None,False)
  files[name+'.ec3']=raw*3;files[name+'-control.ec3']=control*3
  files[name+'.mkv']=movie([control+raw]*3,channels=nf)
  files[name+'-control.mkv']=movie([control]*3,channels=nf)
  cases.append(dict(name=name,file=name+'.ec3',control=name+'-control.ec3',video=name+'.mkv',control_video=name+'-control.mkv',layout=layout,lfe=False,identity=0,channel_map=channel_map,channels=nf,blocks=blocks,mask=(1<<nf)-1,order=list(range(nf)),quantization_bound=0,exact=True))
 bad,_=packet(2,False,0,0,None,0x8000,True)
 base,_=packet(2,False,0,0,None,None,False)
 files['bad-map-count.ec3']=bad
 files['bad-map-count.mkv']=movie([base+bad]*3)
 files['manifest.json']=(json.dumps(dict(cases=cases),indent=2)+'\n').encode()
 return files

if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for name,data in generate().items():(OUT/name).write_bytes(data)
