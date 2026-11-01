#!/usr/bin/env python3
"""Own full-message retransmissions that preserve an inherited fragment exactly."""
import json
from pathlib import Path
from generate_aac_ld_mps_inband_fixtures import raw,header
from generate_aac_ld_mps_transport_fixtures import envelope,packed
from generate_he_aac_packet_fixtures import video_fixture
from generate_aac_eld_fixtures import mkv
from generate_aac_ld_mps_config_fixtures import TREES
ROOT=Path(__file__).resolve().parents[1];FIX=ROOT/'tests/fixtures/playback-errors';OUT=FIX/'aac-ld-mps-retransmission'
def main():
    parent=FIX/'aac-ld-mps-inband';m=json.loads((parent/'manifest.json').read_text());source=(parent/'packets.bin').read_bytes()
    OUT.mkdir(exist_ok=True);blob=bytearray();cases=[]
    for base in m['cases']:
        if base['mode']!='fragmented-gain':continue
        for extra in ([False,True]if base['tree']==7 else[False]):
            payload=header(bytes.fromhex(base['configs'][0]),bytes.fromhex(base['body']))
            half=len(payload)//2;assert half>0
            rows=[];packets=[]
            for i,r in enumerate(base['frames']):
                data=source[r['offset']:r['offset']+r['bytes']]
                if i==1:
                    old=raw(data);stop=envelope(9,1,1,payload[half:],True);tail=old[r['prefix_bits']:];assert tail.startswith(stop)
                    data=packed(old[:r['prefix_bits']]+envelope(9,1,3,payload,True)+(envelope(9,0,2,b'\xa5',True)if extra else'')+tail[len(stop):])
                rows.append(dict(offset=len(blob),bytes=len(data),prefix_bits=r['prefix_bits']));blob+=data;packets.append(data)
            name=base['name']+'-retransmit'+('-then-local'if extra else'');c=dict(base,name=name,frames=rows,control=base['name']+'.mp4',output_channels=TREES[base['tree']][1],inherited_prefix=payload[:half].hex(),replacement=payload.hex())
            (OUT/(name+'.mp4')).write_bytes(video_fixture([c],blob,base['channels'],write=False)['data'])
            (OUT/(name+'.mkv')).write_bytes(mkv(bytes.fromhex(c['asc']),packets,c['rate'],c['slots']*c['bands'],c['output_channels']))
            cases.append(c)
    (OUT/'packets.bin').write_bytes(blob)
    (OUT/'manifest.json').write_text(json.dumps(dict(cases=cases,scope='Forty-five authored three-AU programmes. Whole retransmission preserves every byte of the inherited unfinished fragment; assembled message equals normal stop continuation. No claim for arbitrary prefix loss or concealment. Own AAC/LD-SBR and spatial values, no private/ISO media, external decoder, FFmpeg, process or network.'),indent=2)+'\n')
    print('Generated',len(cases),'prefix-preserving retransmission programmes')
if __name__=='__main__':main()
