#!/usr/bin/env python3
"""Own PS/CCE videos; independent direct-cosine core plus qualified PS stage."""
import hashlib
import json
import struct
from generate_he_aac_dependent_coupling_fixtures import (
    DEST, field, frequency, packed, info, word, codes, lens, core, mono_payload, tables, video_fixture,
)
from generate_aac_ps_fixtures import ps, sbr


def program(prefix, tags, point):
    bits = (prefix + field(0,4)+field(1,2)+frequency(24000)+field(1,4)
            +field(0,4)+field(0,4)+field(0,2)+field(0,3)+field(len(tags),4)
            +'000'+'0'+field(3,4))
    bits += ''.join(field(point==3,1)+field(tag,4) for tag in tags)
    return bits+'0'*(-len(bits)%8)+field(0,8)


def config(slots, rate, kind, tags, point):
    ga=field(slots==15,1)+'00'
    prefix=(field(29 if kind=='PS' else 5,5)+frequency(24000)+'0000'+frequency(rate)+field(2,5)+ga
            if kind!='LC' else field(2,5)+frequency(24000)+'0000'+ga)
    return packed(program(prefix,tags,point))


def channel(frame, silent, tns):
    bits=field(140,8)+info()+field(0 if silent else 1,4)+field(1,5)
    if not silent:bits+=word(60)
    bits+='0'+('1'+field(1,2)+'0'+field(49,6)+field(1,5)+'00'+field(1,3) if tns else '0')+'0'
    if not silent:
        index=80 if frame%2==0 else 0
        bits+=field(codes[index],lens[index])
    return bits


def fill(raw):
    if not raw:return ''
    size=len(raw)
    return '110'+(field(size,4) if size<15 else '1111'+field(size-14,8))+''.join(field(b,8) for b in raw)


def packet(frame,tags,point,target_raw,cce_raw,missing=False):
    target='000'+field(3,4)+channel(frame,True,True)+fill(target_raw)
    sources=[]
    for tag in tags:
        text=('010'+field(tag,4)+field(point==3,1)+'000'+'0'+field(4 if missing and tag==tags[-1] else 3,4)
              +field(point==1,1)+'000'+channel(frame,False,False)+fill(cce_raw[tag]))
        sources.append(text)
    if frame%2:sources.reverse()
    # CCE appears before or after target; target FIL is bound to that target.
    text=program('101',tags,point)+(''.join(sources)+target if frame%2 else target+''.join(sources))
    return packed(text+'111')


