#!/usr/bin/env python3
"""Original Main/PS ADTS transports and companion videos; offline, no encoder."""
import json
from generate_aac_main_tools_fixtures import channel, ics, SEQUENCES
from generate_aac_ps_coupling_fixtures import fill
from generate_he_aac_packet_fixtures import DEST, field, frequency, packed, video_fixture

def main():
    base=json.loads((DEST/'aac-main-ps.json').read_text())
    blob=bytearray((DEST/'aac-main-ps-packets.bin').read_bytes())
    payloads=json.loads((DEST/'aac-ssr-ps.json').read_text())['payloads']
    cases=[]
    for late in (False, True):
        c=dict(base['cases'][1]); c['name']='late' if late else 'immediate'
        c['frames']=[dict(r) for r in c['frames']]
        if late:
            c['frames'][:4]=base['control']['frames'][:4]
            for i in range(4,len(c['frames'])):
                seq=SEQUENCES[i]; active=i>=3 and seq!=2; reset=1 if i==10 else None
                values=[([1,-1,1,-1] if (i+w)%2==0 else [-1,1,-1,1]) for w in range(8 if seq==2 else 1)]
                target='0000000'+channel(seq,[1],values,info=ics(seq,1,active,reset,i%2))
                raw=packed(target+fill(bytes.fromhex(payloads[(i-4)%3]))+'111')
                c['frames'][i]=dict(offset=len(blob),bytes=len(raw));blob.extend(raw)
        stream=bytearray()
        for row in c['frames']:
            raw=blob[row['offset']:row['offset']+row['bytes']]
            header=packed(field(0xfff,12)+'0'+'00'+'1'+'00'+frequency(24000)+'0'+'001'+'0000'+field(len(raw)+7,13)+field(0x7ff,11)+'00')
            stream.extend(header+raw)
        c['adts']=f'aac-main-ps-adts-{c["name"]}-synthetic.aac'
        (DEST/c['adts']).write_bytes(stream)
        c['video']=video_fixture([c],blob,channels=2,filename=f'aac-main-ps-adts-{c["name"]}-synthetic.mp4')
        cases.append(c)
    (DEST/'aac-main-ps-adts.json').write_text(json.dumps(dict(cases=cases,provenance='Own Main prediction residual/window packets and PS payloads from aac-main-ps; Main ADTS headers, immediate and late PS. No private media, FFmpeg or network.'),indent=2)+'\n')
if __name__=='__main__': main()
