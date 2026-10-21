"""Owned hev1 bootstrap dispatch: active multilayer parameters arrive in-band.

Reuses only saved original synthetic coded fixtures, never private input or an
external codec. Generation is separate from ordinary offline Rust acceptance.
"""
import json,struct
from pathlib import Path
from generate_hevc_colour_plane_fixtures import child,packet_units,unescape,escape
from generate_he_aac_packet_fixtures import boxes
from generate_hevc_layered_reconstruction import configuration
from hevc_fixture_mp4 import mux
ROOT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
OUT=ROOT/'hevc-inband-layered-dispatch'
def inventory(path):
    data=path.read_bytes();stbl=data
    for tag in [b'moov',b'trak',b'mdia',b'minf',b'stbl']:stbl=child(stbl,tag)
    codec,entry=next(boxes(child(stbl,b'stsd')[8:]));assert codec in[b'hvc1',b'hev1']
    cfg=child(entry[78:],b'hvcC');sizes=child(stbl,b'stsz');count=int.from_bytes(sizes[8:12],'big');assert len(sizes)==12+4*count and sizes[4:8]==bytes(4)
    offsets=child(stbl,b'stco');assert int.from_bytes(offsets[4:8],'big')==1;at=int.from_bytes(offsets[8:12],'big');packets=[]
    for i in range(count):
        n=int.from_bytes(sizes[12+4*i:16+4*i],'big');packets.append([v[4:]for v in packet_units(data[at:at+n])]);at+=n
    return cfg,packets,list(struct.unpack('>HH',entry[24:28]))
def generate():
    OUT.mkdir(exist_ok=True);cases=[]
    manifest=json.loads((ROOT/'hevc-layered-reconstruction.json').read_text())['cases']
    wanted=['pcm-independent','pcm-dependent','multislice-dependent','colour-planes-independent','inter-prediction']
    for label in wanted:
        original=next(c for c in manifest if c['file']==f'hevc-layered-reconstruct-{label}.mp4')
        cfg,packets,size=inventory(ROOT/original['file']);basecfg,_,_=inventory(ROOT/original['seed']);params=configuration(cfg);base=configuration(basecfg)
        full=[n for k in [32,33,34]for n in params[k]];initial=[n for k in [32,33,34]for n in base[k]]
        for variant in ['base-inline','base-prefix','empty-inline','empty-prefix','multiple-vps']:
            prefix=initial;groups=[list(v)for v in packets];empty=variant.startswith('empty');split=variant.endswith('prefix')
            if variant=='multiple-vps':
                raw=unescape(base[32][0][2:]);unused=base[32][0][:2]+escape(bytes([(raw[0]&15)|16])+raw[1:]);prefix=[unused,*full]
            elif split:groups=[full,*groups]
            else:groups[0]=full+groups[0]
            stream=b''.join(b'\0\0\0\1'+v for v in prefix+[n for group in groups for n in group])
            movie=mux(stream,8+cfg[17]%8,*size,chroma_format=cfg[16]%4,chroma_depth=8+cfg[18]%8,access_units=groups,multilayer=True,inband_parameters=True,empty_configuration=empty)
            name=f'{label}-{variant}';(OUT/f'{name}.mp4').write_bytes(movie)
            cases.append(dict(name=name,source=original['file'],oracle=str(Path(original['seed']).with_suffix('.yuv')),frames=len(packets),first_sample=int(split),variant=variant))
    assert len(cases)==25
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n')
if __name__=='__main__':generate()