def main():
    blob,pcm,cases=bytearray(),bytearray(),[]
    nhigh=len(tables(10,27,0,False,0,0)[1])-1
    offsets={}
    for slots in [15,16]:
        for point in [0,1]:
            data=core(slots*64,False,point)
            offsets[slots,point]=[len(pcm),len(data)//4];pcm.extend(data)
        for point in [0,1,3]:
            for mode,tags in enumerate([[1],[15],[1,15]]):
                frames=[]
                for frame in range(6):
                    target_raw=sbr(ps(1,1,frame,slots=slots*2,count=1)[0],frame)
                    if (slots+point)%2==0 and frame==2:target_raw=b''
                    cce_raw={tag:(mono_payload(nhigh,2,True,frame) if mode and (mode==1 or frame!=2) else b'') for tag in tags}
                    data=packet(frame,tags,point,target_raw,cce_raw)
                    frames.append(dict(offset=len(blob),bytes=len(data),sbr=target_raw.hex(),
                                       cce=[dict(tag=tag,sbr=cce_raw[tag].hex()) for tag in tags]))
                    blob.extend(data)
                for kind in ['PS','SBR','LC']:
                    for rate,bands in [(24000,32),(48000,64)]:
                        c=dict(slots=slots,bands=bands,frames=frames,asc=config(slots,rate,kind,tags,point).hex(),
                               point=point,tags=tags,kind=kind,output_rate=rate,channels=2,
                               core_pcm=offsets[slots,0 if point==0 else 1],pcm_offset=0,samples=slots*bands*12,video=None)
                        if rate==48000:
                            c['video']=video_fixture([c],blob,channels=2,filename=f'he-aac-ps-coupling-{slots*64}-{point}-{mode}-{kind.lower()}-synthetic.mp4')
                        cases.append(c)
    negotiation=[]
    for slots in [15,16]:
        for point in [0,1,3]:
            for late in [False, True]:
                frames=[]
                for frame in range(6):
                    target_raw=sbr(ps(1,1,frame-1,slots=slots*2,count=1)[0],frame-1) if late and frame else b''
                    source=mono_payload(nhigh,2,True,frame)
                    data=packet(frame,[15],point,target_raw,{15:source})
                    frames.append(dict(offset=len(blob),bytes=len(data),sbr=target_raw.hex(),
                                       cce=[dict(tag=15,sbr=source.hex())]));blob.extend(data)
                c=dict(slots=slots,bands=64,frames=frames,asc=config(slots,48000,'LC',[15],point).hex(),
                       point=point,tags=[15],kind='LC',output_rate=48000,channels=2 if late else 1,
                       core_pcm=offsets[slots,0 if point==0 else 1],pcm_offset=0,samples=slots*64*12,late=late)
                c['video']=video_fixture([c],blob,channels=c['channels'],
                    filename=f"he-aac-ps-coupling-{slots*64}-{point}-{'late-target-ps' if late else 'source-fil-only'}-synthetic.mp4")
                negotiation.append(c)
    multiple=[]
    for slots in [15,16]:
        frames=[]
        for frame in range(6):
            first=ps(0,0,0,slots=slots*2,count=1)[0]
            last=ps(5,5,0,slots=slots*2,count=1)[0]
            target_raw=sbr(first+'10'+last,frame)
            baseline=sbr(last,frame)
            data=packet(frame,[15],1,target_raw,{15:b''})
            frames.append(dict(offset=len(blob),bytes=len(data),sbr=target_raw.hex(),
                               baseline_sbr=baseline.hex(),cce=[dict(tag=15,sbr='')]));blob.extend(data)
        for rate,bands in [(24000,32),(48000,64)]:
            for kind in ['PS','SBR','LC']:
                c=dict(slots=slots,bands=bands,frames=frames,asc=config(slots,rate,kind,[15],1).hex(),
                       point=1,tags=[15],kind=kind,output_rate=rate,channels=2,
                       core_pcm=offsets[slots,1],pcm_offset=0,samples=slots*bands*12,video=None)
                if rate==48000:
                    c['video']=video_fixture([c],blob,channels=2,
                        filename=f'he-aac-ps-coupling-{slots*64}-{kind.lower()}-multiple-ps-synthetic.mp4')
                multiple.append(c)
    invalid=[]
    for failure in ['target','crc','source-ps','last-ps']:
        c=next(c for c in cases if c['slots']==16 and c['point']==1 and c['tags']==[15] and c['kind']=='PS' and c['bands']==64)
        frames=[]
        for frame in range(3):
            target_raw=sbr(ps(1,1,frame,slots=32,count=1)[0],frame)
            source=bytearray(mono_payload(nhigh,2,True,frame))
            if frame==1:
                if failure=='last-ps':target_raw=sbr(ps(0,0,0,slots=32,count=1)[0]+'10'+'11111',frame)
                elif failure=='crc':source[0]^=1
                elif failure=='source-ps':source=bytearray(sbr(ps(1,1,0,slots=32,count=1)[0],0))
            data=packet(frame,[15],1,target_raw,{15:bytes(source)},missing=failure=='target' and frame==1)
            frames.append(dict(offset=len(blob),bytes=len(data),sbr=target_raw.hex()));blob.extend(data)
        bad=dict(c,frames=frames,samples=6144)
        bad['error']={'target':'AAC coupling target is absent','crc':'SBR CRC','source-ps':'SBR extended audio/PS synthesis is not yet implemented','last-ps':'reserved PS IID mode'}[failure]
        bad['video']=video_fixture([bad],blob,channels=2,filename=f'he-aac-ps-coupling-{failure}-synthetic.mp4')
        invalid.append(bad)
    (DEST/'he-aac-ps-coupling-packets.bin').write_bytes(blob)
    (DEST/'he-aac-ps-coupling-core.f32le').write_bytes(pcm)
    (DEST/'he-aac-ps-coupling-oracles.json').write_text(json.dumps(dict(
        kind='own PS/CCE packets; independent direct-cosine core, separately qualified PS/SBR composition',
        cases=cases,invalid=invalid,negotiation=negotiation,multiple=multiple,packet_sha256=hashlib.sha256(blob).hexdigest(),core_sha256=hashlib.sha256(pcm).hexdigest()),indent=2)+'\n')
    print(len(cases),'PS/CCE cases;',sum(c['video'] is not None for c in cases),'acceptance videos')


if __name__=='__main__':main()
