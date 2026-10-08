#!/usr/bin/env python3
"""Owned three-spatial/three-temporal-layer SVC wrappers, no external tools."""
import hashlib,json
from generate_av1_spatial_container_samples import ROOT,pieces,webm,mp4
def main():
    data=(ROOT/'av1-spatiotemporal-operating-points.obu').read_bytes();units=[];seq=None
    for kind,sid,p in pieces(data):
        if kind==2:units.append([])
        assert units
        units[-1].append((kind,sid,p))
        if kind==1:seq=p
    assert len(units)==8
    records=[]
    for name,groups in [('complete',units),('missing-last-upper',units[:-1]+[[p for p in units[-1] if not(p[0]==6 and p[1]==2)] ]),('invalid-two-units',[units[0]+units[1]]),('invalid-repeated-layers',[units[0]+[p for p in units[1] if p[0]!=2]])]:
        packets=[b''.join(p for _,_,p in group) for group in groups]
        r={'name':name,'shown_indices':[2,5,8,11,14,17,20,23] if name=='complete' else [2,5,8,11,14,17,20,22] if name=='missing-last-upper' else [],'artifacts':[]}
        for ext,wrap in [('webm',webm),('mp4',mp4)]:
            output=wrap(packets,seq,(128,96));file=f'av1-spatiotemporal-container-{name}.{ext}';(ROOT/file).write_bytes(output)
            r['artifacts'].append({'file':file,'sha256':hashlib.sha256(output).hexdigest()})
        records.append(r)
    (ROOT/'av1-spatiotemporal-container-generated.json').write_text(json.dumps({'fixtures':records},indent=2)+'\n')
if __name__=='__main__':main()
