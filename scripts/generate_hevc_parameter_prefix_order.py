"""Owned parameter replacement ordering; unchanged original coded pictures."""
import json
from pathlib import Path
from generate_hevc_inband_layered_dispatch import inventory, configuration, ROOT
from hevc_fixture_mp4 import mux

OUT = ROOT / 'hevc-parameter-prefix-order'
OLD = 'hevc-pcm-reference-rext8'
NEW = 'hevc-scc-act-slice-y10-c10'

def generate():
    old_cfg, old_packets, size = inventory(ROOT / f'{OLD}.mp4')
    new_cfg, new_packets, new_size = inventory(ROOT / f'{NEW}.mp4')
    assert size == new_size
    old = configuration(old_cfg)
    new = configuration(new_cfg)
    initial = [n for k in (32, 33, 34) for n in old[k]]
    files, cases = {}, []
    for order in [(32, 33, 34), (32, 34, 33), (34, 32, 33)]:
        prefix = [n for k in order for n in new[k]]
        for separate in [False, True]:
            groups = [list(p) for p in old_packets + new_packets]
            if separate:
                groups.insert(len(old_packets), prefix)
            else:
                groups[len(old_packets)] = prefix + groups[len(old_packets)]
            stream = b''.join(b'\0\0\0\1' + n for n in initial + [n for p in groups for n in p])
            name = '-'.join(str(k) for k in order) + ('-prefix' if separate else '-inline')
            files[f'{name}.mp4'] = mux(
                stream, 8 + new_cfg[17] % 8, *size,
                chroma_format=new_cfg[16] % 4, chroma_depth=8 + new_cfg[18] % 8,
                access_units=groups, inband_parameters=True,
            )
            cases.append(dict(name=name, order=order, separate=separate,
                              old_frames=len(old_packets), new_frames=len(new_packets)))
    files['manifest.json'] = (json.dumps(cases, indent=2) + '\n').encode()
    return files

if __name__ == '__main__':
    OUT.mkdir(exist_ok=True)
    for name, data in generate().items():
        (OUT / name).write_bytes(data)
