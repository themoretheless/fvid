"""Owned unequal final-dependent dual-mono DRC and expanded role ownership."""
import json
from pathlib import Path
from generate_eac3_dependent import packet,COUNTS,SLOTS
from generate_eac3_changing_roles import video
from generate_eac3_bsid import movie
from generate_eac3_presentation import ORDER
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/eac3-expanded-dual-drc'
PLANS=[('51-lr',7,True,[(0,0x0200,['Lrs','Rrs']),(0,0xa000,['L','R'])]),('51-lc',7,True,[(0,0x0200,['Lrs','Rrs']),(0,0xc000,['L','C'])]),('dual-lc',0,False,[(7,0x5a00,['C','Ls','Rs','Lrs','Rrs']),(0,0xc000,['L','C'])])]
def factors(codes):
 gain=1;out=[]
 for c in codes:
  if c is not None:gain=2**((c>>5)-(8 if c&128 else 0))*(32+(c&31))/32
  out.append(gain)
 return out

def generate(*, rf=False):
 files={};cases=[]
 for label,base_layout,lfe,deps in PLANS:
  for rate in [48000,44100]:
   for blocks in ([3]if base_layout==0 else[3,6]):
    for transient in [False,True]:
     for clip in [False,True]:
      for switching in [False,True]:
       name=f'{label}-{rate}-{blocks}-{int(transient)}-{int(clip)}'+('-switch'if switching else'');programmes=[[],[],[]];parts=[]
       first=[127,None,224]if clip else[64,None,224];second=[255,None,96]if clip else[192,32,None]
       if blocks==6:first=first+[None,32,None];second=second+[None,224,None]
       entries=[(base_layout,lfe,None,['L','R']if base_layout==0 else['L','C','R','Ls','Rs','LFE'])]+[(l,False,m,r)for l,m,r in deps]
       for i,(layout,has_lfe,mapping,roles)in enumerate(entries):
        nf=COUNTS[layout];last=i==2;windows=[None]*nf
        if transient and i:windows[0]=(0,255);windows[1]=(240,127)
        args=dict(blocks_override=blocks,divisor=1 if clip else 1<<i,rate_code={48000:0,44100:1}[rate],allocated=not(i==0 and blocks==6))
        control,_=packet(layout,has_lfe,0,0,'mixed',None,False,**args)
        key=f'{name}-stream{i}-control.mkv';files[key]=movie([control]*3,blocks=blocks,channels=len(roles),rate=rate)
        codes=first if last else([96]+[None]*(blocks-1)if i==0 else None)
        codes2=second if last else([160]+[None]*(blocks-1)if i==0 and layout==0 else None)
        raw,_=packet(layout,has_lfe,max(0,i-1),0,'mixed',mapping,i>0,last=last,gains=codes,gains_second=codes2,exponent=0 if clip else 10,transient_windows=windows if transient else None,**args)
        for f,ps in enumerate(programmes):
         if rf:
          frame_layout=2 if last and switching and f==1 else layout
          compr=[16,240,127][f] if last else (96 if i==0 else None)
          compr2=[240,None,31][f] if last else (112 if i==0 and layout==0 else None)
          changed,_=packet(frame_layout,has_lfe,max(0,i-1),0,'mixed',mapping,i>0,last=last,gains=codes,gains_second=codes2,compression=compr,compression_second=compr2,exponent=0 if clip else 10,transient_windows=windows if transient else None,**args)
          ps.append(changed)
         elif last and switching and f==1:
          changed,_=packet(2,has_lfe,max(0,i-1),0,'mixed',mapping,True,last=True,gains=codes,exponent=0 if clip else 10,transient_windows=windows if transient else None,**args)
          ps.append(changed)
         else:ps.append(raw)
        slots=SLOTS[layout]+([3]if has_lfe else[])
        parts.append(dict(control=key,roles=roles,coded_to_native=[sorted(slots).index(s)for s in slots],dual=layout==0,dual_frames=[layout==0,layout==0 and not(last and switching),layout==0],windows=[None if w is None else dict(location=w[0]*4,length=w[1])for w in windows]+([None]if has_lfe else[])))
       speakers=sorted({r for p in parts for r in p['roles']},key=ORDER.index)
       files[name+'.ec3']=b''.join(p for ps in programmes for p in ps)
       # Base helper's channel-count field is eight; seven-role cases need seven.
       film=video(programmes,blocks,rate)
       if len(speakers)==7:
        from generate_eac3_presentation import element,uint
        assert film.count(uint(0x9f,8))==1
        film=film.replace(uint(0x9f,8),uint(0x9f,7))
       files[name+'.mkv']=film
       cases.append(dict(name=name,rate=rate,blocks=blocks,transient=transient,clip=clip,switching=switching,scale=1024 if clip else 1,speakers=speakers,channels=len(speakers),mask=0x63f if len(speakers)==8 else 0x637,components=parts,gains=[factors(first),factors(second)]))
       if rf:cases[-1]['rf_words']=[[16,240,127],[240,None,31]]
       if blocks==6:
        from generate_eac3_mp4_presentation import rewrite
        from generate_eac3_aht_lfe import audiovisual
        from generate_eac3_playback import BitWriter
        bitrate=(sum(map(len,programmes[0]))*8*rate+1535999)//1536000
        w=BitWriter().put(bitrate,13).put(0,3).put({48000:0,44100:1}[rate],2).put(16,5).zeros(5).put(7,3).put(1,1).zeros(3).put(2,4).put(2,9)
        config=bytes(sum(w.bits[i+j]<<(7-j)for j in range(8))for i in range(0,48,8))
        files[name+'.mp4']=rewrite(audiovisual([b''.join(ps)for ps in programmes]),config,rate);cases[-1]['mp4']=name+'.mp4'
 files['manifest.json']=(json.dumps(dict(accepted=True,cases=cases),indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
