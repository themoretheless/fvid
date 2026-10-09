#!/usr/bin/env python3
"""Authored SSR window transitions plus SBR; no external codec or private media."""
import json
from generate_he_aac_packet_fixtures import DEST,field,frequency,packed,video_fixture

def main():
    binary=(DEST/'aac-sbr-dsp-syntax.bin').read_bytes()
    ref=next(c for c in json.loads((DEST/'aac-sbr-dsp-oracles.json').read_text())['cases'] if c['slots']==16 and c['bands']==64 and c['limiter']==0 and not c['smoothing'])
    blob=bytearray();rows=[];controls=[]
    for i,seq in enumerate([0,1,2,2,3,0]):
        # SCE0, zero bands, no pulse/TNS/gain. SSR accepts the same ICS shape.
        core='0000000'+field(100,8)+'0'+field(seq,2)+'0'+field(0,4 if seq==2 else 6)+('1111111' if seq==2 else '0')+'000'
        p=packed(core+'111');controls.append(dict(offset=len(blob),bytes=len(p)));blob.extend(p)
        r=ref['frames'][i%3];raw=binary[r['offset']:r['offset']+r['byte_length']];n=len(raw)
        p=packed(core+'110'+(field(n,4) if n<15 else '1111'+field(n-14,8))+''.join(field(b,8) for b in raw)+'111')
        rows.append(dict(offset=len(blob),bytes=len(p)));blob.extend(p)
    cases=[]
    for signal in ('explicit','sync','implicit','core-control'):
        prefix=field(3,5)+frequency(24000)+'0001'+'000'
        config=(field(5,5)+frequency(24000)+'0001'+frequency(48000)+field(3,5)+'000' if signal=='explicit' else prefix+(field(0x2b7,11)+field(5,5)+'1'+frequency(48000) if signal=='sync' else ''))
        c=dict(name=signal,asc=packed(config).hex(),frames=controls if signal=='core-control' else rows,slots=16,bands=64,container_rate=24000 if signal=='core-control' else 48000,container_frame_samples=1024 if signal=='core-control' else 2048,pcm_offset=0,samples=6144 if signal=='core-control' else 12288)
        c['video']=video_fixture([c],blob,filename='aac-ssr-sbr-'+signal+'-synthetic.mp4');cases.append(c)
    (DEST/'aac-ssr-sbr-packets.bin').write_bytes(blob)
    (DEST/'aac-ssr-sbr.json').write_text(json.dumps(dict(cases=cases,provenance='Own silent SSR SCE across long/start/short/stop transitions; existing authored SBR syntax and owned AVC/container seeds. No private media, external codec or network.'),indent=2)+'\n')
if __name__=='__main__':main()
