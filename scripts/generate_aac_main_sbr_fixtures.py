#!/usr/bin/env python3
"""Authored active Main predictor plus SBR, offline owned transport helper only."""
import json, subprocess
from generate_aac_main_prediction_fixtures import CODES,LENS,SC,SL,field,frequency,packed
from generate_aac_ssr_fixtures import DEST
from generate_he_aac_packet_fixtures import video_fixture
from generate_adts_multiblock_fixtures import transport
from generate_adts_ps_fixtures import region_helper

def main():
    syntax=(DEST/'aac-sbr-dsp-syntax.bin').read_bytes()
    ref=next(c for c in json.loads((DEST/'aac-sbr-dsp-oracles.json').read_text())['cases'] if c['slots']==16 and c['bands']==64 and c['limiter']==0 and not c['smoothing'])
    blob=bytearray();rows=[];implicit=packed(field(1,5)+frequency(24000)+'0001'+'000')
    for frame in range(6):
        active=frame>=3
        info='0000'+field(1,6)+('101' if active else '0')
        indices=[1,-1,1,-1] if frame%2 else [-1,1,-1,1];index=0
        for v in indices:index=index*3+v+1
        core='0000000'+field(140,8)+info+field(1,4)+field(1,5)+field(SC[60],SL[60])+'000'+field(CODES[index],LENS[index])
        r=ref['frames'][frame%3];raw=syntax[r['offset']:r['offset']+r['byte_length']]
        count=len(raw);fill='110'+(field(count,4) if count<15 else '1111'+field(count-14,8))
        p=packed(core+fill+''.join(field(b,8) for b in raw)+'111')
        size=len(p)+9
        h=packed(field(0xfff,12)+'0'+'00'+'0'+'00'+frequency(24000)+'0'+'001'+'0000'+field(size,13)+field(0x7ff,11)+'00')
        rows.append(dict(offset=len(blob),bytes=len(p),header=h.hex()));blob.extend(p)
    inputs=[dict(asc=implicit.hex(),payload=blob[r['offset']:r['offset']+r['bytes']].hex()) for r in rows]
    spans=json.loads(subprocess.run([str(region_helper())],input=json.dumps(inputs),capture_output=True,text=True,check=True).stdout)
    for r,s in zip(rows,spans):r['regions']=s
    cases=[]
    for signal in ('explicit','sync','implicit'):
        asc=(packed(field(5,5)+frequency(24000)+'0001'+frequency(48000)+field(1,5)+'000') if signal=='explicit' else
             packed(field(1,5)+frequency(24000)+'0001'+'000'+(field(0x2b7,11)+field(5,5)+'1'+frequency(48000) if signal=='sync' else '')))
        c=dict(asc=asc.hex(),frames=rows,slots=16,bands=64,container_rate=48000,container_frame_samples=2048,pcm_offset=0,samples=12288)
        c['video']=video_fixture([c],blob,filename='aac-main-sbr-'+signal+'-synthetic.mp4');cases.append(c)
    files=[]
    for group in ([1]*6,[3,3],[1,2,3]):
        for crc in (False,True):
            at=0;data=bytearray()
            for n in group:data+=transport(rows[at:at+n],blob,crc);at+=n
            name='aac-main-sbr-'+''.join(map(str,group))+('-crc' if crc else '')+'-synthetic.aac'
            (DEST/name).write_bytes(data);files.append(name)
    (DEST/'aac-main-sbr-packets.bin').write_bytes(blob)
    (DEST/'aac-main-sbr.json').write_text(json.dumps(dict(cases=cases,files=files,provenance='Own nonzero integer spectra with prediction enabled after frame three, authored SBR syntax, independent polynomial CRC; equivalent explicit/sync/implicit signalling. No private media or foreign codec.'),indent=2)+'\n')
if __name__=='__main__':main()
