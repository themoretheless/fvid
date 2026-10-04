"""Attach presentation edits to checked-in synthetic MP4, preserving mdat offsets.
The old moov becomes a same-size free box; the rebuilt moov is appended.
No private media, codec parameters, encoder or network is used.
"""
from pathlib import Path
root=Path(__file__).parent

def boxes(data):
    at=0
    while at<len(data):
        size=int.from_bytes(data[at:at+4],'big')
        if size<8 or at+size>len(data):raise ValueError('unsupported synthetic box layout')
        yield data[at+4:at+8],data[at+8:at+size],at,size
        at+=size

def box(kind,payload):return (len(payload)+8).to_bytes(4,'big')+kind+payload

def edit_file(source,ranges):
    data=source.read_bytes()
    kind,movie,position,size=next(v for v in boxes(data) if v[0]==b'moov')
    mvhd=next(v[1] for v in boxes(movie) if v[0]==b'mvhd')
    old_scale=int.from_bytes(mvhd[12:16],'big')
    scale=1000
    if mvhd[0]!=0:raise ValueError('unsupported movie header version')
    output=b''
    for kind,payload,_,_ in boxes(movie):
        if kind==b'mvhd':
            payload=bytearray(payload)
            duration=int.from_bytes(payload[16:20],'big')
            payload[12:16]=scale.to_bytes(4,'big')
            payload[16:20]=((duration*scale+old_scale-1)//old_scale).to_bytes(4,'big')
        if kind==b'trak':
            rebuilt=b''
            for child,body,_,_ in boxes(payload):
                if child==b'tkhd':
                    body=bytearray(body)
                    if body[0]!=0:raise ValueError('unsupported track header version')
                    duration=int.from_bytes(body[20:24],'big')
                    body[20:24]=((duration*scale+old_scale-1)//old_scale).to_bytes(4,'big')
                rebuilt+=box(child,body)
            payload=rebuilt
            mdia=next(v[1] for v in boxes(payload) if v[0]==b'mdia')
            handler=next(v[1] for v in boxes(mdia) if v[0]==b'hdlr')
            if handler[8:12]==b'vide':
                mdhd=next(v[1] for v in boxes(mdia) if v[0]==b'mdhd')
                track_scale=int.from_bytes(mdhd[12:16],'big')
                minf=next(v[1] for v in boxes(mdia) if v[0]==b'minf')
                stbl=next(v[1] for v in boxes(minf) if v[0]==b'stbl')
                stts=next(v[1] for v in boxes(stbl) if v[0]==b'stts')
                durations=[]
                for at in range(8,len(stts),8):
                    durations.extend([int.from_bytes(stts[at+4:at+8],'big')]*int.from_bytes(stts[at:at+4],'big'))
                ctts=next((v[1] for v in boxes(stbl) if v[0]==b'ctts'),None)
                offsets=[]
                if ctts:
                    for at in range(8,len(ctts),8):
                        offsets.extend([int.from_bytes(ctts[at+4:at+8],'big',signed=ctts[0]==1)]*int.from_bytes(ctts[at:at+4],'big'))
                else:offsets=[0]*len(durations)
                if len(offsets)!=len(durations):raise ValueError('bad synthetic timing table')
                dts=0
                pts=[]
                for duration,offset in zip(durations,offsets):
                    pts.append(dts+offset)
                    dts+=duration
                origin=min(pts)
                edits=[]
                for duration_ms,start_ms,extra_ticks in ranges:
                    duration=duration_ms*scale//1000
                    start=-1 if start_ms<0 else origin+start_ms*track_scale//1000+extra_ticks
                    edits.append(duration.to_bytes(8,'big')+start.to_bytes(8,'big',signed=True)+b'\0\1\0\0')
                elst=box(b'elst',b'\1\0\0\0'+len(edits).to_bytes(4,'big')+b''.join(edits))
                payload=b''.join(box(k,p) for k,p,_,_ in boxes(payload) if k!=b'edts')+box(b'edts',elst)
        output+=box(kind,payload)
    data=bytearray(data)
    data[position+4:position+8]=b'free'
    return data+box(b'moov',output)

for name,source,ranges in [
    ('shared-edit-repeat','shared-avc-baseline.mp4',[(250,0,0),(250,0,0)]),
    ('shared-edit-disjoint','shared-avc-bframes.mp4',[(250,0,0),(250,500,0)]),
    ('shared-edit-fractional','shared-hevc-main.mp4',[(41,40,1),(1,120,1)]),
    ('shared-edit-leading','shared-hevc-main10.mp4',[(200,-1,0),(100,0,0)]),
    ('shared-edit-interior-empty','shared-avc-baseline.mp4',[(120,0,0),(120,-1,0),(120,0,0)]),
]:
    (root/(name+'.mp4')).write_bytes(edit_file(root/source,ranges))
