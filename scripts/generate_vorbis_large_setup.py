#!/usr/bin/env python3
"""Authored large sparse Vorbis setups, Ogg continuations and short HEVC videos.
Offline; no private media/parameters, external codecs, encoders or network.
"""
import hashlib,json,struct
from generate_vorbis_timestamps import bits
from generate_hevc_matroska_formats import D,unpack
from generate_ffv1_gray_fixtures import element,uint

def page(payload,granule,flags,sequence,serial,laces):
    data=bytearray(b'OggS'+bytes([0,flags])+struct.pack('<qII',granule,serial,sequence)+bytes(4)+bytes([len(laces)])+bytes(laces)+payload)
    crc=0
    for b in data:
        crc^=b<<24
        for _ in range(8):crc=((crc<<1)^0x04c11db7 if crc&0x80000000 else crc<<1)&0xffffffff
    data[22:26]=struct.pack('<I',crc);return bytes(data)
def packet(payload,granule,flags,sequence,serial):
    out=b'';at=0;continued=False
    while len(payload)-at>=65025:
        out+=page(payload[at:at+65025],-1,(flags&2)|(1 if continued else 0),sequence,serial,[255]*255)
        at+=65025;sequence+=1;continued=True
    tail=payload[at:];laces=[255]*(len(tail)//255)+[len(tail)%255]
    out+=page(tail,granule,(flags&4)|(1 if continued else flags&2),sequence,serial,laces)
    return out,sequence+1

def main():
    ident=b'\x01vorbis'+struct.pack('<IBIiiiBB',0,2,8000,0,0,0,0x86,1)
    comment=b'\x03vorbis'+bytes(8)+b'\x01'
    tail=[(0,6),(0,16),(0,6),(1,16),(0,5),(0,2),(6,4),(0,6),(0,16),(0,24),(128,24),(31,24),(0,6),(0,8),(0,3),(0,1),(0,6),(0,16),(0,1),(0,1),(0,2),(0,8),(0,8),(0,8),(2,6)]
    for long in [0,0,1]:tail.extend([(long,1),(0,16),(0,16),(0,8)])
    tail.append((1,1));modes=[0,2,1,2,2,0];audio=[]
    for i,mode in enumerate(modes):
        header=[(0,1),(mode,2)]
        if mode==2:header.extend([(int(i>0 and modes[i-1]==2),1),(int(i+1<len(modes)and modes[i+1]==2),1)])
        audio.append(bits(header+[(0,1),(0,1)]))
    private,video,w,h,_=unpack(D/'hevc-pcm-reference-rext8.mp4')
    vt=uint(0xd7,1)+uint(0x83,1)+element(0x86,b'V_MPEGH/ISO/HEVC')+element(0x63a2,private)+element(0xe0,uint(0xb0,w)+uint(0xba,h))
    fixtures=[]
    for index,(name,entries,dimensions,lookup) in enumerate([('entries',1000001,1,0),('multiplicands',33,32768,2)]):
        fields=[(5,8)]+[(b,8)for b in b'vorbis']+[(0,8),(0x564342,24),(dimensions,16),(entries,24),(0,1),(1,1),(1,1),(0,5)]+[(0,1)]*(entries-1)+[(lookup,4)]
        if lookup:fields.extend([(0,32),((788<<21)|1,32),(0,4),(0,1)]+[(0,1)]*(entries*dimensions))
        setup=bits(fields+tail);serial=0x46565b00+index;ogg=b'';sequence=0
        for payload,granule,flags in [(ident,0,2),(comment,0,0),(setup,0,0)]+[(p,411 if i==5 else 80 if i==1 else -1,4 if i==5 else 0)for i,p in enumerate(audio)]:
            chunk,sequence=packet(payload,granule,flags,sequence,serial);ogg+=chunk
        ogg_name=f'vorbis-large-{name}.ogg';(D/ogg_name).write_bytes(ogg)
        config=bytes([2,len(ident),len(comment)])+ident+comment+setup
        track=uint(0xd7,2)+uint(0x83,2)+element(0x86,b'A_VORBIS')+element(0x63a2,config)+element(0xe1,element(0xb5,struct.pack('>d',8000.))+uint(0x9f,2))
        parts=b''.join(element(0x1f43b675,uint(0xe7,i*40)+element(0xa3,b'\x81\0\0\x80'+p))for i,p in enumerate(video))
        for i,(t,p)in enumerate(zip([0,0,80,160,240,368],audio)):
            block=b'\x82\0\0\x80'+p
            payload=element(0xa0,element(0xa1,block)+element(0x75a2,(4625000).to_bytes(8,'big',signed=True)))if i==5 else element(0xa3,block)
            parts+=element(0x1f43b675,uint(0xe7,t//8)+payload)
        movie=element(0x1a45dfa3,element(0x4282,b'matroska'))+element(0x18538067,element(0x1654ae6b,element(0xae,vt)+element(0xae,track))+parts)
        video_name=f'hevc-vorbis-large-{name}.mkv';(D/video_name).write_bytes(movie)
        fixtures.append(dict(file=ogg_name,video=video_name,entries=entries,dimensions=dimensions,multiplicands=entries*dimensions if lookup else 0,setup_bytes=len(setup),pages=sequence,sha256=hashlib.sha256(ogg).hexdigest(),video_sha256=hashlib.sha256(movie).hexdigest()))
    (D/'vorbis-large-setup.json').write_text(json.dumps(dict(generator=__file__,fixtures=fixtures),indent=2)+'\n')
if __name__=='__main__':main()
