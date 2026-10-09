#!/usr/bin/env python3
"""Own multi-element/height/LFE ADTS SBR matrix with scalar PCM pointers."""
import json
from generate_adts_ps_fixtures import DEST,generate
if __name__=='__main__':
    source=json.loads((DEST/'he-aac-multi-sbr-oracles.json').read_text());cases=[]
    for i,c in enumerate(source['cases']):
        if c['slots']!=16 or (not c['video'] or '-implicit-' not in c['video']['file']) or c['bands']!=64 or c['layout']=='height-7.1-indexed':continue
        stem='adts-multi-sbr-'+str(i)
        row=generate(c,stem,'authored multi-element PCE/indexed SBR, height and silent LFE; independent per-lane scalar PCM and polynomial CRC',c['asc'],'he-aac-multi-sbr-packets.bin',6 if c['layout']=='5.1-indexed' else 0,channels=c['channels'])
        for key in ['layout','channels','mapping','pcm_offsets','widths','missing_element','missing_pcm_offset','channel_mask']:row[key]=c[key]
        cases.append(row)
    assert len(cases)==9
    (DEST/'adts-multi-sbr.json').write_text(json.dumps(dict(cases=cases),indent=2)+'\n')
