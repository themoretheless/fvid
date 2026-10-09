#!/usr/bin/env python3
"""Authored tagged mono PCE and late PS in ADTS, offline."""
import json
from generate_adts_ps_fixtures import DEST, generate
from generate_aac_ps_pce_fixtures import packet,config
if __name__=='__main__':
    manifest=json.loads((DEST/'aac-ps-absence-oracles.json').read_text())
    source=dict(next(c for c in manifest['cases'] if c['slots']==16 and c['name']=='late-ps'))
    raw=(DEST/'he-aac-ps-absence-packets.bin').read_bytes();blob=bytearray();frames=[]
    for row in source['frames']:
        payload=packet(raw[row['offset']:row['offset']+row['bytes']])
        frames.append(dict(offset=len(blob),bytes=len(payload)));blob.extend(payload)
    (DEST/'adts-pce-ps-packets.bin').write_bytes(blob);source['frames']=frames
    generate(source,'adts-pce-ps','authored normal-front mono PCE, tagged SCE and late PS; independent scalar stereo PCM and polynomial CRC',config(16,'LC').hex(),'adts-pce-ps-packets.bin',0)
