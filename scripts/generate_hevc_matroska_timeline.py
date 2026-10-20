#!/usr/bin/env python3
"""Offline authored HEVC wrappers; reuse only synthetic AAC/Opus packets."""
import json,struct,hashlib
from generate_hevc_matroska_formats import D,unpack
from generate_ffv1_gray_fixtures import element,uint

def vint(data,at,size=False):
    first=data[at];n=1
    while not first&(1<<(8-n)):n+=1
    value=int.from_bytes(data[at:at+n],'big')
    return (value&((1<<(7*n))-1) if size else value),at+n

def fields(data):
    at=0
    while at<len(data):
        tag,at=vint(data,at);n,at=vint(data,at,True)
        assert at+n<=len(data)
        yield tag,data[at:at+n];at+=n

def child(data,tag):return next(v for k,v in fields(data) if k==tag)
def main():
    companion=(D/'shared-vp9-companions.mkv').read_bytes();seg=child(companion,0x18538067)
    audio=[]
    for tag,body in fields(child(seg,0x1654ae6b)):
        f=dict(fields(body))
        if f.get(0x86,b'').startswith(b'A_'):audio.append(element(tag,body))
    assert int.from_bytes(child(child(seg,0x1549a966),0x2ad7b1),'big')==1
    audio_clusters=[]
    for tag,body in fields(seg):
        if tag!=0x1f43b675:continue
        # Keep a short authored prefix whose complete audio packets end before
        # the independently declared 146 ms Segment boundary.
        if int.from_bytes(child(body,0xe7),'big')>=120000000:continue
        blocks=[]
        for k,v in fields(body):
            block=v if k==0xa3 else child(v,0xa1) if k==0xa0 else None
            if block is not None:
                track,_=vint(block,0,True)
                if track in [2,3]:blocks.append(element(k,v))
        if blocks:audio_clusters.append(element(0x1f43b675,element(0xe7,child(body,0xe7))+b''.join(blocks)))
    assert len(audio)==2 and audio_clusters
    src=json.loads((D/'hevc-matroska-formats.json').read_text())['cases']
    rgb=json.loads((D/'hevc-identity-rgb.json').read_text())['cases']
    selected=[c for c in src if c['depth']==8]
    selected += [c for c in rgb if ((c['depth']==8) or (c['depth']==14 and c['full_range']) or (c['depth']==16 and not c['full_range']))]
    cases=[]
    for c in selected:
        source=c.get('mp4',c.get('source'));private,packets,w,h,rate=unpack(D/source);assert len(packets)==3
        for untimed in [False,True]:
            for companions in [False,True]:
                pts=[0,37,91] if untimed else [0,40,80];duration=[37,54,55] if untimed else [40]*3
                video=uint(0xb0,w)+uint(0xba,h)+b''.join(uint(tag,n) for tag,n in zip([0x54cc,0x54bb,0x54dd,0x54aa],c['crop']))
                rotation=c['rotation'];video+=element(0x7670,uint(0x7671,0)+element(0x7675,struct.pack('>d',90. if rotation==270 else -float(rotation))))
                track=uint(0xd7,1)+uint(0x83,1)+element(0x86,b'V_MPEGH/ISO/HEVC')+element(0x63a2,private)+element(0xe0,video)
                if not untimed:track+=uint(0x23e383,40000000)
                tracks=element(0xae,track)
                if companions:tracks=audio[0]+tracks+audio[1]
                payload=b''.join(element(0x1f43b675,uint(0xe7,t*1000000)+element(0xa3,b'\x81\0\0'+(b'\x80' if i==0 else b'\0')+p)) for i,(t,p) in enumerate(zip(pts,packets)))
                if companions:payload+=b''.join(audio_clusters)
                info=uint(0x2ad7b1,1)+element(0x4489,struct.pack('>d',146000000.))
                data=element(0x1a45dfa3,element(0x4282,b'matroska'))+element(0x18538067,element(0x1549a966,info)+element(0x1654ae6b,tracks)+payload)
                name=source[:-4]+f'-timeline-r{rotation}-crop{c["crop"][0]}'+('-untimed' if untimed else '-timed')+('-companions' if companions else '-single')+'.mkv';(D/name).write_bytes(data)
                cases.append(dict(file=name,rgb='mp4'in c,depth=c['depth'],source=c,pts_ns=[t*1000000 for t in pts],durations_ns=[t*1000000 for t in duration],companions=companions,untimed=untimed,sha256=hashlib.sha256(data).hexdigest()))
    (D/'hevc-matroska-timeline.json').write_text(json.dumps(dict(cases=cases,provenance='Only authored HEVC PCM and existing synthetic AAC/Opus containers. Own finite EBML reader/writer; 37/54/55 ms irregular intervals, explicit segment end. No private media, codecs run, FFmpeg or network.'),indent=2)+'\n')
if __name__=='__main__':main()
