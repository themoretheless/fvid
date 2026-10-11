#!/usr/bin/env python3
"""Original complete VPS metadata NALs, without coded pictures/profile qualification."""
import json
from pathlib import Path
D=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
def ue(v):
 s=f'{v+1:b}';return '0'*(len(s)-1)+s
def packed(s):return int(s+'0'*((-len(s))%8),2).to_bytes((len(s)+7)//8,'big')
def escape(data):
 out=bytearray();zeros=0
 for value in data:
  if zeros==2 and value<=3:out.append(3);zeros=0
  out.append(value);zeros=zeros+1 if value==0 else 0
 return bytes(out)
def source(manifest,file):
 c=next(c for c in json.loads((D/manifest).read_text()) if c['file']==file)
 return ''.join(f'{v:08b}' for v in (D/file).read_bytes())[:c['end']]
plain=source('hevc-dependency-types.json','hevc-dependency-types-both.bin')
rich=source('hevc-layered-vui.json','hevc-layered-vui-rich.bin')
external=source('hevc-layered-vui.json','hevc-layered-vui-external.bin')
bsp=source('hevc-bsp.json','hevc-bsp-basic.bin')
def hrd_rows(rate,size):
 return ''.join('1'+ue(0)+ue(1)+ue(rate-1)+ue(size-1)+'1'+ue(rate+9)+ue(size-2)+'1' for _ in range(3))
common='100'+'0'*8+'0'*15
borrowed=rich[:-1]+'1'+ue(1)+'0'+ue(2)+hrd_rows(30,96)+ue(0)
for tid in range(3):
 borrowed+=ue(1)
 for schedule in range(2):
  for partition in range(4):borrowed+=str(schedule)+ue(0)
cases=[]
for name,payload,vui,has_bsp,internal,future,error in [
 ('plain',plain,False,False,True,False,None),('vui',rich,True,False,True,False,None),
 ('bsp',bsp,True,True,True,False,None),('external',external,True,False,False,False,None),
 ('base-hrd',borrowed,True,True,True,False,None),
 ('ptl-inheritance',plain,False,False,True,False,None),
 ('bad-ptl-profile',plain,False,False,True,False,'absent HEVC general profile'),
 ('future',plain,False,False,True,True,None),
 ('missing-extension','',False,False,True,False,'requires extension syntax'),
 ('bad-alignment',plain,False,False,True,False,'alignment bit'),
 ('bad-stop',plain,False,False,True,False,'RBSP'),
 ('unmarked-tail',plain,False,False,True,False,'RBSP'),
]:
 bits='0000'+str(int(internal))+'1'+f'{3:06b}'+'0101'+'1'*16
 profile=lambda id: f'{id:08b}'+'0'*80
 if name=='ptl-inheritance':
  # Base sublayer0 signals only profile; sublayer1 signals profile and level.
  bits+='00000001'+f'{1<<30:032b}'+'0'*48+f'{120:08b}'+'1011'+'0'*12+profile(2)+profile(3)+f'{30:08b}'
  # PTL1 has no general/sub-layer profiles; it supplies only sublayer0 level.
  payload=f'{90:08b}'+'0100'+'0'*12+f'{60:08b}'+payload[24:]
 else:bits+='00000001'+f'{1<<30:032b}'+'0'*48+f'{120:08b}'+'0'*16
 if name=='bad-ptl-profile':
  # F.7.4.4 does not remove 7.4.4's profilePresentFlag==0 restriction.
  payload=payload[:8]+'1000'+'0'*12+profile(2)+payload[24:]
 bits+='0'+ue(2)+ue(1)+ue(0)
 bits+=f'{15:06b}'+ue(1)+''.join(str(int(bool(32905&(1<<id)))) for id in range(16))
 if has_bsp:
  bits+='1'+f'{1001:032b}{30000:032b}'+'0'+ue(1 if name=='base-hrd' else 0)
  if name=='base-hrd':bits+=ue(0)+common+hrd_rows(10,128)
 else:bits+='0'
 bits+=str(int(name!='missing-extension'))
 if payload:
  alignment=(-len(bits))%8
  if name=='bad-alignment':assert alignment>0
  bits+=('0' if name=='bad-alignment' else '1')*alignment
  bits+=payload+str(int(future))
  if future:bits+='101100001011'
  if name=='unmarked-tail':bits+='01'
 bits+='0' if name=='bad-stop' else '1'
 file='hevc-layered-vps-'+name+'.vps';(D/file).write_bytes(b'\x40\x01'+escape(packed(bits)))
 cases.append(dict(file=file,error=error,vui=vui,bsp=has_bsp,internal=internal,future=future,base_hrd=name=='base-hrd',schemes=1 if name=='base-hrd' else 2,ptl_inheritance=name=='ptl-inheritance'))
(D/'hevc-layered-vps.json').write_text(json.dumps(cases,indent=2)+'\n')

# Short synthetic MP4 reproducers for the profile-loss/illegal-presence cases.
# Only hvcC VPS changes; moov follows mdat, so media offsets remain unchanged.
import struct
from generate_he_aac_packet_fixtures import boxes
from hevc_fixture_mp4 import box
for seed in ('main-ipb','main10-ipb'):
 original=(D.parent/'hevc'/f'{seed}.mp4').read_bytes()
 roots=list(boxes(original));assert roots[-1][0]==b'moov'
 for kind in ('ptl-inheritance','bad-ptl-profile'):
  nal=(D/f'hevc-layered-vps-{kind}.vps').read_bytes()
  def config(data):
   out=bytearray(data[:23]);at=23;changed=0
   for _ in range(data[22]):
    header=data[at:at+3];at+=3;out+=header
    for _ in range(int.from_bytes(header[1:],'big')):
     n=int.from_bytes(data[at:at+2],'big');at+=2;unit=data[at:at+n];at+=n
     if header[0]&63==32:unit=nal;changed+=1
     out+=struct.pack('>H',len(unit))+unit
   assert at==len(data) and changed==1
   return bytes(out)
  def rewrite(tag,payload):
   if tag in (b'moov',b'trak',b'mdia',b'minf',b'stbl'):payload=b''.join(rewrite(t,p) for t,p in boxes(payload))
   elif tag==b'stsd':payload=payload[:8]+b''.join(box(t,p[:78]+b''.join(box(k,config(v) if k==b'hvcC' else v) for k,v in boxes(p[78:]))) for t,p in boxes(payload[8:]))
   return box(tag,payload)
  (D/f'hevc-layered-vps-{seed}-{kind}-synthetic.mp4').write_bytes(b''.join(rewrite(t,p) for t,p in roots))
