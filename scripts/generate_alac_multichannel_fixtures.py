"""Own ALAC multi-element raw/Rice packets and neutral VP9, offline only."""
import struct,json,hashlib
from pathlib import Path
from generate_audio_resample_window_fixture import ebml,word,atom
from generate_vp9_chroma import frame
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/alac-multichannel'
LAYOUTS={2:(101,[0,1],3,[2]),3:(113,[1,2,0],7,[1,2]),4:(116,[1,2,0,3],0x107,[1,2,1]),5:(120,[1,2,0,3,4],0x37,[1,2,2]),6:(124,[1,2,0,5,3,4],0x3f,[1,2,2,1]),7:(142,[1,2,0,6,3,4,5],0x13f,[1,2,2,1,1]),8:(127,[3,4,0,7,5,6,1,2],0xff,[1,2,2,2,1])}
def u(t,n):return ebml(t,n.to_bytes(max(1,(n.bit_length()+7)//8),'big'))
def value(n,c):return ((c+1)*101+n*7)*(-1 if c%2 else 1)
def packet(depth,groups,start,channels):
 bits='';c=0
 for instance,width in enumerate(groups):
  tag=1 if width==2 else 3 if channels>=6 and c==channels-1 else 0
  bits+=f'{tag:03b}{instance:04b}'+12*'0'+'1'+'00'+'1'+f'{16:032b}'
  for n in range(start,start+16):
   for ch in range(c,c+width):bits+=f'{value(n,ch)&((1<<depth)-1):0{depth}b}'
  c+=width
 assert c==channels
 bits+='111';bits+='0'*(-len(bits)%8);return int(bits,2).to_bytes(len(bits)//8,'big')
def rice_scalar(v,k,bps,stats):
 assert 0<=v<(1<<bps) and k>=1
 q,r=(v,0)if k==1 else divmod(v,(1<<k)-1)
 if q>8:
  stats['escapes']+=1;return '1'*9+f'{v:0{bps}b}'
 stats['ordinary']+=1
 return '1'*q+'0'+(''if k==1 else f'{r+1:0{k}b}'if r else '0'*(k-1))
def rice_plane(values,bps,stats):
 history=10;bits='';index=0;modifier=0
 while index<len(values):
  v=values[index];folded=2*v if v>=0 else -2*v-1
  k=min(((history>>9)+3).bit_length()-1,14)
  assert folded>=modifier
  coded=folded-modifier
  bits+=rice_scalar(coded,k,bps,stats);modifier=0
  history=65535 if coded>65535 else history+folded*40-((history*40)>>9)
  if history<128 and index+1<len(values):
   k=8-history.bit_length()+((history+16)>>6)
   run=0
   while run<65535 and index+run+1<len(values)and values[index+run+1]==0:run+=1
   bits+=rice_scalar(run,k,16,stats);stats['runs']+=1;stats['run_samples']+=run
   if index+run+1<len(values):stats['resumed_run_samples']+=run
   index+=run;modifier=int(run<65535);history=0
  index+=1
 return bits
def adaptive_residuals(target,order,stats,quant=1):
 # Encoder inversion uses authored PCM directly, in newest-first wire order.
 # PCM expectation remains a literal ramp; Apple independently checks the packet.
 coefs=[0]*order
 residual=[target[0]]+[target[j]-target[j-1]for j in range(1,order+1)]
 for i in range(order+1,len(target)):
  base=target[i-order-1]
  prediction=base+((sum(coefs[k]*(target[i-k-1]-base)for k in range(order))+(0 if quant==0 else 1<<(quant-1)))>>quant)
  value=target[i]-prediction
  residual.append(value)
  sign=(value>0)-(value<0)
  remaining=value
  if sign:stats['adaptive_positive'if sign>0 else 'adaptive_negative']=stats.get('adaptive_positive'if sign>0 else 'adaptive_negative',0)+1
  for k in range(order-1,-1,-1):
   if not sign:break
   diff=base-target[i-k-1]
   adjustment=((diff>0)-(diff<0))*sign
   coefs[k]-=adjustment
   if adjustment:
    stats['adaptive_updates']=stats.get('adaptive_updates',0)+1
    stats['adaptive_positions']=stats.get('adaptive_positions',0)|(1<<k)
   remaining-=(order-k)*((diff*adjustment)>>quant)
   if remaining*sign<=0:break
 return residual

def compressed_packet(depth,groups,start,channels,mode,stats,samples=16):
 order=int(mode.split('-')[1])if mode.startswith(('predictor-','adaptive-'))else 31 if mode in ['zero-quant-integrate','zero-quant-double']else 0
 quant=0 if mode.startswith('zero-quant-')or '-q0'in mode else 1
 prediction=15 if mode in ['first-order','zero-run','zero-run-resume','zero-quant-twice','zero-quant-double']or mode.endswith('-twice')else 0
 coefficients=([order+2]+[0]*(order-2)+[1])if mode.startswith('predictor-')and 1<order<31 else [0]*order
 if mode.startswith('predictor-')and quant==0:coefficients=[2]if order==1 else [1]+[0]*(order-2)+[1]
 bits='';c=0;extra=8 if depth>=24 else 0
 for instance,width in enumerate(groups):
  if mode in ['fill','fill-extended']:
   count=3 if mode=='fill'else 16
   bits+='110'+(f'{count:04b}'if count<15 else '1111'+f'{count-14:08b}')+'10100101'*count
  if mode in ['data','data-extended','data-unaligned','data-unaligned-extended']:
   count=256 if mode.endswith('extended')else 3
   bits+='100'+'0000'+('0'if 'unaligned'in mode else '1')+(f'{count:08b}'if count<255 else '11111111'+f'{count-255:08b}')
   if 'unaligned'not in mode:bits+='0'*(-len(bits)%8)
   bits+='01011010'*count
  tag=1 if width==2 else 3 if channels>=6 and c==channels-1 else 0
  bits+=f'{tag:03b}{instance:04b}'+12*'0'+'1'+f'{extra//8:02b}'+'0'+f'{samples:032b}'
  bits+=f'{1 if width==2 else 0:08b}{1 if width==2 else 0:08b}'
  for _ in range(width):bits+=f'{prediction:04b}{quant:04b}'+'100'+f'{order:05b}'+''.join(f'{v&65535:016b}'for v in reversed(coefficients))
  planes=[[]for _ in range(width)]
  for n in range(start,start+samples):
   original=[value(n,ch)+(23 if (mode=='zero-run-resume'or (mode.startswith('adaptive-')and mode!='adaptive-1'))and n>=80 else 0)for ch in range(c,c+width)]
   mixed=[original[1]+((original[0]-original[1])>>1),original[0]-original[1]]if width==2 else original
   for ch,v in enumerate(mixed):planes[ch].append(v)
   if extra:
    for ch in range(c,c+width):bits+=f'{ch+1:08b}'
  bps=depth-extra+width-1
  for plane in planes:
   residual=[plane[0]]+[plane[j]-plane[j-1]for j in range(1,samples)]if mode in ['first-order','zero-run','zero-run-resume','zero-quant-integrate','zero-quant-twice','zero-quant-double']else plane
   if mode=='zero-quant-double':residual=[residual[0]]+[residual[j]-residual[j-1]for j in range(1,samples)]
   if mode.startswith('predictor-'):
    # Linear ramp: asymmetric endpoint coefficients (order+2, 1), quant=1.
    # Their weighted offsets sum to 2*(order+1)*d; no adaptive loop oracle.
    d=plane[1]-plane[0]
    residual=[plane[0]]+[d]*(samples-1)if order==31 else [plane[0]]+[d]*order+[0]*(samples-order-1)
   if mode=='adaptive-1':
    # A literal linear target forces the initially-zero coefficient through
    # 0,1,2,3,4 (quantization=1), then leaves zero residuals. This closed-form
    # sequence does not run a decoder or copy its adaptive loop.
    d=plane[1]-plane[0]
    residual=[plane[0],d]+[2*d-(((j-2)*d+1)//2)if j<6 else 0 for j in range(2,samples)]
   if mode.startswith('adaptive-')and mode!='adaptive-1':residual=adaptive_residuals(plane,order,stats,quant)
   if '-q0'in mode and mode.endswith('-twice'):residual=[residual[0]]+[residual[j]-residual[j-1]for j in range(1,samples)]
   bits+=rice_plane(residual,bps,stats)
  c+=width
 bits+='111';bits+='0'*(-len(bits)%8);return int(bits,2).to_bytes(len(bits)//8,'big')
def mp4(cookie,channels,depth,packets,samples=16):
 ftyp=atom(b'ftyp',b'isom'+word(0)+b'isommp42');mdat=atom(b'mdat',b''.join(packets))
 entry=bytearray(28);entry[6:8]=b'\0\1';entry[16:20]=struct.pack('>HH',channels,depth);entry[24:28]=word(48000<<16)
 stbl=atom(b'stbl',atom(b'stsd',word(0,1)+atom(b'alac',entry+atom(b'alac',word(0)+cookie)))+atom(b'stts',word(0,1,2,samples))+atom(b'stsz',word(0,0,2,*map(len,packets)))+atom(b'stsc',word(0,1,1,2,1))+atom(b'stco',word(0,1,len(ftyp)+8)))
 tkhd=bytearray(84);tkhd[12:16]=word(1);tkhd[20:24]=word(samples*2);tkhd[40:44]=tkhd[56:60]=word(65536);tkhd[72:76]=word(0x40000000)
 mdhd=bytearray(24);mdhd[12:20]=word(48000,samples*2);mdhd[20:22]=(21956).to_bytes(2,'big')
 dinf=atom(b'dinf',atom(b'dref',word(0,1)+atom(b'url ',word(1))));mdia=atom(b'mdia',atom(b'mdhd',mdhd)+atom(b'hdlr',word(0,0)+b'soun'+bytes(12))+atom(b'minf',dinf+stbl));mvhd=bytearray(100);mvhd[12:20]=word(48000,samples*2)
 return ftyp+mdat+atom(b'moov',atom(b'mvhd',mvhd)+atom(b'trak',atom(b'tkhd',tkhd)+mdia))
def generate():
 files={};cases=[];ancillary_invalid=[]
 for channels,(tag,order,mask,groups)in LAYOUTS.items():
  for depth in [16,20,24,32]:
   for named in [False,True]:
    for mode in ['raw','rice','first-order','zero-run','adaptive-1','zero-run-resume','fill','fill-extended','data','data-extended','data-unaligned','data-unaligned-extended','predictor-2','predictor-4','predictor-8','predictor-16','predictor-30','predictor-31','adaptive-2','adaptive-4','adaptive-8','adaptive-16','adaptive-30','zero-quant-rice','zero-quant-integrate','zero-quant-twice','zero-quant-double']+[f'{kind}-{order}-q0'+suffix for kind in ['predictor','adaptive']for order in [1,2,4,8,16,30]for suffix in ['','-twice']]:
     samples=128 if (mode.startswith('adaptive-')or mode in ['zero-run','zero-run-resume'])or (mode.startswith('predictor-')and mode!='predictor-31')else 16
     cookie=bytearray(24);cookie[:4]=word(samples);cookie[5:10]=bytes([depth,40,10,14,channels]);cookie[20:24]=word(48000)
     if named:cookie+=word(24)+b'chan'+word(0,(tag<<16)|channels,0,0)
     audio=ebml('ae',u('d7',2)+u('73c5',2)+u('83',2)+ebml('86',b'A_ALAC')+ebml('63a2',cookie)+ebml('e1',ebml('b5',struct.pack('>d',48000))+u('9f',channels)))
     video=ebml('ae',u('d7',1)+u('73c5',1)+u('83',1)+ebml('86',b'V_VP9')+ebml('e0',u('b0',16)+u('ba',16)))
     stats={'ordinary':0,'escapes':0,'runs':0,'run_samples':0,'resumed_run_samples':0}
     packets=[packet(depth,groups,i*16,channels)if mode=='raw'else compressed_packet(depth,groups,i*samples,channels,mode,stats,samples)for i in range(2)]
     blocks=u('e7',0)
     for i,payload in enumerate(packets):blocks+=ebml('a3',b'\x81'+struct.pack('>hB',i*1000,128)+frame(8,'420',16,16))+ebml('a3',b'\x82'+struct.pack('>hB',i*samples*1000000000//48000//1000,128)+payload)
     name=f'{channels}-{depth}-'+('named'if named else'discrete')+(''if mode=='raw'else '-'+mode)+'.mkv'
     files[name]=ebml('1a45dfa3',ebml('4282',b'matroska')+u('4287',3)+u('4285',2))+ebml('18538067',ebml('1549a966',u('2ad7b1',1000))+ebml('1654ae6b',video+audio)+ebml('1f43b675',blocks))
     files[name[:-4]+'.m4a']=mp4(cookie,channels,depth,packets,samples)
     if channels==2 and depth==16 and not named and mode in ['fill','fill-extended','data','data-extended','data-unaligned','data-unaligned-extended']:
      # Keep the genuine video; truncate the ancillary header/payload itself.
      bad_packets=[packets[0][:1],packets[1]]
      bad_blocks=u('e7',0)
      for i,payload in enumerate(bad_packets):bad_blocks+=ebml('a3',b'\x81'+struct.pack('>hB',i*1000,128)+frame(8,'420',16,16))+ebml('a3',b'\x82'+struct.pack('>hB',i*samples*1000000000//48000//1000,128)+payload)
      bad_name='invalid-ancillary-'+mode+'.mkv'
      files[bad_name]=ebml('1a45dfa3',ebml('4282',b'matroska')+u('4287',3)+u('4285',2))+ebml('18538067',ebml('1549a966',u('2ad7b1',1000))+ebml('1654ae6b',video+audio)+ebml('1f43b675',bad_blocks))
      ancillary_invalid.append(dict(file=bad_name,error='ALAC ancillary element exceeds packet'))

     cases.append(dict(file=name,predictor_order=int(mode.split('-')[1])if mode.startswith(('predictor-','adaptive-'))else 31 if mode in ['zero-quant-integrate','zero-quant-double']else 0,prefix_bits={'fill':31,'fill-extended':143,'data':40,'data-extended':2072,'data-unaligned':40,'data-unaligned-extended':2072}.get(mode,0),mode=mode,samples=samples*2,frame_samples=samples,jump_sample=80 if mode=='zero-run-resume'or (mode.startswith('adaptive-')and mode!='adaptive-1')else None,jump_high=23 if mode=='zero-run-resume'or (mode.startswith('adaptive-')and mode!='adaptive-1')else 0,scale=256 if mode!='raw'and depth>=24 else 1,low_bias=mode!='raw'and depth>=24,rice=stats,channels=channels,depth=depth,order=order if named else list(range(channels)),mask=mask if named else (3 if channels==2 else 0)))
     if channels==2 and depth==32 and not named and mode=='raw':files['stereo32.cookie']=bytes(cookie);files['stereo32.frame']=packets[0]
 cookie=bytearray(24);cookie[:4]=word(16);cookie[5:10]=bytes([16,40,10,14,3]);cookie[20:24]=word(48000);cookie+=word(24)+b'chan'+word(0,(113<<16)|3,0,0)
 invalid=ancillary_invalid
 for label,offset,value in [('count',39,4),('tag',37,255),('flags',35,1)]:
  bad=bytearray(cookie);bad[offset]=value
  name='invalid-layout-'+label+'.mkv';files[name]=files['3-16-named.mkv'].replace(bytes(cookie),bytes(bad),1);invalid.append(dict(file=name,error='count disagrees' if label=='count' else 'unsupported ALAC channel layout tag' if label=='tag' else 'invalid ALAC channel layout block'))
 for label,rate in [('zero',0),('mismatch',44100)]:
  bad=bytearray(cookie);bad[20:24]=word(rate)
  name='invalid-rate-'+label+'.mkv'
  files[name]=files['3-16-named.mkv'].replace(bytes(cookie),bytes(bad),1)
  invalid.append(dict(file=name,error='ALAC cookie and track sample rates disagree'))
 files['manifest.json']=(json.dumps(dict(provenance='Own ALAC raw/compressed SCE/CPE/LFE multi-element packets, ordinary/escape Rice, terminal/resumed zero runs, first-order and closed-form adaptive order1 coding, authored cookies, neutral VP9. Independent integer ramp targets with optional low byte. No private media, foreign codec or network.',cases=cases,invalid=invalid,files={n:hashlib.sha256(v).hexdigest()for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
