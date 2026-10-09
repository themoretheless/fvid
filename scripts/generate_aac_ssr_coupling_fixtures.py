#!/usr/bin/env python3
"""Original SSR CCE videos: scalar oracle, no network/FFmpeg/private media."""
import json,hashlib,struct
from generate_aac_ssr_fixtures import (DEST,field,frequency,packed,video_fixture,SEQUENCES,SAMPLES,LONG,SHORT,info,gain_bits,channel,oracle,spectrum)

def program(prefix,channels,point,tags=(1,)):
    bits=prefix+field(0,4)+field(2,2)+frequency(24000)+field(1,4)+field(0,4)+field(0,4)+field(0,2)+field(0,3)+field(len(tags),4)+'000'+field(channels==2,1)+field(0,4)+''.join(field(point==3,1)+field(tag,4) for tag in tags)
    return bits+'0'*(-len(bits)%8)+field(0,8)
def config(channels,point,tags=(1,)):return packed(program(field(3,5)+frequency(24000)+'0000'+'000',channels,point,tags))
def silent(seq,shape,c,active,common):
    swb=SHORT if seq==2 else LONG
    result=field(140,8)+('' if common else info(seq,shape))+'0000'
    remain=len(swb)-1;escape=7 if seq==2 else 31;size=3 if seq==2 else 5
    while remain>=escape:result+=field(escape,size);remain-=escape
    return result+field(remain,size)+'00'+'1'+gain_bits(seq,active,c)
def packet(frame,seq,target_shape,source_shape,channels,point,active,missing=False,bad_gain=False,tags=(1,),source_sequence=None):
    target='0000000'+silent(seq,target_shape,0,active,False) if channels==1 else '0010000'+'1'+info(seq,target_shape)+'00'+''.join(silent(seq,target_shape,c,active,True) for c in range(2))
    sources=[]
    source_seq=seq if source_sequence is None else source_sequence
    for tag in tags:
        source='010'+field(tag,4)+field(point==3,1)+'000'+field(channels==2,1)+field(1 if missing else 0,4)+('00' if channels==2 else '')+field(point==1,1)+'0'+'10'+channel(frame,source_seq,source_shape,0,active and point==3,False,bad_gain and tag==tags[-1])
        sources.append(source)
    source=''.join(sources)
    # Alternate source ordering; PCE is authoritative in either order.
    return packed((source+target if frame%2 else target+source)+'111')
def main():
    blob=bytearray();gold=bytearray();cases=[];invalid=[]
    for channels in [1,2]:
        for point in [0,1,3]:
            for active in [False,True]:
                shapes=[int(i%2==0) for i in range(6)]
                source_shapes=[1-s if point==3 else s for s in shapes]
                if point==3:
                    mono,frames=oracle(source_shapes,1,active)
                    pcm=bytearray()
                    for row in struct.iter_unpack('<f',mono):
                        pcm.extend(struct.pack('<f',row[0])*channels)
                    for frame in frames:frame['pcm_offset']*=channels
                else:pcm,frames=oracle(shapes,channels,active,lambda f,s,c:spectrum(f,s,0))
                off=len(gold);gold.extend(pcm)
                for i,seq in enumerate(SEQUENCES):
                    data=packet(i,seq,shapes[i],source_shapes[i],channels,point,active)
                    frames[i].update(offset=len(blob),bytes=len(data),sequence=seq,shape=shapes[i],source_shape=source_shapes[i]);blob.extend(data)
                case=dict(slots=16,bands=32,channels=channels,point=point,active=active,asc=config(channels,point).hex(),frames=frames,pcm_offset=off,pcm_bytes=len(pcm),samples=6144,container_rate=24000,container_frame_samples=1472,durations=[SAMPLES[s] for s in SEQUENCES])
                case['video']=video_fixture([case],blob,channels=channels,filename=f'aac-ssr-coupling-{channels}-{point}-{int(active)}-synthetic.mp4');cases.append(case)
    for channels in [1,2]:
        base=next(c for c in cases if c['channels']==channels and c['point']==3 and c['active'])
        pcm=bytearray()
        for (value,) in struct.iter_unpack('<f',gold[base['pcm_offset']:base['pcm_offset']+base['pcm_bytes']]):pcm.extend(struct.pack('<f',value*2))
        frames=[];off=len(gold);gold.extend(pcm)
        for i,seq in enumerate(SEQUENCES):
            source=base['frames'][i];data=packet(i,seq,source['shape'],source['source_shape'],channels,3,True,tags=(1,15))
            frames.append(dict(source,offset=len(blob),bytes=len(data)));blob.extend(data)
        case=dict(base,asc=config(channels,3,(1,15)).hex(),frames=frames,pcm_offset=off,tags=[1,15])
        case['video']=video_fixture([case],blob,channels=channels,filename=f'aac-ssr-coupling-multiple-{channels}-synthetic.mp4');cases.append(case)
    for kind in ['missing','shape','gain']:
        point=0 if kind=='shape' else 3
        base=next(c for c in cases if c['channels']==1 and c['point']==point and c['active'] and (kind=='shape' or c.get('tags')==[1,15]))
        frames=[]
        for i,seq in enumerate(SEQUENCES):
            shape=base['frames'][i]['shape'];source_shape=base['frames'][i]['source_shape']
            if kind=='shape' and i==1:source_shape=1-source_shape
            data=packet(i,seq,shape,source_shape,1,point,True,missing=kind=='missing' and i==1,bad_gain=kind=='gain' and i==1,tags=base.get('tags',(1,)))
            frames.append(dict(offset=len(blob),bytes=len(data),samples=SAMPLES[seq]));blob.extend(data)
        case=dict(base,frames=frames)
        case['error']={'missing':'AAC coupling target is absent','shape':'AAC SSR dependent coupling window shape mismatch','gain':'AAC SSR invalid gain adjustment location or level'}[kind]
        case['video']=video_fixture([case],blob,channels=1,filename=f'aac-ssr-coupling-invalid-{kind}-synthetic.mp4');invalid.append(case)
    (DEST/'aac-ssr-coupling-packets.bin').write_bytes(blob);(DEST/'aac-ssr-coupling-pcm.f32le').write_bytes(gold)
    (DEST/'aac-ssr-coupling.json').write_text(json.dumps(dict(cases=cases,invalid=invalid,packet_sha256=hashlib.sha256(blob).hexdigest(),pcm_sha256=hashlib.sha256(gold).hexdigest(),oracle='independent scalar SSR synthesis on source or target spectra; no production decoder'),indent=2)+'\n')
    print(len(cases),'SSR coupling acceptance videos;',len(invalid),'invalid videos')
if __name__=='__main__':main()
