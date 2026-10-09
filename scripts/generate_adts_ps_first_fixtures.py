#!/usr/bin/env python3
"""Own first-packet PS / missing-middle ADTS remux regressions, offline."""
import json
from generate_adts_ps_fixtures import DEST, generate
if __name__=='__main__':
    manifest=json.loads((DEST/'aac-ps-absence-oracles.json').read_text())
    source=next(c for c in manifest['cases'] if c['slots']==16 and c['name']=='missing-middle')
    generate(source, 'adts-first-ps', 'existing authored first-packet PS, missing-middle and independent scalar stereo PCM; own CRC regions and independent polynomial division')
