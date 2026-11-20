#!/usr/bin/env python3
"""Authored in-tail ancillary replacement movies and inherited-fragment refusals."""
import json
from pathlib import Path
from generate_aac_ld_mps_inband_fixtures import raw,header,gain_changed
from generate_aac_ld_mps_transport_fixtures import envelope,packed
from generate_he_aac_packet_fixtures import video_fixture
from generate_aac_eld_fixtures import mkv
from generate_aac_ld_mps_config_fixtures import TREES
ROOT=Path(__file__).resolve().parents[1];FIX=ROOT/'tests/fixtures/playback-errors';OUT=FIX/'aac-ld-mps-local-replacement'
def main():
    parent=FIX/'aac-ld-mps-inband';m=json.loads((parent/'manifest.json').read_text());source=(parent/'packets.bin').read_bytes()
    OUT.mkdir(exist_ok=True);blob=bytearray();cases=[];errors=[]
    for base in m['cases']:
        if base['mode'] not in ['same','fragmented-gain']:continue
        for empty in ([False,True]if base['mode']=='same'else[False]):
            prefix=envelope(9,0,2,b''if empty else b'\xa5\x37',True)
            rows=[];packets=[]
            for i,r in enumerate(base['frames']):
                old=raw(source[r['offset']:r['offset']+r['bytes']]);end=r['prefix_bits'];core,tail=old[:end],old[end:]
                if base['mode']=='same' or i==2:tail=prefix+tail
                elif i==1:
                    payload=header(bytes.fromhex(base['configs'][0]),bytes.fromhex(base['body']))
                    stop=envelope(9,1,1,payload[len(payload)//2:],True)
                    assert tail.startswith(stop);tail=stop+prefix+tail[len(stop):]
                data=packed(core+tail);rows.append(dict(offset=len(blob),bytes=len(data),prefix_bits=end));blob+=data;packets.append(data)
            name=base['name']+('-local-empty'if empty else'-local')
            c=dict(base,name=name,frames=rows,control=base['name']+'.mp4',empty=empty,
                output_channels=TREES[base['tree']][1],inherited_completes_first=base['mode']=='fragmented-gain')
            (OUT/(name+'.mp4')).write_bytes(video_fixture([c],blob,base['channels'],write=False)['data'])
            samples=base['slots']*base['bands']
            (OUT/(name+'.mkv')).write_bytes(mkv(bytes.fromhex(c['asc']),packets,c['rate'],samples,c['output_channels']))
            cases.append(c)
        # An inherited unfinished fragment is intentionally still refused.
        # The valid fragmented control is retained for a same-packet retry.
        if base['mode']=='fragmented-gain' and base['tree']==7:
            rows=[]
            for i,r in enumerate(base['frames']):
                data=source[r['offset']:r['offset']+r['bytes']]
                if i==1:
                    old=raw(data);payload=header(bytes.fromhex(base['configs'][0]),bytes.fromhex(base['body']))
                    stop=envelope(9,1,1,payload[len(payload)//2:],True);tail=old[r['prefix_bits']:];assert tail.startswith(stop)
                    replacement=header(gain_changed(bytes.fromhex(base['configs'][0]),base['tree']),bytes.fromhex(base['body']))
                    assert not replacement.startswith(payload[:len(payload)//2])
                    data=packed(old[:r['prefix_bits']]+envelope(9,1,3,replacement,True)+envelope(9,0,2,b'\xa5',True)+tail[len(stop):])
                rows.append(dict(offset=len(blob),bytes=len(data),prefix_bits=r['prefix_bits']));blob+=data
            name=base['name']+'-inherited-refusal';c=dict(base,name=name,frames=rows,control=base['name']+'.mp4',output_channels=TREES[base['tree']][1],local_replacement_after_inherited=True)
            (OUT/(name+'.mp4')).write_bytes(video_fixture([c],blob,base['channels'],write=False)['data']);errors.append(c)
    (OUT/'packets.bin').write_bytes(blob)
    (OUT/'manifest.json').write_text(json.dumps(dict(cases=cases,errors=errors,scope='Authored AAC core/LD-SBR prefixes and spatial messages from the existing own in-band corpus. Local incomplete fragments replaced in one extension tail, including after completing an inherited fragment. Inherited replacement remains a distinct refusal, not acceptance. No private media, external decoder, FFmpeg, process or network.'),indent=2)+'\n')
    print('Local replacement:',len(cases),'acceptance and',len(errors),'inherited-refusal programmes')
if __name__=='__main__':main()
