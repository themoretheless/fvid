"""Owned disappearing/returning coded planes with fixed 7.1 speaker output."""
import json
from pathlib import Path
from generate_eac3_dependent import packet,COUNTS,SLOTS
from generate_eac3_changing_roles import video
from generate_eac3_bsid import movie
from generate_eac3_presentation import ORDER
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/eac3-changing-widths'
LAYOUT={1:1,2:2,3:3,4:6,5:7}
ROLES=['L','C','R','Ls','Rs'];BITS=[0x8000,0x4000,0x2000,0x1000,0x0800]
def generate():
 files={};cases=[]
 for lfe_case,maximum in [(False,n)for n in range(2,6)]+[(True,2)]:
  for blocks in ([3,6]if maximum<=3 else[3]):
   for rate in [48000,44100]:
    for transient in [False,True]:
     name=f'w{"lfe" if lfe_case else maximum}-{blocks}-{rate}-{int(transient)}';programmes=[[],[],[]];components=[]
     for i in range(3):
      largest=6 if i==0 else(2 if i==1 else maximum)
      layout=7 if i==0 else(2 if i==1 else (1 if lfe_case else LAYOUT[maximum]));lfe=i==0 or(i==2 and lfe_case)
      args=dict(allocated=not(i==0 and blocks==6),blocks_override=blocks,divisor=1<<i,rate_code={48000:0,44100:1}[rate])
      controls=[];roles=[];widths=[];windows=[None]*COUNTS[layout]
      if transient and i==2:windows[0]=(0,255)
      for f in range(3):
       width=1 if i==2 and f==1 else largest
       coded_layout=(1 if lfe_case else LAYOUT[width])if i==2 else layout
       frame_lfe=lfe and not(i==2 and lfe_case and f==1)
       mapping=None if i==0 else(0x0200 if i==1 else ((0x8001 if frame_lfe else 0x8000)if lfe_case else sum(BITS[:width])))
       present=['L','C','R','Ls','Rs','LFE']if i==0 else(['Lrs','Rrs']if i==1 else (['L','LFE']if lfe_case else ROLES)[:width])
       roles.append(present);widths.append(width)
       control,_=packet(layout,lfe,0,0,'mixed',None,False,silent_planes=range(width,largest)if i==2 else(),**args)
       controls.append(control)
       raw,_=packet(coded_layout,frame_lfe,max(0,i-1),0,'mixed',mapping,i>0,last=i==2,transient_windows=windows[:COUNTS[coded_layout]]if transient else None,**args)
       programmes[f].append(raw)
      key=f'{name}-stream{i}-control.mkv';files[key]=movie(controls,blocks=blocks,channels=largest,rate=rate)
      slots=SLOTS[layout]+([3]if lfe else[])
      components.append(dict(control=key,roles=roles,widths=widths,coded_to_native=[sorted(slots).index(s)for s in slots],windows=[None if w is None else dict(location=w[0]*4,length=w[1])for w in windows]+([None]if lfe else[])))
     files[name+'.ec3']=b''.join(p for ps in programmes for p in ps);files[name+'.mkv']=video(programmes,blocks,rate)
     cases.append(dict(name=name,rate=rate,blocks=blocks,maximum=maximum,lfe_case=lfe_case,transient=transient,components=components,speakers=sorted({'L','R','C','LFE','Lrs','Rrs','Ls','Rs'},key=ORDER.index)))
     if blocks==6:
      from generate_eac3_mp4_presentation import rewrite
      from generate_eac3_aht_lfe import audiovisual
      from generate_eac3_playback import BitWriter
      bitrate=(sum(map(len,programmes[0]))*8*rate+1535999)//1536000
      w=BitWriter().put(bitrate,13).put(0,3).put({48000:0,44100:1}[rate],2).put(16,5).zeros(5).put(7,3).put(1,1).zeros(3).put(2,4).put(2,9)
      assert len(w.bits)==48
      config=bytes(sum(w.bits[i+j]<<(7-j)for j in range(8))for i in range(0,48,8))
      files[name+'.mp4']=rewrite(audiovisual([b''.join(ps)for ps in programmes]),config,rate);cases[-1]['mp4']=name+'.mp4'
 files['manifest.json']=(json.dumps(dict(accepted=True,cases=cases),indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
