"""Owned base hev1 parameter bootstrap, preserving original synthetic entropy."""
import json
from pathlib import Path
from generate_hevc_inband_layered_dispatch import inventory,configuration,ROOT
from hevc_fixture_mp4 import mux
OUT=ROOT/'hevc-inband-base-bootstrap'
SEEDS=['hevc-pcm-reference-rext8','hevc-pcm-dependent-rext8','hevc-separate-colour-planes-distinct-synthetic','hevc-mixed-depth-y12-c10-wpp','hevc-mixed-depth-y10-c8-mixed-tiles','hevc-scc-act-slice-y10-c10']
def generate():
    OUT.mkdir(exist_ok=True);cases=[]
    layouts={'empty-inline':([],False),'empty-prefix':([],True),'empty-split':([],True),'vps-recorded':([32],False),'sps-recorded':([33],True),'sps-pps-recorded':([33,34],False),'pps-recorded':([34],False),'empty-reverse':([],True)}
    for stem in SEEDS:
        cfg,packets,size=inventory(ROOT/f'{stem}.mp4');params=configuration(cfg);full=[n for k in [32,33,34]for n in params[k]]
        for layout,(recorded,split)in layouts.items():
            pending=[n for k in [32,33,34]if k not in recorded for n in params[k]];groups=[list(p)for p in packets]
            if layout in ['empty-split','empty-reverse']:
                prefix=[[n for n in params[k]]for k in ([34,33,32]if layout=='empty-reverse'else[32,33,34])];groups=prefix+groups;first=3
            elif split:groups=[pending]+groups;first=1
            else:groups[0]=pending+groups[0];first=0
            units=full+[n for group in groups for n in group]
            stream=b''.join(b'\0\0\0\1'+n for n in units)
            movie=mux(stream,8+cfg[17]%8,*size,chroma_format=cfg[16]%4,chroma_depth=8+cfg[18]%8,access_units=groups,inband_parameters=True,configuration_types=recorded)
            name=f'{stem}-{layout}';(OUT/f'{name}.mp4').write_bytes(movie)
            cases.append(dict(name=name,seed=stem,layout=layout,configuration_types=recorded,frames=len(packets),first_sample=first))
    assert len(cases)==48
    # Repeated unresolved PPS reception must charge the caller's aggregate budget.
    cfg,packets,size=inventory(ROOT/'hevc-pcm-reference-rext8.mp4');params=configuration(cfg)
    groups=[]
    for ident in range(32):
        # Only IDs are interpreted before the unavailable SPS 15 arrives.
        from generate_hevc_poc_reset_fixtures import ue
        from generate_hevc_colour_plane_fixtures import packed,escape
        body=packed(ue(ident)+ue(15)+'1')+b'\xff'*4096+b'\x80'
        groups.append([b'\x44\x01'+escape(body)])
    full=[n for k in [32,33,34]for n in params[k]];groups.append(packets[0])
    (OUT/'recovery.packet').write_bytes(b''.join(len(n).to_bytes(4,'big')+n for n in full))
    stream=b''.join(b'\0\0\0\1'+n for n in full+[n for group in groups for n in group])
    (OUT/'unresolved-pps-budget.mp4').write_bytes(mux(stream,8,*size,access_units=groups,inband_parameters=True,configuration_types=[]))
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n')
if __name__=='__main__':generate()
