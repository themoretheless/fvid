"""Own LD-MPS configuration grammar corpus; offline, no foreign bitstreams.
Movie variants qualify configuration parsing and renderer refusal only.
"""
import json,struct
from pathlib import Path
from generate_he_aac_packet_fixtures import field,packed,frequency,video_fixture
from generate_aac_eld_fixtures import generate as eld
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/aac-ld-mps-config'
RATES=[96000,88200,64000,48000,44100,32000,24000,22050,16000,12000,11025,8000,7350]
BANDS=[0,23,15,12,9,7,5,4]
TREES=[(1,6,[0,0,0,0,1],0),(1,6,[0,0,1,0,0],0),(2,6,[1,0,0],1),(2,8,[1,0,0,0,0],1),(2,8,[1,0,0,0,0],1),(6,8,[0,0],0),(6,8,[0,0],0),(1,2,[0],0)]
def length(n):
 return field(n,4)if n<15 else field(15,4)+field(n-15,8)if n<270 else field(15,4)+field(255,8)+field(n-270,16)
def config(tree,res,v,movie=False):
 inp,out,lfe,nt=TREES[tree];bands=BANDS[res];idx=3 if movie else (tree*7+res+v)%14;rate=RATES[idx]if idx<13 else 27713
 frame=15 if movie else [0,14,15,29,30,31][v];quant=v%3;one=v%2 if tree!=7 else 0;raw_dmx=v%2;sur=v%5 if tree!=7 else 0;lf=(v+2)%5 if tree!=7 else 0;gain=(tree+res+v)%8;matrix=tree%2 if tree!=7 else 0;shape=v%3;dec=(tree+v)%3;bin_mode=(tree!=7 and v in [1,2,5]);hres=1+(tree+v)%7;asym=v%2
 wire=field(idx if idx<13 else 15,4)+(field(rate,24)if idx==13 else'')+field(frame,5)+field(res,3)+field(tree,4)+field(quant,2)
 if tree!=7:wire+=field(one,1)
 wire+=field(raw_dmx,1)
 if tree!=7:wire+=field(sur,3)+field(lf,3)
 wire+=field(gain,3)
 if tree!=7:wire+=field(matrix,1)
 wire+=field(shape,2)+field(dec,2)
 if tree!=7:wire+=field(bin_mode,1)
 ott=[bands if x else 0 for x in lfe]
 for x in lfe:
  if x:wire+=field(bands,5)
 ttt=[]
 for i in range(nt):
  dual=v%2;lo=v;hi=(v+1)%6;split=bands//2;wire+=field(dual,1)+field(lo,3)+(field(hi,3)+field(split,5)if dual else'');ttt.append([dual,lo,hi if dual else 0,split if dual else 0])
 if shape==2:wire+='0'
 hrtf=[]
 if bin_mode:
  hb=BANDS[hres];wire+='00'+field(hres,3)+field(asym,1)
  for ch in range(5):
   left=[(ch*7+k*3+v)%64 for k in range(hb)];right=[(ch*11+k*5+v)%64 for k in range(hb)]if asym else left;phase=(ch+v)%2;ph=[(ch+k*9)%64 for k in range(hb)]if phase else[0]*hb
   wire+=''.join(field(x,6)for x in left)+(''.join(field(x,6)for x in right)if asym else'')+field(phase,1)+(''.join(field(x,6)for x in ph)if phase else'');hrtf.append([left,right,phase,ph])
 wire+='0'*(-len(wire)%8);ext=[];residual=[0,0,[0]*(len(lfe)+nt),[0]*(len(lfe)+nt)];dmx_residual=[0,0,0];arbitrary=[0,0,[],[],[],[]]
 kinds=[[],[0],[1],[2],[14,15],[0,1,2,15]][v]
 for kind in kinds:
  data=''
  if kind==0:
   ri=(res+v)%13;fc=v%4;present=[int(i%2==0)for i in range(len(lfe)+nt)];rb=[(0 if i==0 else bands)if flag else 0 for i,flag in enumerate(present)];data=field(ri,4)+field(fc,2)+''.join(field(flag,1)+(field(rb[i],5)if flag else'')for i,flag in enumerate(present));residual=[ri,fc,present,rb]
  elif kind==1:
   dmx_residual=[(tree+v)%13,v%4,bands];data=field(dmx_residual[0],4)+field(dmx_residual[1],2)+field(bands,5)
  elif kind==2:
   # A complete binary split of the first output; all other roots are leaves.
   data='100'+'0'*(out-1)+'1'+field(v%2,1)+(field(bands,5)if v%2 else'');positions=list(range(out+1));data+=''.join(field(x,5)for x in positions);arbitrary=[1,out+1,positions,[1],[v%2],[bands]]
  else:data=''.join(field((i*13+kind)%256,8)for i in range(15 if kind==14 else 281 if v==4 else 1))
  payload=packed(data);wire+=field(kind,4)+length(len(payload))+''.join(field(x,8)for x in payload);ext.append(kind)
 head=[idx if idx<13 else 15,0 if idx<13 else rate,frame,res,tree,quant,one,2 if 1 in kinds else raw_dmx,sur,lf,gain,matrix,shape,dec,int(bin_mode),0,hres if bin_mode else 0,asym if bin_mode else 0,0,inp,out,len(lfe),nt]
 expected=dict(head=head,ott=ott,ttt=ttt,hrtf=hrtf,extensions=ext,residual=residual,dmx_residual=dmx_residual,arbitrary=arbitrary)
 return packed(wire),dict(rate=rate,raw_downmix=raw_dmx,expected=expected)
def generate():
 files={};cases=[]
 for tree in range(8):
  for res in range(1,8):
   for v in range(6):
    data,c=config(tree,res,v);name=f'tree{tree}-res{res}-v{v}';files[name+'.bin']=data;c.update(name=name,file=name+'.bin');cases.append(c)
 movies=[]
 for tree in range(8):
  inp=TREES[tree][0];cfg=inp if inp in [1,2]else 6
  source=eld(specs=[(512,48000,cfg,'base',0,0,False)]);m=json.loads(source['manifest.json']);case=m['cases'][0];blob=source['packets.bin'];payload,c=config(tree,1,5,True)
  asc=packed(field(31,5)+field(7,6)+frequency(48000)+field(cfg,4)+'00000'+field(2,4)+length(len(payload))+''.join(field(x,8)for x in payload)+'0000'+'00')
  case.update(asc=asc.hex());name=f'tree{tree}';files[name+'.config']=payload;files[name+'.asc']=asc;files[name+'.mp4']=video_fixture([case],blob,inp,filename=name+'.mp4',write=False)['data'];movies.append(dict(name=name,config=name+'.config',asc=name+'.asc',video=name+'.mp4',input_channels=inp,output_channels=TREES[tree][1]))
 files['manifest.json']=(json.dumps(dict(cases=cases,movies=movies,provenance='Own configuration fields, analytical expected metadata and own nonzero AAC-ELD cores. Movie tests are configuration/refusal controls, not spatial playback acceptance.'),indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(exist_ok=True)
 for name,data in generate().items():(OUT/name).write_bytes(data)
