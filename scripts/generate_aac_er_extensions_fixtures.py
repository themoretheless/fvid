#!/usr/bin/env python3
"""Authored ER trailing fill/ancillary, direct scalar PCM and bounded failures."""
import json,math,struct
from generate_aac_ltp_pns_fixtures import Oracle,prediction
from generate_aac_main_tools_fixtures import channel
from generate_he_aac_packet_fixtures import DEST,field,frequency,packed,video_fixture

def main(dynamic=False):
    blob=bytearray();gold=bytearray();cases=[]
    payloads=[bytes.fromhex('00'),bytes.fromhex('10a5a5'),b'\x20\x03ABC',bytes.fromhex('20014210a5'),b'\x20\xff\x00'+b'Z'*255,bytes.fromhex('2000')]
    prefix='aac-er-dynamic-range' if dynamic else 'aac-er-extensions'
    if dynamic:
        payloads=[]
        for flags in range(16):
            wire=field(11,4)+field(bool(flags&1),1)
            if flags&1: wire+=field(9,4)+field(0,4)
            wire+=field(bool(flags&2),1)
            if flags&2: wire+='10101011'+'01010100'
            wire+=field(bool(flags&4),1)
            bands=16 if flags&4 else 1
            if flags&4: wire+=field(15,4)+field(3,4)+''.join(field(15+16*i,8) for i in range(bands))
            wire+=field(bool(flags&8),1)
            if flags&8: wire+=field(73,7)+'0'
            wire+=''.join(field(i%2,1)+field((i*9+flags)%128,7) for i in range(bands))
            payloads.append(packed(wire)+bytes.fromhex('20014210a5'))
    frame_count=len(payloads) if dynamic else 12
    for aot in (17,19,23):
        for n in ((480,512) if aot==23 else (960,1024)):
            cos=[[math.cos(math.pi/n*(i+.5+n/2)*(k+.5)) for i in range(2*n)] for k in range(8)]
            oracle=Oracle(n,aot==23,cos);rows=[];controls=[];bad_rows=[];start=len(gold)
            for frame in range(frame_count):
                active=aot!=17 and frame>=1;lag=1023 if aot==23 else n-13;coef=frame%8
                q=[(frame+k)%3-1 for k in range(8)]
                base='0000'+field(2,6)
                info=base+field(active,1)+(prediction(aot,True,lag,coef,[True,True]) if active else '')
                wire='0000'+channel(0,[1,1],[q],info=info)
                for extra,listing in ((payloads[frame%len(payloads)],rows),(b'',controls),(bytes.fromhex('b2f0' if dynamic else '200500'),bad_rows)):
                    raw=packed(wire+''.join(field(v,8) for v in extra));listing.append(dict(offset=len(blob),bytes=len(raw),audio_bits=len(wire),extension=extra.hex()));blob.extend(raw)
                pcm=oracle.run([float(v*1024) for v in q],0,active,lag,coef,[True,True]);gold.extend(struct.pack('<'+'f'*n,*pcm))
            asc=packed(field(aot,5)+frequency(24000)+'0001'+field(n in (480,960),1)+'00'+'00').hex()
            name=f'{aot}-{n}';c=dict(name=name,aot=aot,n=n,asc=asc,channels=1,frames=rows,control_frames=controls,bad_frames=bad_rows,reference_offset=start,reference_bytes=len(gold)-start,container_rate=24000,container_frame_samples=n,samples=frame_count*n,slots=n//64,bands=32,pcm_offset=0)
            for key,frames,suffix in [('video',rows,''),('control_video',controls,'-control'),('bad_video',bad_rows,'-malformed')]:
                c[key]=video_fixture([dict(c,frames=frames)],blob,filename=f'{prefix}-{name}{suffix}-synthetic.mp4')
            cases.append(c)
    (DEST/f'{prefix}-packets.bin').write_bytes(blob);(DEST/f'{prefix}-reference.f32le').write_bytes(gold)
    (DEST/f'{prefix}.json').write_text(json.dumps(dict(cases=cases,provenance=('Own DRC all optional fields, 16 bands, continued exclusions and chained ancillary. ' if dynamic else '')+'Own ER LC/LTP/LD ep0 packets, trailing EXT_FILL/EXT_FILL_DATA/ANC_DATA, chained and escaped lengths, scalar cosine PCM and malformed '+('DRC band table truncation' if dynamic else 'ancillary overrun')+'. No private media, FFmpeg, foreign decoder or network.'),indent=2)+'\n')
    print('generated six ER extension videos, controls and malformed companions')
if __name__=='__main__':main()
