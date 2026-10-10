#!/usr/bin/env python3
"""Own indexed ER multichannel SBR, distinct gains and scalar QMF references."""
import json,math
from generate_he_aac_packet_fixtures import DEST,field,frequency,packed,video_fixture
from generate_aac_sbr_data_fixtures import word
from generate_aac_sbr_dsp_fixtures import header
from generate_aac_sbr_frequency_oracles import tables
LAYOUTS={2:([1],[0,1],3),3:([0,1],[2,0,1],7),4:([0,1,0],[2,0,1,3],0x107),5:([0,1,1],[2,0,1,3,4],0x37),6:([0,1,1,3],[2,0,1,4,5,3],0x3f),7:([0,1,1,1,3],[2,6,7,0,1,4,5,3],0xff),11:([0,1,1,0,3],[2,0,1,4,5,6,3],0x13f),12:([0,1,1,1,3],[2,0,1,6,7,4,5,3],0x63f),14:([0,1,1,3,1],[2,0,1,4,5,3,6,7],0x503f)}
def sbr(gains,frame,nhigh):
    temporal=frame>0;env=[]
    for gain in gains:env.append(word(0,0)*nhigh if temporal else field(2+2*gain,7)+word(1,0)*(nhigh-1))
    noise=word(8,0) if temporal else field(7,5)
    width=len(gains)
    data='0'+('0' if width==2 else '')+'00001'*width+field(temporal,1)*(2*width)+'00'*width+''.join(env)+noise*width+'0'*(width+1)
    return packed(field(13,4)+field(frame==0,1)+(header(0,True) if frame==0 else '')+data)
def main():
    source=json.loads((DEST/'aac-sbr-dsp-oracles.json').read_text())['cases'];_,high,_,_=tables(10,27,0,False,0,0);blob=bytearray();cases=[]
    channel=field(100,8)+'0000'+'000000'+'0'+'000'
    def store(data):
        row=dict(offset=len(blob),bytes=len(data));blob.extend(data);return row
    for aot in (17,19):
      for slots in (15,16):
       for bands,rate in ((32,24000),(64,48000)):
        ref=next(r for r in source if r['slots']==slots and r['bands']==bands and r['limiter']==0 and r['smoothing'])
        for layout,(kinds,mapping,mask) in LAYOUTS.items():
          groups=[];offset=0;scales=[]
          for i,kind in enumerate(kinds):
            width=2 if kind==1 else 1;gain=list(range(offset,offset+width));offset+=width
            groups.append(dict(kind=kind,tag=i+1,gains=gain));scales.extend([0.0]*width if kind==3 else [math.sqrt(2**g) for g in gain])
          asc=packed(field(5,5)+frequency(24000)+field(layout,4)+frequency(rate)+field(aot,5)+field(slots==15,1)+'00'+'00').hex()
          control_asc=packed(field(5,5)+frequency(24000)+field(layout,4)+frequency(rate)+field(2,5)+field(slots==15,1)+'00').hex()
          frames=[];controls=[];bad={key:[] for key in ('missing','excess','order')}
          for frame in range(3):
            cores=[];extensions=[];ordinary=''
            for g in groups:
              kind=g['kind'];core=field(g['tag'],4)+('0'+channel*2 if kind==1 else channel);cores.append(core)
              ordinary+=field(kind,3)+core
              if kind!=3:
                raw=sbr(g['gains'],frame,len(high)-1);ext=''.join(field(v,8) for v in raw);extensions.append(ext)
                count=len(raw);ordinary+='110'+(field(count,4) if count<15 else '1111'+field(count-14,8))+ext
            prefix=''.join(cores);ext=''.join(extensions)
            frames.append(store(packed(prefix+ext)));controls.append(store(packed(ordinary+'111')))
            bad['missing'].append(store(packed(prefix+''.join(extensions[:-1]))))
            bad['excess'].append(store(packed(prefix+ext+extensions[0])))
            bad['order'].append(store(packed(prefix+ext+field(0x20,8)+field(0,8))))
          name=f'{aot}-{layout}-{slots}-{bands}';c=dict(name=name,aot=aot,layout=layout,slots=slots,bands=bands,asc=asc,control_asc=control_asc,frames=frames,control_frames=controls,bad_frames=bad,mapping=mapping,scales=scales,channels=offset,channel_mask=mask,pcm_offset=ref['pcm_offset'],samples=ref['samples'],container_rate=rate,container_frame_samples=slots*bands*2,invalid=[])
          c['video']=video_fixture([c],blob,channels=offset,filename=f'aac-er-multi-sbr-{name}-synthetic.mp4')
          for key,error in [('missing','missing ER AAC SBR element'),('excess','excess ER AAC SBR element'),('order','ER AAC extensions must precede SBR')]:
            if key=='missing' and len(extensions)==1:continue
            rows=[frames[0],bad[key][1],frames[2]]
            c['invalid'].append(dict(kind=key,error=error,video=video_fixture([dict(c,frames=rows)],blob,channels=offset,filename=f'aac-er-multi-sbr-{name}-{key}-synthetic.mp4')))
          cases.append(c)
    (DEST/'aac-er-multi-sbr-packets.bin').write_bytes(blob)
    (DEST/'aac-er-multi-sbr.json').write_text(json.dumps(dict(cases=cases,provenance='Own silent indexed ER LC/LTP core; distinct SBR per-channel absolute envelope levels, temporal zero deltas and uncoupled CPE. Independent direct QMF gold scaled by sqrt(2)^channel from envelope energy; zero LFE. No private media, foreign decoder, FFmpeg or network.'),indent=2)+'\n')
    print(len(cases),'ER multichannel SBR cases')
if __name__=='__main__':main()
