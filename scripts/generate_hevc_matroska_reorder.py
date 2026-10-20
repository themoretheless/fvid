#!/usr/bin/env python3
"""Own Matroska wrappers of saved authored IPB streams; no codec execution."""
import json,struct,hashlib
from generate_hevc_matroska_formats import D,unpack,boxes,child
from generate_hevc_matroska_timeline import fields,child as ebml_child
from generate_ffv1_gray_fixtures import element,uint

def main():
    cases=[]
    companion=ebml_child((D/'hevc-pcm-444-reference-rext8-timeline-r0-crop0-timed-companions.mkv').read_bytes(),0x18538067)
    audio_tracks=[]
    for k,v in fields(ebml_child(companion,0x1654ae6b)):
        if dict(fields(v)).get(0x86,b'').startswith(b'A_'):audio_tracks.append(element(k,v))
    audio_clusters=[]
    for k,v in fields(companion):
        if k!=0x1f43b675:continue
        if any((x==0xa3 and (p[0]&127)!=1) or x==0xa0 for x,p in fields(v)):audio_clusters.append(element(k,v))
    for depth,name in [(8,'main-ipb'),(10,'main10-ipb')]:
        path=D.parent/'hevc'/f'{name}.mp4';private,packets,w,h,_=unpack(path)
        moov=child(path.read_bytes(),b'moov');mdia=child(child(moov,b'trak'),b'mdia');stbl=child(child(mdia,b'minf'),b'stbl')
        duration=[];stts=child(stbl,b'stts')
        for at in range(8,len(stts),8):
            count,delta=struct.unpack('>II',stts[at:at+8]);duration += [delta]*count
        offsets=[];ctts=child(stbl,b'ctts')
        for at in range(8,len(ctts),8):
            count=int.from_bytes(ctts[at:at+4],'big');offset=int.from_bytes(ctts[at+4:at+8],'big',signed=ctts[0]==1);offsets += [offset]*count
        dts=0;pts=[]
        for delta,offset in zip(duration,offsets):pts.append(dts+offset);dts+=delta
        order=sorted(range(len(pts)),key=lambda i:pts[i]);rank={index:i for i,index in enumerate(order)}
        assert order!=list(range(len(order))) and len(set(pts))==len(pts)
        gold=(D.parent/'hevc'/f'{name}.yuv').read_bytes();unit=1 if depth==8 else 2;n=w*h*3//2*unit;assert len(gold)==n*len(packets)
        frames=[]
        for i in range(len(packets)):
            file=f'hevc-matroska-ipb-{depth}-{i}.yuv';(D/file).write_bytes(gold[i*n:(i+1)*n]);frames.append(file)
        for untimed in [False,True]:
            for end in [False,True]:
                for companions in [False,True]:
                    timeline=[i*37+i*i if untimed else i*40 for i in range(len(packets))]
                    last=(71 if end else timeline[-1]-timeline[-2]) if untimed else 40
                    durations=[b-a for a,b in zip(timeline,timeline[1:])]+[last]
                    info=uint(0x2ad7b1,1000000) if not companions else uint(0x2ad7b1,1)
                    scale=1 if companions else 1000000
                    if end:info+=element(0x4489,struct.pack('>d',(timeline[-1]+last)*1000000/scale))
                    track=uint(0xd7,1)+uint(0x83,1)+element(0x86,b'V_MPEGH/ISO/HEVC')+element(0x63a2,private)+element(0xe0,uint(0xb0,w)+uint(0xba,h))
                    if not untimed:track+=uint(0x23e383,40000000)
                    tracks=element(0xae,track)
                    if companions:tracks=audio_tracks[0]+tracks+audio_tracks[1]
                    clusters=b''.join(element(0x1f43b675,uint(0xe7,timeline[rank[i]]*1000000//scale)+element(0xa3,b'\x81\0\0'+(b'\x80' if i==0 else b'\0')+packet)) for i,packet in enumerate(packets))
                    if companions:clusters+=b''.join(audio_clusters)
                    data=element(0x1a45dfa3,element(0x4282,b'matroska'))+element(0x18538067,element(0x1549a966,info)+element(0x1654ae6b,tracks)+clusters)
                    file=f'hevc-matroska-ipb-{depth}-{int(untimed)}-{int(end)}-{int(companions)}.mkv';(D/file).write_bytes(data)
                    cases.append(dict(file=file,depth=depth,size=[w,h],untimed=untimed,segment_end=end,companions=companions,decode_to_display=[rank[i] for i in range(len(packets))],frames=frames,pts_ns=[t*1000000 for t in timeline],durations_ns=[d*1000000 for d in durations],sha256=hashlib.sha256(data).hexdigest()))
    (D/'hevc-matroska-reorder.json').write_text(json.dumps(dict(cases=cases,provenance='Existing authored main/main10 IPB packets and saved synthetic decoder oracles only. MP4 CTTS establishes original presentation order; own Matroska rebinding preserves decode order. Ordinary generator uses no codec, private data, FFmpeg or network.'),indent=2)+'\n')
if __name__=='__main__':main()
