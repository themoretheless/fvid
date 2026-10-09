#!/usr/bin/env python3
"""Own CCE phase videos and direct scalar float PCM oracle; no foreign codec."""
import json,math,struct
from generate_aac_main_tools_fixtures import channel,ics
from generate_aac_ssr_fixtures import windows
from generate_aac_ltp_coupling_fixtures import program
from generate_he_aac_packet_fixtures import DEST,field,packed,video_fixture
N=1024
GAINS=[.570829,.696616,.813004,.911304,.984900,1.067894,1.194601,1.369533]
COEF=math.sin(math.pi/7)
COS=[[math.cos(math.pi/N*(i+.5+N/2)*(k+.5)) for i in range(2*N)] for k in range(8)]
KBD=windows(N,4)
def f32(x):return struct.unpack('<f',struct.pack('<f',x))[0]
def window(i,shape):return KBD[i] if shape else math.sin(math.pi*(i+.5)/(2*N))
class Oracle:
 def __init__(self):self.history=[0.]*(4*N);self.overlap=[0.]*N;self.previous=0
 def prepare(self,source,data):
  spectrum=[v*1024. for v in source]
  if data['active']:
   lag=data['lag'];gain=GAINS[data['coefficient']]
   estimate=[v*gain*window(i,self.previous if i<N else data['shape']) for i,v in enumerate(self.history[2*N-lag:4*N-lag])]
   prediction=[sum(a*b for a,b in zip(estimate,COS[k])) for k in range(8)]
   order=list(range(8));
   if data['reverse']:order.reverse()
   previous=0.
   for i in order:original=prediction[i];prediction[i]=original+COEF*previous;previous=original
   spectrum=[f32(v+prediction[k]) if data['used'][k//4] else v for k,v in enumerate(spectrum)]
  return self.tns(spectrum,data)
 def tns(self,spectrum,data):
  out=spectrum.copy();order=list(range(8));
  if data['reverse']:order.reverse()
  previous=0.
  for i in order:value=spectrum[i]-COEF*previous;out[i]=f32(value);previous=value
  return out
 def synthesize(self,spectrum,data):
  weighted=[2/N*sum(spectrum[k]*COS[k][i] for k in range(8))*window(i,self.previous if i<N else data['shape']) for i in range(2*N)]
  raw=[self.overlap[i]+weighted[i] for i in range(N)];self.overlap=weighted[N:]
  self.history=self.history[N:2*N]+raw+self.overlap+[0.]*N;self.previous=data['shape']
  return [v/65536 for v in raw]
def metadata(frame,role,point):
 return dict(active=frame>=3 and (frame%4!=2 if role==0 else point==3 and frame%3!=1),lag=N-role*17-point*5,coefficient=(frame+role*3+point)%8,used=[True,(frame+role)%2==0],shape=(frame+role+point)%2 if point==3 else (frame+point)%2,reverse=bool((frame+role+point)%2))
def info(data):
 base=ics(0,2,False,shape=data['shape'])
 return base[:-1]+'11'+field(data['lag'],11)+field(data['coefficient'],3)+''.join(str(int(v)) for v in data['used']) if data['active'] else base

def main():
 blob=bytearray();gold=bytearray();control_gold=bytearray();control_blob=bytearray();cases=[]
 for point in (0,1,3):
  target=Oracle();source=Oracle();control=Oracle();rows=[];gold_start=len(gold)
  for frame in range(12):
   td=metadata(frame,0,point);sd=metadata(frame,1,point)
   tq=[1,-1,1,0,0,1,-1,1];sq=[(-1 if (frame+k)%3==0 else 1 if (frame+k)%3==1 else 0) for k in range(8)]
   prepared=source.prepare(sq,sd)
   # For point zero the source is already TNS-filtered before target mixing.
   if point==0:
    residual=[f32(v*1024.+s) for v,s in zip(tq,prepared)]
    # prepare accepts quantized input; the exact f32 residual is represented in raw units.
    target_spectrum=target.prepare([v/1024. for v in residual],td)
   else:target_spectrum=target.prepare(tq,td)
   if point==1:target_spectrum=[f32(a+b) for a,b in zip(target_spectrum,prepared)]
   pcm=target.synthesize(target_spectrum,td)
   if point==3:pcm=[f32(f32(a)+f32(b)) for a,b in zip(pcm,source.synthesize(prepared,sd))]
   reference=len(gold);gold.extend(struct.pack('<'+str(N)+'f',*pcm))
   audio='0000000'+channel(0,[1,1],[tq],info=info(td),tns=(td['reverse'],1))
   control_reference=len(control_gold);control_gold.extend(struct.pack('<'+str(N)+'f',*control.synthesize(control.prepare(tq,td),td)))
   control_packet=packed(audio+'111');control_offset=len(control_blob);control_blob.extend(control_packet)
   cce='010'+field(1,4)+field(point==3,1)+'000'+'0'+'0000'+field(point==1,1)+'0'+'10'+channel(0,[1,1],[sq],info=info(sd),tns=(sd['reverse'],1))
   packet=packed((cce+audio if frame%2 else audio+cce)+'111')
   rows.append(dict(offset=len(blob),bytes=len(packet),target=td,source=sd,target_quantized=tq,source_quantized=sq,reference_offset=reference,control_reference_offset=control_reference,control_offset=control_offset,control_bytes=len(control_packet)));blob.extend(packet)
  case=dict(name=str(point),point=point,channels=1,asc=program(1,point).hex(),frames=rows,container_rate=24000,container_frame_samples=1024,slots=16,bands=32,samples=12288,pcm_offset=gold_start)
  case['control_video']=video_fixture([dict(case,frames=[dict(row,offset=row['control_offset'],bytes=row['control_bytes']) for row in rows])],control_blob,channels=1,filename=f'aac-ltp-phase-target-only-{point}-synthetic.mp4')
  case['video']=video_fixture([case],blob,channels=1,filename=f'aac-ltp-phase-{point}-synthetic.mp4');cases.append(case)
 # Own independent CCE with an absent target: synthesis succeeds before routing fails.
 bad_case=cases[2];row=bad_case['frames'][1]
 bad=bytearray(blob[row['offset']:row['offset']+row['bytes']]);bad[1]=(bad[1]&0xf0)|1
 (DEST/'aac-ltp-phase-absent-target.bin').write_bytes(bad)
 video_fixture([dict(bad_case,frames=[dict(row,offset=0,bytes=len(bad))])],bad,channels=1,filename='aac-ltp-phase-absent-target-synthetic.mp4')
 (DEST/'aac-ltp-phase-control-packets.bin').write_bytes(control_blob)
 (DEST/'aac-ltp-phase-control-reference.f32le').write_bytes(control_gold)
 (DEST/'aac-ltp-phase-packets.bin').write_bytes(blob)
 (DEST/'aac-ltp-phase-reference.f32le').write_bytes(gold)
 (DEST/'aac-ltp-phase.json').write_text(json.dumps(dict(cases=cases,provenance='Own mono CCE point0/1/3 with target LTP, independent-source LTP, directional source/target TNS and sine/KBD; direct sparse MDCT/IMDCT, scalar FIR/AR and float history. Staged, native and public MP4 PCM acceptance; broader profile combinations remain separate.'),indent=2)+'\n')
 print('generated three coupling-phase and three target-only control videos')
if __name__=='__main__':main()
