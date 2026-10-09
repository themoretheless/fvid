#!/usr/bin/env python3
"""Own AAC LTP syntax and profile acceptance videos; no codec executables."""
import json
from generate_aac_main_tools_fixtures import channel,ics
from generate_he_aac_packet_fixtures import DEST,field,frequency,packed,video_fixture

def main():
    blob=bytearray();cases=[]
    for n in (960,1024):
        for lag in (0,n,min(2*n,2047)):
            for coefficient in range(8):
                for bands in (0,1,40,63):
                    used=[i%3==coefficient%3 for i in range(min(bands,40))]
                    raw='101'+field(lag,11)+field(coefficient,3)+''.join(str(int(v)) for v in used)
                    data=packed(raw);cases.append(dict(offset=len(blob),bytes=len(data),bits=len(raw),n=n,sequence=0,bands=bands,lag=lag,coefficient=coefficient,used=used));blob.extend(data)
        for coefficient in range(8):
            for pattern in range(3):
                windows=[];body=''
                for w in range(8):
                    enabled=pattern==0 or (pattern==1 and w%2==0)
                    offset=w*2 if enabled and w%3 else None
                    windows.append(dict(enabled=enabled,lag=offset))
                    body+=str(int(enabled))
                    if enabled:body+=str(int(offset is not None))+(field(offset,4) if offset is not None else '')
                raw='101'+field(n,11)+field(coefficient,3)+body;data=packed(raw)
                cases.append(dict(offset=len(blob),bytes=len(data),bits=len(raw),n=n,sequence=2,bands=2,lag=n,coefficient=coefficient,windows=windows));blob.extend(data)
    data=packed('101'+field(1921,11)+field(0,3)+'1')
    invalid=dict(offset=len(blob),bytes=len(data));blob.extend(data)
    (DEST/'aac-ltp-syntax.bin').write_bytes(blob)
    videos=[];packets=bytearray()
    for active in (False,True):
        rows=[]
        for i in range(12):
            info=ics(0,2,False)
            if active and i>=3:info=info[:-1]+'11'+field(1024,11)+field(i%8,3)+'11'
            raw=packed('0000000'+channel(0,[1,1],[[1,-1,1,-1,0,1,-1,1]],info=info)+'111')
            rows.append(dict(offset=len(packets),bytes=len(raw)));packets.extend(raw)
        c=dict(name='active' if active else 'inactive',asc=packed(field(4,5)+frequency(24000)+'0001'+'000').hex(),frames=rows,container_rate=24000,container_frame_samples=1024,channels=1,slots=16,bands=32,samples=12288,pcm_offset=0)
        c['video']=video_fixture([c],packets,filename='aac-ltp-'+c['name']+'-synthetic.mp4');videos.append(c)
    (DEST/'aac-ltp-packets.bin').write_bytes(packets)
    (DEST/'aac-ltp-syntax.json').write_text(json.dumps(dict(cases=cases,invalid_lag=invalid,videos=videos,provenance='Own ordinary LTP syntax and AOT4 videos; independent bit writer, no private source or external codecs. Ordinary native AOT4 playback admitted; full profile qualification remains separate.'),indent=2)+'\n')
if __name__=='__main__':main()
