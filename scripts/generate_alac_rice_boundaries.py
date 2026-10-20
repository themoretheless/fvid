"""Own ALAC Rice boundary packets, literal mono PCM, neutral VP9; offline."""
import json,hashlib,struct
from pathlib import Path
from generate_alac_multichannel_fixtures import mp4,u
from generate_audio_resample_window_fixture import ebml,word
from generate_vp9_chroma import frame
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/alac-rice-boundaries'
def scalar(value,k,divisor,width):
 quotient,remainder=divmod(value,divisor)
 if quotient>8:return '1'*9+f'{value:0{width}b}'
 return '1'*quotient+'0'+(''if k==1 else f'{remainder+1:0{k}b}'if remainder else '0'*(k-1))
def entropy(target,depth,initial,mult,limit):
 history=initial;modifier=0;index=0;bits='';stats=dict(zero_history_runs=0,masked_run_parameters=0,max_runs=0,modifier_history_edges=0)
 while index<len(target):
  value=target[index];folded=2*value if value>=0 else -2*value-1;coded=folded-modifier;assert coded>=0
  stats['modifier_history_edges']+=coded==65535 and modifier==1
  k=min(((history>>9)+3).bit_length()-1,limit)
  bits+=scalar(coded,k,(1<<k)-1,depth)
  history=65535 if coded>65535 else (history+folded*mult-((history*mult)>>9))&0xffffffff
  modifier=0
  if history<128 and index+1<len(target):
   k=32-history.bit_length()-24+((history+16)>>6)
   divisor=((1<<k)-1)&((1<<limit)-1)
   stats['zero_history_runs']+=history==0;stats['masked_run_parameters']+=k>limit
   run=0
   while run<65535 and index+run+1<len(target)and target[index+run+1]==0:run+=1
   stats['max_runs']+=run==65535
   bits+=scalar(run,k,divisor,16)
   index+=run;modifier=int(run<65535);history=0
  index+=1
 return bits,stats

def generate():
 files={};cases=[]
 profiles=[('zero-initial-history',[0]*32,16,0,40,14),('zero-history-after-run',[0]*3+[-1]+[0]*28,16,10,0,14),('small-limit-1',[0]*6+[5]+[0]*9,16,10,40,1),('small-limit-2',[0]*6+[5]+[0]*9,16,10,40,2),('small-limit-4',[0]*21+[5]+[0]*10,16,0,40,4),('long-run-resume',[0]*65536+[5],16,10,40,14),('terminal-run-65535',[0]*65536,16,10,40,14),('long-multiple-runs',[0]*65570,16,10,40,14),('modifier-history-edge',[0,0,32768,7]+[0]*12,24,10,40,14)]
 for name,target,depth,initial,mult,limit in profiles:
  n=len(target);body,stats=entropy(target,depth,initial,mult,limit)
  bits='000'+'0000'+'0'*12+'1'+'00'+'0'+f'{n:032b}'+'0'*16+'0000'+'0001'+('100'if mult else'000')+'00000'+body+'111'
  bits+='0'*(-len(bits)%8);packet=int(bits,2).to_bytes(len(bits)//8,'big')
  cookie=bytearray(24);cookie[:4]=word(n);cookie[5:10]=bytes([depth,40,initial,limit,1]);cookie[20:24]=word(48000)
  video=ebml('ae',u('d7',1)+u('73c5',1)+u('83',1)+ebml('86',b'V_VP9')+ebml('e0',u('b0',16)+u('ba',16)))
  audio=ebml('ae',u('d7',2)+u('73c5',2)+u('83',2)+ebml('86',b'A_ALAC')+ebml('63a2',cookie)+ebml('e1',ebml('b5',struct.pack('>d',48000))+u('9f',1)))
  clusters=b''
  for i in range(2):clusters+=ebml('1f43b675',u('e7',i*n*1000000000//48000//1000)+ebml('a3',b'\x81\0\0\x80'+frame(8,'420',16,16))+ebml('a3',b'\x82\0\0\x80'+packet))
  files[name+'.mkv']=ebml('1a45dfa3',ebml('4282',b'matroska')+u('4287',3)+u('4285',2))+ebml('18538067',ebml('1549a966',u('2ad7b1',1000))+ebml('1654ae6b',video+audio)+clusters)
  files[name+'.m4a']=mp4(cookie,1,depth,[packet,packet],n)
  files[name+'.cookie']=bytes(cookie);files[name+'.packet']=packet;files[name+'.i32le']=struct.pack('<'+'i'*n,*target)
  cases.append(dict(name=name,samples=n,depth=depth,initial_history=initial,history_mult=mult,rice_limit=limit,stats=stats))
 files['manifest.json']=(json.dumps(dict(provenance='Authored mono integer PCM, independent Rice encoder and neutral VP9, covering zero history, masked run divisors, 65535 run modifier reset, and coded-vs-folded history clamp. No private media/foreign encoder/network.',cases=cases,files={n:hashlib.sha256(v).hexdigest()for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
