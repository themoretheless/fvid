#!/usr/bin/env python3
"""Authored ADTS protected regions, GF(2) division oracle and companion videos.
ISO/IEC 13818-7:2004 8.1.1.1; offline, no external codec or private media.
"""
import json,hashlib
from generate_aac_ssr_fixtures import DEST,field,frequency,packed,channel as ssr_channel,info as ssr_info
from generate_aac_main_tools_fixtures import channel,ics
from generate_aac_pce_profile_fixtures import program,config
from generate_he_aac_packet_fixtures import video_fixture

def polynomial(bits):
    # Literal polynomial long division, independent of a shift-register implementation.
    dividend=(0xffff<<len(bits))^(int(bits,2)<<16)
    while dividend.bit_length()>16:dividend^=0x18005<<(dividend.bit_length()-17)
    return dividend

def main():
    cases=[];blob=bytearray()
    scenarios=[(obj,ch,bands,False) for obj in (1,2) for ch in (1,2) for bands in (1,11)]
    scenarios += [(3,ch,0,False) for ch in (1,2)]+[(1,ch,1,True) for ch in (1,2)]
    scenarios=[(*s,True) for s in scenarios]+[(obj,ch,1 if obj!=3 else 0,False,False) for obj in (1,2,3) for ch in (1,2)]
    for obj,ch,bands,coupled,explicit in scenarios:
        name=f'{obj}-{ch}-{bands}-{int(coupled)}'+('' if explicit else '-indexed')
        asc=config(obj,ch,coupling=[(True,1)] if coupled else []) if explicit else packed(field(obj,5)+frequency(24000)+field(ch,4)+'000')
        packets=[];rows=[];adts=bytearray()
        for frame in range(4):
            wire=program('101',obj,ch,coupling=[(True,1)] if coupled else []) if explicit else ''
            spans=[dict(start=3,end=len(wire),width=len(wire)-3)] if explicit else []
            # DSE tag, alignment flag, byte count, byte alignment and private-free data.
            start=len(wire)+3;wire+='100'+'0000'+'1'+field(3,8);wire+='0'*(-len(wire)%8);wire+=''.join(field(b,8) for b in [0x15,0xa3,frame]);spans.append(dict(start=start,end=len(wire),width=len(wire)-start))
            # Unprotected FIL, followed by genuine own audio.
            wire+='110'+field(1,4)+'00000000'
            start=len(wire)+3;wire+=('000' if ch==1 else '001')+'0000'
            info=ssr_info(0,0) if obj==3 else ics(0,bands,False)
            if ch==2:wire+='1'+info+'00'
            values=[[1,-1,1,-1]*bands]
            def stream(c):return ssr_channel(frame,0,0,c,True,ch==2) if obj==3 else channel(0,[1]*bands,values,info=info if ch==1 else '')
            wire+=stream(0);right_start=len(wire)
            if ch==2:wire+=stream(1)
            end=len(wire);spans.append(dict(start=start,end=min(end,start+192),width=192))
            if ch==2:spans.append(dict(start=right_start,end=min(end,right_start+128),width=128))
            if coupled:
                start=len(wire)+3
                wire+='010'+field(1,4)+'1'+'000'+field(ch==2,1)+'0000'+('00' if ch==2 else '')+'0'+'0'+'10'
                wire+=channel(0,[1],[[1,0,-1,1]],info=ics(0,1,False))
                end=len(wire);spans.append(dict(start=start,end=min(end,start+192),width=192))
            payload=packed(wire+'111');size=len(payload)+9
            header=field(0xfff,12)+'0'+'00'+'0'+field(obj-1,2)+frequency(24000)+'0'+field(0 if explicit else ch,3)+'0000'+field(size,13)+field(0x7ff,11)+'00'
            protected=header+''.join(wire[s['start']:s['end']]+'0'*(s['width']-(s['end']-s['start'])) for s in spans)
            checksum=polynomial(protected)
            rows.append(dict(offset=len(blob),bytes=len(payload),samples=1024,regions=spans,header=packed(header).hex(),crc=checksum));blob.extend(payload)
            packets.append(payload);adts+=packed(header)+checksum.to_bytes(2,'big')+payload
        case=dict(name=name,explicit=explicit,channels=ch,asc=asc.hex(),frames=rows,slots=16,bands=32,container_rate=24000,container_frame_samples=1024,pcm_offset=0,samples=4096)
        case['video']=video_fixture([case],blob,channels=ch,filename='adts-crc-'+name+'-synthetic.mp4')
        file='adts-crc-'+name+'-synthetic.aac';(DEST/file).write_bytes(adts);case['adts']=file
        bad=bytearray(adts);bad[7]^=0x80;file='adts-crc-'+name+'-invalid-check-synthetic.aac';(DEST/file).write_bytes(bad);case['invalid']=file
        cases.append(case)
    (DEST/'adts-crc-packets.bin').write_bytes(blob)
    (DEST/'adts-crc.json').write_text(json.dumps(dict(cases=cases,oracle='GF(2) polynomial division, initial all ones, 0x18005, MSB first, no inversion'),indent=2)+'\n')
    print(len(cases),'authored ADTS CRC videos')
if __name__=='__main__':main()
