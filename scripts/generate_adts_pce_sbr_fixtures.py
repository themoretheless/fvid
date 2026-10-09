#!/usr/bin/env python3
"""Authored sole tagged stereo PCE SBR in ADTS, offline."""
import json
from generate_adts_ps_fixtures import DEST,generate
if __name__=='__main__':
    source=json.loads((DEST/'he-aac-pce-sbr-oracles.json').read_text())
    cases=[]
    for c in source['cases']:
        if c['slots']!=16 or c['channels']!=2 or c['kind']!='implicit':continue
        coupled='pair-True-' in c['video']['file']
        stem='adts-pce-sbr-'+('coupled' if coupled else 'uncoupled')
        row=generate(c,stem,'authored tagged sole CPE PCE and SBR; independent scalar PCM and polynomial CRC',c['asc'],'he-aac-pce-sbr-packets.bin',0)
        row['pcm_offset']=c['pcm_offset'];cases.append(row)
    assert len(cases)==2
    (DEST/'adts-pce-sbr.json').write_text(json.dumps(dict(cases=cases),indent=2)+'\n')
