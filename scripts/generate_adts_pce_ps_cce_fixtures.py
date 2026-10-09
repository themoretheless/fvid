#!/usr/bin/env python3
"""Own mono PCE + PS + CCE ADTS transport matrix, offline."""
import json
from generate_adts_ps_fixtures import DEST,generate
if __name__=='__main__':
    source=json.loads((DEST/'he-aac-ps-coupling-oracles.json').read_text())
    rows=[]
    for case in source['cases']:
        if case['slots']!=16 or case['kind']!='LC' or case['output_rate']!=48000:continue
        stem='adts-pce-ps-cce-'+str(case['point'])+'-'+''.join(map(str,case['tags']))
        rows.append(generate(case,stem,'authored PCE/PS/CCE; previously qualified direct-cosine core and PS/SBR composition; independent polynomial CRC',case['asc'],'he-aac-ps-coupling-packets.bin',0,[[1]*6,[3,3],[1,2,3]]))
    assert len(rows)==9
    (DEST/'adts-pce-ps-cce.json').write_text(json.dumps(dict(cases=rows),indent=2)+'\n')
