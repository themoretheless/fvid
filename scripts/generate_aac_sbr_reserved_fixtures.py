#!/usr/bin/env python3
"""Own reserved SBR extension IDs, escaped lengths and bounded truncations."""
import json
from generate_he_aac_packet_fixtures import DEST,field,frequency,packed,video_fixture
from generate_aac_sbr_data_fixtures import word
from generate_aac_sbr_dsp_fixtures import header
from generate_aac_sbr_frequency_oracles import tables

def main():
    blob=bytearray();cases=[]
    def store(data):
        row=dict(offset=len(blob),bytes=len(data));blob.extend(data);return row
    _,high,_,_=tables(10,27,0,False,0,0)
    channel=field(100,8)+'0000'+'000000'+'0'+'000'
    for aot in (2,17,19):
      for slots in (15,16):
       for width in (1,2):
        rows=[];controls=[];bad=[]
        for frame,(id,count) in enumerate([(0,1),(1,15),(3,240 if aot==2 else 270)]):
            temporal=frame>0
            env=(word(0,0)*(len(high)-1) if temporal else field(2,7)+word(1,0)*(len(high)-2))*width
            noise=(word(8,0) if temporal else field(7,5))*width
            data='0'+('0' if width==2 else '')+'00001'*width+field(temporal,1)*(2*width)+'00'*width+env+noise+'0'*width
            start=field(13,4)+field(frame==0,1)+(header(0,True) if frame==0 else '')+data
            # A reserved ID consumes all following bits, including fake PS IDs.
            area=field(id,2)+'101101'+'10000000'*(count-1)
            ext='1'+field(min(count,15),4)+(field(count-15,8) if count>=15 else '')+area
            raw=packed(start+ext);base=packed(start+'0')
            core=field(1,4)+('0'+channel*2 if width==2 else channel)
            def packet(payload):
                bits=''.join(field(v,8) for v in payload)
                if aot==2:
                    n=len(payload);assert n<=269
                    return packed(field(width==2,3)+core+'110'+(field(n,4) if n<15 else '1111'+field(n-14,8))+bits+'111')
                return packed(core+bits)
            rows.append(store(packet(raw)));controls.append(store(packet(base)))
            # Shorter enclosing packet retains the original declared length.
            bad.append(store(packet(raw[:-1])))
        asc=packed(field(5,5)+frequency(24000)+field(width,4)+frequency(48000)+field(aot,5)+field(slots==15,1)+'00'+('00' if aot!=2 else '')).hex()
        name=f'{aot}-{slots}-{width}'
        c=dict(name=name,aot=aot,asc=asc,control_asc=asc,frames=rows,control_frames=controls,bad_frames=bad,channels=width,bands=64,slots=slots,pcm_offset=0,samples=3*slots*128,container_rate=48000,container_frame_samples=slots*128)
        for key,frames,suffix in [('video',rows,''),('control_video',controls,'-control'),('bad_video',bad,'-truncated')]:
            c[key]=video_fixture([dict(c,frames=frames)],blob,channels=width,filename=f'aac-sbr-reserved-{name}{suffix}-synthetic.mp4')
        cases.append(c)
    (DEST/'aac-sbr-reserved-packets.bin').write_bytes(blob)
    (DEST/'aac-sbr-reserved.json').write_text(json.dumps(dict(cases=cases,provenance='Own ordinary LC and ER LC/LTP, mono/CPE, 960/1024 core samples. Reserved SBR IDs 0/1/3 consume opaque fill containing fake PS markers. Lengths 1/15/240 ordinary and 1/15/270 ER; matched absent-extension controls and declared-area truncations. No private media, FFmpeg, foreign encoder or network.'),indent=2)+'\n')
if __name__=='__main__':main()
