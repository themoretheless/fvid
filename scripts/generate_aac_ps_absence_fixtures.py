#!/usr/bin/env python3
"""Own absent/late PS packet videos and independent stereo PCM references.
Normative dual mono (GOST 6.5.1/A.1), independent Decimal decorrelation/matrices
and direct synthesis contributions. Offline, no foreign decoder or private data.
"""
import json,struct,re,hashlib
from decimal import Decimal as D,localcontext
from generate_he_aac_packet_fixtures import DEST,packet,asc,video_fixture
from generate_aac_sbr_dsp_fixtures import payload,ROOT
from generate_aac_sbr_frequency_oracles import tables
from generate_aac_ps_fixtures import sbr
from generate_aac_ps_history_fixtures import zero,targets,encode
from generate_aac_ps_dsp_oracles import read,synthesize
from generate_aac_ps_decorrelation_oracles import MAP,ZERO,mul,add,scale,response

def main():
 blob=bytearray();gold=bytearray();cases=[]
 def save(values):
  values=list(values);off=len(gold);gold.extend(struct.pack('<'+'d'*len(values),*[float(v) for v in values]));return [off,len(values)]
 source=(ROOT/'crates/fvid-media/src/owned_aac/aac_sbr_qmf_window.rs').read_text();window=[float(v) for v in re.findall(r'-?\d+\.\d+',source.split('= [',1)[1])]
 raw_noise=(DEST/'aac-sbr-noise-protocol.f64le').read_bytes();noise=[tuple(D.from_float(v) for v in struct.unpack_from('<dd',raw_noise,i*16)) for i in range(512)]
 with localcontext() as context:
  context.prec=80;amplitude=(D(128)*D('.5')/D('1.5')).sqrt()*min(D(3).sqrt(),D('1.584893192'))
 for slots in [15,16]:
  source_manifest=json.loads((DEST/('aac-sbr-ps-30-oracles.json' if slots==15 else 'aac-ps-matrix-controller-oracles.json')).read_text());matrix=source_manifest['videos'][0]['expected']
  for name,flags in [('all-mono',[False,False,False]),('late-ps',[False,True,True]),('missing-middle',[True,False,True]),('trailing-mono',[True,True,False]),('headerless-start',[True,True,True]),('header-only-start',[True,True,True]),('dependent-start',[True,True,True])]:
   frames=[];parameter=zero();ps_index=0;spec=[]
   for fi,present in enumerate(flags):
    if fi==0 and name.endswith('-start'):
     header=name!='headerless-start';dependent=name=='dependent-start'
     mode=0 if dependent else 1
     rows=[targets(mode,mode,0,phase=True)] if dependent else []
     text,borders=encode(header,mode,mode,header,header,dependent,rows,parameter,slots*2,first_time=dependent,extension=dependent)
     raw=sbr(text,fi);spec.append(None)
     if rows:parameter=rows[-1]
    elif present:
     enabled=ps_index==0;count=2 if enabled else 0;rows=[targets(1,1,e,phase=True) for e in range(count)]
     text,borders=encode(True,1,1,True,True,enabled,rows,parameter,slots*2)
     raw=sbr(text,fi);spec.append(matrix[ps_index]);ps_index+=1
     if rows:parameter=rows[-1]
    else:
     raw=payload(len(tables(10,27,0,False,0,0)[1])-1,2,True,fi);spec.append(None)
    data=packet(raw);frames.append(dict(offset=len(blob),bytes=len(data)));blob.extend(data)
   assert len(frames)==len(flags)==3
   case=dict(slots=slots,bands=64,frames=frames,asc=asc(24000,48000,slots,'explicit',ps=True).hex(),pcm_offset=0,samples=slots*128*3*2)
   video=video_fixture([case],blob,channels=2,filename=f'he-aac-ps-{name}-{slots*64}-synthetic.mp4');video.pop('pcm_offset');video.pop('samples')
   qmf=[[scale(noise[(n*17+k-10+1)%512],amplitude) if 10<=k<27 else ZERO for k in range(64)] for n in range(slots*6)]
   output=[[],[]];history=[];power=[];peak=[];previous_present=False
   for fi,e in enumerate(spec):
    rows=qmf[fi*slots*2:(fi+1)*slots*2]
    if e is None:
     output[0].extend(rows);output[1].extend(rows);previous_present=False;continue
    if not previous_present:history=[];power=[];peak=[]
    bindings=MAP['hybrid20'];mono=[[r[b['qmf']] for b in bindings] for r in rows];history.extend(mono)
    for offset,row in enumerate(mono):
     n=len(history)-len(mono)+offset;p=[D(0)]*20
     for value,b in zip(row,bindings):p[b['parameter']]+=value[0]**2+value[1]**2
     power.append(p);pe=[max((D('.76592833836465')**(n-j))*v[b] for j,v in enumerate(power)) for b in range(20)];peak.append(pe)
     sm=[D('.25')*sum(D('.75')**(n-j)*v[b] for j,v in enumerate(power)) for b in range(20)]
     df=[D('.25')*sum(D('.75')**(n-j)*(peak[j][b]-power[j][b]) for j in range(n+1)) for b in range(20)]
     gain=[sm[b]/(D('1.5')*df[b]) if D('1.5')*df[b]>sm[b] else D(1) for b in range(20)]
     coeff=read(e['coefficients'][offset],e.get('coefficients_file','aac-ps-matrix-controller-coefficients.bin'));stereo=[[ZERO]*64 for _ in range(2)]
     for k,b in enumerate(bindings):
      h=response(20,k,len(history));y=ZERO
      for j in range(n+1):y=add(y,mul(h[n-j],history[j][k]))
      y=scale(y,gain[b['parameter']]);base=b['parameter']*8;c=[tuple(coeff[base+m*2:base+m*2+2]) for m in range(4)]
      if b['conjugate']:c=[(r,-i) for r,i in c]
      for channel in range(2):stereo[channel][b['qmf']]=add(stereo[channel][b['qmf']],add(mul(c[channel],row[k]),mul(c[2+channel],y)))
     for channel in range(2):output[channel].append(stereo[channel])
    previous_present=True
   refs={}
   for mode,bands in [('Double',64),('Core',32)]:refs[mode]=[save(v/D(32768) for v in map(D.from_float,synthesize(c,bands,window))) for c in output]
   cases.append(dict(name=name,video=video,slots=slots,frames=frames,ps_present=flags,stereo_active=[e is not None for e in spec],asc_core=asc(24000,24000,slots,'explicit',ps=True).hex(),pcm=refs))
 (DEST/'he-aac-ps-absence-packets.bin').write_bytes(blob);(DEST/'aac-ps-absence-pcm.bin').write_bytes(gold)
 (DEST/'aac-ps-absence-oracles.json').write_text(json.dumps(dict(kind='original silent native core, SBR noise and independent mono/PS stereo transitions',cases=cases,packet_sha256=hashlib.sha256(blob).hexdigest(),pcm_sha256=hashlib.sha256(gold).hexdigest()),indent=2)+'\n')
 print(len(cases),'original absent/late PS videos')
if __name__=='__main__':main()
