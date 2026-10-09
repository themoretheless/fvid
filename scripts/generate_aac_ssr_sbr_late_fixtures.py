#!/usr/bin/env python3
"""Owned silent/nonzero SSR prefixes followed by SBR; no foreign codec."""
import json, struct
from generate_aac_ssr_fixtures import channel
from generate_aac_main_sbr_oracle import reference
from generate_aac_ssr_coupling_fixtures import program, silent
from generate_he_aac_packet_fixtures import DEST, field, frequency, packed, video_fixture

def main():
    raw=(DEST/'aac-sbr-dsp-syntax.bin').read_bytes()
    ref=next(c for c in json.loads((DEST/'aac-sbr-dsp-oracles.json').read_text())['cases']
             if c['slots']==16 and c['bands']==64 and c['limiter']==0 and not c['smoothing'])
    blob=bytearray();cases=[]
    for late, active, coupled in [(False,False,True),(True,False,True),(False,True,True),(True,True,True),(False,True,False),(True,True,False)]:
        frames=[]
        for i,seq in enumerate([0,1,2,2,3,0]):
            target='0000000'+(silent(seq,0,0,False,False) if coupled else channel(i,seq,0,0,True,False))
            source='0100001'+'1'+'000'+'0'+'0000'+'0'+'0'+'10'+(channel(i,seq,0,0,True,False) if active else silent(seq,0,0,False,False))
            if not coupled: source=''
            fill=''
            if late and i>=3:
                row=ref['frames'][(i-3)%3];payload=raw[row['offset']:row['offset']+row['byte_length']];n=len(payload)
                fill='110'+(field(n,4) if n<15 else '1111'+field(n-14,8))+''.join(field(b,8) for b in payload)
            packet=packed(target+source+fill+'111')
            frames.append(dict(offset=len(blob),bytes=len(packet)));blob.extend(packet)
        asc=packed(program(field(3,5)+frequency(24000)+'0000'+'000',1,3,(1,) if coupled else ())).hex()
        case=dict(name=('late' if late else 'core-control')+('-active' if active else '')+('-target' if not coupled else ''),asc=asc,frames=frames,container_rate=24000,container_frame_samples=1024,samples=6144,slots=16,bands=32,pcm_offset=0)
        case['video']=video_fixture([case],blob,filename='aac-ssr-sbr-late-'+case['name']+'-synthetic.mp4');cases.append(case)
    core_bytes=(DEST/'aac-ssr-sbr-active-core-reference.f32le').read_bytes()
    core=[x[0] for x in struct.iter_unpack('<f',core_bytes)]
    warm=reference(pcm_override=core,bands=32,first_sbr_frame=3)
    expected=core[:3072]+warm[3072:]
    (DEST/'aac-ssr-sbr-late-active-reference.f64le').write_bytes(struct.pack('<6144d',*expected))
    (DEST/'aac-ssr-sbr-late-packets.bin').write_bytes(blob)
    (DEST/'aac-ssr-sbr-late.json').write_text(json.dumps(dict(cases=cases,first_sbr_packet=3,
        provenance='Own silent/nonzero SSR SCE0 and optional CCE1 with independent unit gain; valid SBR FIL starts at packet 3. Active scalar reference includes full 32-band pre-FIL analysis and synthesis history, then fixed authored HF/noise. No private/foreign media.'),indent=2)+'\n')
if __name__=='__main__':main()
