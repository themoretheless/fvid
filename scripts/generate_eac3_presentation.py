"""Owned replacement/extension, common DRC and final-clamp witnesses."""
import json,math
from pathlib import Path
from generate_eac3_dependent import packet,COUNTS,SLOTS
from generate_eac3_aht import spectrum,conventional
from generate_ffv1_gray_fixtures import element,uint
from generate_vp9_chroma import frame
import struct
from generate_eac3_bsid import movie
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/eac3-presentation'
# Ordering independently declares expected WAVE positions then extended roles.
ORDER=['L','R','C','LFE','Lrs','Rrs','Lc','Rc','Cs','Ls','Rs','Ts','Vhl','Vhc','Vhr','Lts','Rts','Lsd','Rsd','Lw','Rw','LFE2']
BASE={2:['L','R'],7:['L','C','R','Ls','Rs','LFE']}
PLANS=[('7.1',7,True,3,[(2,0x0200,['Lrs','Rrs'])],0x63f),
 ('7.1-replace',7,True,3,[(3,0xe000,['L','C','R']),(2,0x0200,['Lrs','Rrs'])],0x63f),
 ('10',7,True,3,[(2,0x0200,['Lrs','Rrs']),(2,0x0010,['Vhl','Vhr'])],0x563f),
 ('16',7,True,3,[(6,0x0600,['Lc','Rc','Lrs','Rrs']),(3,0x0018,['Vhl','Vhr','Vhc']),(3,0x00a0,['Ts','Lw','Rw'])],None)]

