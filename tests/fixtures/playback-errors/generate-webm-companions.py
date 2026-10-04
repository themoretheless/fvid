"""Join committed synthetic WebM pictures and AAC/Opus packets; no encoders."""
from pathlib import Path
import struct
root=Path(__file__).parent

def vint(n):
    for width in range(1,9):
        if n<(1<<(7*width))-1:return ((1<<(7*width))|n).to_bytes(width,'big')
    raise ValueError('size overflow')
def atom(kind,payload):return kind.to_bytes((kind.bit_length()+7)//8,'big')+vint(len(payload))+payload
def uint(kind,n):return atom(kind,n.to_bytes(max(1,(n.bit_length()+7)//8),'big'))
def integer(data,pos,size):
    first=data[pos];width=1
    while not first&(1<<(8-width)):width+=1
    value=int.from_bytes(data[pos:pos+width],'big')
    if size:value&=(1<<(7*width))-1
    return value,pos+width

def fields(data):
    at=0
    while at<len(data):
        kind,pos=integer(data,at,False);size,start=integer(data,pos,True);end=min(start+size,len(data))
        yield kind,data[start:end]
        at=end

def contents(data,kind):return next(body for tag,body in fields(data) if tag==kind)
def number(data,kind,default=0):return next((int.from_bytes(b,'big') for k,b in fields(data) if k==kind),default)
def packets(data):
    segment=contents(data,0x18538067);info=contents(segment,0x1549a966)
    scale=number(info,0x2ad7b1,1_000_000)
    entries=list(fields(contents(segment,0x1654ae6b)))
    tracks={number(t,0xd7):t for k,t in entries if k==0xae}
    out=[]
    for kind,cluster in fields(segment):
        if kind!=0x1f43b675:continue
        base=number(cluster,0xe7)
        for kind,body in fields(cluster):
            duration=None;padding=None
            if kind==0xa0:
                duration=number(body,0x9b)*scale or None
                padding=next((int.from_bytes(v,'big',signed=True) for k,v in fields(body) if k==0x75a2),None)
                body=contents(body,0xa1)
            elif kind!=0xa3:continue
            track,pos=integer(body,0,True);relative=int.from_bytes(body[pos:pos+2],'big',signed=True);flags=body[pos+2]
            assert flags&6==0,'source must not be laced'
            out.append((track,(base+relative)*scale,flags,body[pos+3:],duration,padding))
    return tracks,out

def entry(track,identifier,strip_duration=False):
    # The joined synthetic file explicitly declares default tracks and und.
    return atom(0xae,uint(0xd7,identifier)+uint(0x73c5,identifier)+uint(0x88,1)+atom(0x22b59c,b'und')+b''.join(atom(k,b) for k,b in fields(track) if k not in [0xd7,0x73c5,0x88,0x22b59c,0x22b59d] and not (strip_duration and k==0x23e383)))
def block(track,pts,flags,payload,duration,padding=None):
    data=vint(track)+bytes(2)+bytes([flags])+payload
    group=atom(0xa1,data)+uint(0x9b,duration)
    if not flags&0x80:group+=atom(0xfb,(-1).to_bytes(1,'big',signed=True))
    if padding is not None:group+=atom(0x75a2,padding.to_bytes(8,'big',signed=True))
    return atom(0x1f43b675,uint(0xe7,pts)+atom(0xa0,group))

aac=(root.parent/'audio/aac-mono-44k.aac').read_bytes();at=0;audio=[];config=None
rates=[96000,88200,64000,48000,44100,32000,24000,22050,16000,12000,11025,8000,7350]
while at<len(aac):
    h=aac[at:at+7];assert h[0]==255 and h[1]&0xf6==0xf0
    profile=(h[2]>>6)+1;rate_index=(h[2]>>2)&15;channels=((h[2]&1)<<2)|(h[3]>>6)
    asc=((profile<<11)|(rate_index<<7)|(channels<<3)).to_bytes(2,'big')
    assert config is None or config==asc
    config=asc;rate=rates[rate_index];length=((h[3]&3)<<11)|(h[4]<<3)|(h[5]>>5);header=7 if h[1]&1 else 9
    assert h[6]&3==0
    audio.append(aac[at+header:at+length]);at+=length
assert at==len(aac)
aac_track=atom(0xae,uint(0xd7,2)+uint(0x73c5,2)+uint(0x83,2)+uint(0x88,1)+atom(0x86,b'A_AAC')+atom(0x63a2,config)+atom(0x22b59c,b'und')+uint(0x56aa,128*1_000_000_000//rate)+atom(0xe1,atom(0xb5,struct.pack('>d',rate))+uint(0x9f,channels)))
opus_tracks,opus_packets=packets((root/'framestep-opus.mkv').read_bytes())
opus_number=next(i for i,t in opus_tracks.items() if contents(t,0x86)==b'A_OPUS')
opus_track=entry(opus_tracks[opus_number],3,True)
opus_packets=[p for p in opus_packets if p[0]==opus_number];shift=max(0,-min(p[1] for p in opus_packets))
def opus_duration(payload):
    config=payload[0]>>3
    micros=(2500<<(config&3)) if config>=16 else (10000<<(config&1)) if config>=12 else [10000,20000,40000,60000][config&3]
    count=1 if payload[0]&3==0 else (payload[1]&63) if payload[0]&3==3 else 2
    assert 0<count and micros*count<=120000
    return micros*count*1000
for label,source in [('vp9',root.parent/'short/vp9-motion.webm'),('av1',root.parent/'av1/ramp.webm')]:
    source_bytes=source.read_bytes();tracks,video=packets(source_bytes);video_number=next(i for i,t in tracks.items() if number(t,0x83)==1)
    vtrack=entry(tracks[video_number],1);default=number(tracks[video_number],0x23e383)
    video_packets=[p for p in video if p[0]==video_number]
    info_source=contents(contents(source_bytes,0x18538067),0x1549a966)
    duration_bytes=contents(info_source,0x4489)
    declared_end=round(struct.unpack('>d' if len(duration_bytes)==8 else '>f',duration_bytes)[0]*number(info_source,0x2ad7b1,1_000_000))
    last=video_packets[-1][1]
    if declared_end<=last:
        assert len(video_packets)>1
        declared_end=last+(last-video_packets[-2][1])
    spans={p[1]:(video_packets[i+1][1] if i+1<len(video_packets) else declared_end)-p[1] for i,p in enumerate(video_packets)}
    assert all(span>0 for span in spans.values())
    scheduled=[]
    for original,pts,flags,payload,duration,padding in video:
        if original==video_number:scheduled.append((pts,1,block(1,pts,flags,payload,duration or default or spans[pts],padding)))
    for i,payload in enumerate(audio):
        begin=i*1024*1_000_000_000//rate;end=(i+1)*1024*1_000_000_000//rate
        scheduled.append((begin,2,block(2,begin,0x80,payload,end-begin,7*1_000_000_000//rate if i+1==len(audio) else None)))
    for _,pts,flags,payload,duration,padding in opus_packets:
        pts+=shift;scheduled.append((pts,3,block(3,pts,flags,payload,duration or opus_duration(payload),padding)))
    scheduled.sort(key=lambda p:(p[0],p[1]))
    end=max(p[1]+(p[4] or default or spans[p[1]]) for p in video if p[0]==video_number)
    info=uint(0x2ad7b1,1)+atom(0x4489,struct.pack('>d',float(end)))
    header=atom(0x1a45dfa3,atom(0x4282,b'matroska')+uint(0x4287,4)+uint(0x4285,2))
    def tag(key,value,uid=None):
        targets=atom(0x63c0,uint(0x63c5,uid)) if uid is not None else b''
        return atom(0x7373,targets+atom(0x67c8,atom(0x45a3,key.encode())+atom(0x4487,value.encode())))
    tags=atom(0x1254c367,tag('SOURCE_NOTE','public synthetic')+tag('ORIGINAL_NOTE','video',1)+tag('ORIGINAL_NOTE','aac',2)+tag('ORIGINAL_NOTE','opus',3))
    segment=atom(0x1549a966,info)+atom(0x1654ae6b,aac_track+vtrack+opus_track)+tags+b''.join(p[2] for p in scheduled)
    (root/f'shared-{label}-companions.mkv').write_bytes(header+atom(0x18538067,segment))

# Ordinary audio SimpleBlock-style timing: no BlockDuration/DefaultDuration.
# Keep BlockGroup here to retain signed DiscardPadding while omitting duration.
data=(root/'shared-vp9-companions.mkv').read_bytes()
segment=contents(data,0x18538067);parts=[]
for kind,body in fields(segment):
    if kind==0x1f43b675:
        rewritten=[]
        for child,payload in fields(body):
            if child==0xa0:
                block_data=contents(payload,0xa1);track,_=integer(block_data,0,True)
                if track!=1:payload=b''.join(atom(k,v) for k,v in fields(payload) if k!=0x9b)
            rewritten.append(atom(child,payload))
        body=b''.join(rewritten)
    parts.append(atom(kind,body))
(root/'shared-vp9-untimed-companions.mkv').write_bytes(atom(0x1a45dfa3,contents(data,0x1a45dfa3))+atom(0x18538067,b''.join(parts)))