def split_movie(parts,blocks,channels,rate=48000,*,truncate_last=False):
 video=uint(0xd7,1)+uint(0x73c5,1)+uint(0x83,1)+element(0x86,b'V_VP9')+element(0xe0,uint(0xb0,16)+uint(0xba,16))
 audio=uint(0xd7,2)+uint(0x73c5,2)+uint(0x83,2)+element(0x86,b'A_EAC3')+element(0xe1,element(0xb5,struct.pack('>d',rate))+uint(0x9f,channels))
 vp9=frame(8,'420',16,16)
 clusters=b''.join(element(0x1f43b675,uint(0xe7,i*blocks*256*1000//rate)+element(0xa3,b'\x81\0\0\x80'+vp9)+b''.join(element(0xa3,b'\x82\0\0\x80'+(p[:-1]if truncate_last and i==2 and j==len(parts)-1 else p))for j,p in enumerate(parts)))for i in range(3))
 return element(0x1a45dfa3,element(0x4282,b'matroska'))+element(0x18538067,element(0x1549a966,uint(0x2ad7b1,1000000)+element(0x4489,struct.pack('>d',3*blocks*256*1000/rate)))+element(0x1654ae6b,element(0xae,video)+element(0xae,audio))+clusters)

def generate(*, expanded_aht=False, expanded_aht_base51=False, coupled=False):
 files={};cases=[]
 def add(plan,mask,basis,mode,exponent=10,rate=48000,coupling=None,enhanced=None,spx=None):
  label,base_layout,lfe,blocks,deps,wave=plan
  codes={'neutral':[None]*blocks,'unity':[None]*blocks,'vary':([0,None,224,None,32,None])[:blocks],'boost':([64,None,192,None,1,None])[:blocks],'clip':[160]+[None]*(blocks-1)}[mode]
  powers=[1,2,4,8,1/16,1/8,1/4,1/2];gain=1;gains=[]
  for code in codes:
   if code is not None:gain=powers[code>>5]*(32+(code&31))/32
   gains.append(gain)
  name=f'{label}-m{mask}-{basis}-{mode}'+(f'-rate{rate}'if rate!=48000 else'');parts=[];components=[]
  entries=[(base_layout,lfe,None,BASE[base_layout])]+[(layout,False,m,roles)for layout,m,roles in deps]
  for i,(layout,has_lfe,m,roles) in enumerate(entries):
   dependent=i>0;divisor=1<<i;last=i==len(entries)-1
   raw,_=packet(layout,has_lfe,max(0,i-1),mask if dependent and blocks==6 else 0,basis,m,dependent,last=last,blocks_override=blocks,gains=(codes if last else ([96]+[None]*(blocks-1)if i==0 and mode!='neutral' else None)),divisor=divisor,exponent=exponent,rate_code={48000:0,44100:1}[rate],allocated=not(expanded_aht_base51 and i==0),block_allocation=([True,False,True,False,True,False] if expanded_aht_base51=="pulsed" and i==0 else None))
   control,_=packet(layout,has_lfe,0,0,basis,None,False,blocks_override=blocks,divisor=divisor,rate_code={48000:0,44100:1}[rate],allocated=not(expanded_aht_base51 and i==0),block_allocation=([True,False,True,False,True,False] if expanded_aht_base51=="pulsed" and i==0 else None))
   if coupling is not None and dependent:
    from generate_eac3_aht_coupling import packet as coupled_packet
    begin,phase=coupling
    raw=coupled_packet(mask,False,basis,begin,phase,dependent=True,channel_map=m,gains=codes)
    control=coupled_packet(0,False,basis,begin,phase,aht=False,refresh=True)
   if enhanced is not None and dependent:
    from generate_eac3_enhanced_coupling import packet as enhanced_packet
    begin,seed,merged=enhanced
    raw=enhanced_packet(False,begin,seed,merged,True,basis,dependent=True,channel_map=m,gains=codes)
    control=enhanced_packet(False,begin,seed,merged,True,basis,control=True)
   if spx is not None and dependent:
    from generate_eac3_playback import build_spx_atten_frame
    attenuation,blend=spx
    raw=build_spx_atten_frame(attenuation,blend=blend,dependent=True,channel_map=m,gains=codes)
    control=build_spx_atten_frame(attenuation,blend=blend)
   key=f'{name}-stream{i}';files[key+'.ec3']=raw;files[key+'-control.ec3']=control*3;files[key+'-control.mkv']=movie([control]*3,blocks=blocks,channels=COUNTS[layout]+int(has_lfe),rate=rate)
   slots=SLOTS[layout]+([3]if has_lfe else[]);permutation=[sorted(slots).index(s)for s in slots]
   components.append(dict(raw=key+'.ec3',control=key+'-control.ec3',control_video=key+'-control.mkv',roles=roles,coded_to_native=permutation))
   parts.append(raw)
  bundle=b''.join(parts);speakers=sorted({r for c in components for r in c['roles']},key=ORDER.index)
  files[name+'.ec3']=bundle*3;files[name+'.mkv']=split_movie(parts,blocks,len(speakers),rate)
  l1=max(sum(abs(conventional([round(v/(1<<i))for v in spectrum(k,ch,basis)],b)-round(conventional([round(v/(1<<i))for v in spectrum(k,ch,basis)],b)))for k in range(73))for i in range(1,len(entries))for ch in range(COUNTS[entries[i][0]])for b in range(blocks))if mask else 0
  if coupling is not None:
   begin,_=coupling;start=37+12*begin;stop=73 if begin==0 else 85
   def rounding(plane,lo,hi,b):return sum(abs(conventional(spectrum(k,plane,basis),b)-round(conventional(spectrum(k,plane,basis),b)))for k in range(lo,hi))
   l1=max(rounding(ch,0,start,b)+.5*rounding(2,start,stop,b)for ch in range(2)for b in range(blocks))
  bound=4*l1/(32768*1024)*max(gains) if mask else 0
  if enhanced is not None:
   from generate_eac3_enhanced_coupling import geometry
   _,_,start,stop=geometry(enhanced[0]);bound=4*(stop-start)/(32768*1024)*max(gains)
  cases.append(dict(rate=rate,bound=bound,name=name,file=name+'.ec3',video=name+'.mkv',channels=len(speakers),blocks=blocks,speakers=speakers,wave_mask=wave,components=components,gains=gains,scale=2**(10-exponent)))
 if coupled=='spx':
  for label,attenuation in [('none',None),('notch',[5,12]),('edge',[15,31])]:
   for blend in [31,0,8,16,24]:
    plan=(f'7.1-spx-{label}-blend{blend}',7,True,6,[(2,0x0200,['Lrs','Rrs'])],0x63f)
    for mode in ['unity','vary']:add(plan,0,None,mode,spx=(attenuation,blend))
  files['manifest.json']=(json.dumps(dict(cases=cases),indent=2)+'\n').encode()
  return files
 if coupled=='enhanced':
  for begin in [0,4,8]:
   for seed in [0,31]:
    for merged in [False,True]:
     plan=(f'7.1-aht-ecpl-b{begin}-s{seed}-g{int(merged)}',7,True,6,[(2,0x0200,['Lrs','Rrs'])],0x63f)
     for basis in [3,'mixed']:
      for mode in ['unity','vary']:add(plan,4,basis,mode,enhanced=(begin,seed,merged))
  files['manifest.json']=(json.dumps(dict(cases=cases),indent=2)+'\n').encode()
  return files
 if coupled:
  for begin in [0,2]:
   for phase in [False,True]:
    plan=(f'7.1-aht-cpl-b{begin}-p{int(phase)}',7,True,6,[(2,0x0200,['Lrs','Rrs'])],0x63f)
    for mask in [4,5,7]:
     for basis in [None,'mixed']:
      for mode in ['unity','vary']:add(plan,mask,basis,mode,coupling=(begin,phase))
  files['manifest.json']=(json.dumps(dict(cases=cases),indent=2)+'\n').encode()
  return files
 if expanded_aht_base51:
  plan=('7.1-aht-base51pulse' if expanded_aht_base51=='pulsed' else '7.1-aht-base51zero',7,True,6,[(2,0x0200,['Lrs','Rrs'])],0x63f)
  for mask in [1,2,3]:
   for basis in [None,'mixed']:
    for mode in ['unity','vary','boost']:add(plan,mask,basis,mode)
  files['manifest.json']=(json.dumps(dict(cases=cases),indent=2)+'\n').encode()
  return files
 if expanded_aht:
  plan=('8-aht',2,False,6,[(2,0x4001,['C','LFE']),(2,0x0200,['Lrs','Rrs']),(2,0x1800,['Ls','Rs'])],0x63f)
  for mask in [1,2,3]:
   for basis in [None,'mixed']:
    for mode in ['unity','vary','boost']:add(plan,mask,basis,mode)
  files['manifest.json']=(json.dumps(dict(cases=cases),indent=2)+'\n').encode()
  return files
 for plan in PLANS:
  for mask in [0]:
   for basis in [None,'mixed']:
    for mode in ['unity','vary','boost']:add(plan,mask,basis,mode)
 add(PLANS[0],0,None,'clip',1)
 add(PLANS[0],0,None,'neutral')
 add(PLANS[0],0,None,'neutral',rate=44100)
 first=cases[0];parts=[files[c['raw']]for c in first['components']]
 files['truncated-expanded-tail.ec3']=files[first['file']][:-1]
 files['truncated-expanded-tail.mkv']=split_movie(parts,first['blocks'],first['channels'],truncate_last=True)
 files['manifest.json']=(json.dumps(dict(cases=cases),indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
