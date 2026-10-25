#!/usr/bin/env python3
"""Repeated complete target IDR AUs from owned PCM entropy; no codec process."""
import json
from pathlib import Path
from generate_hevc_inband_layered_dispatch import inventory
from generate_hevc_layered_reconstruction import configuration
from generate_hevc_colour_plane_fixtures import escape, unescape, child
from generate_he_aac_packet_fixtures import boxes
from generate_ffv1_gray_fixtures import element, uint
from hevc_fixture_mp4 import mux
ROOT = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors'
OUT = ROOT / 'hevc-idr-output-epochs'

def generate():
    files, cases = {}, []
    sources = json.loads((ROOT / 'hevc-layered-reconstruction.json').read_text())['cases']
    for label in ['pcm', 'multislice', 'colour-planes']:
        for independent in [False, True]:
            role = 'independent' if independent else 'dependent'
            source = f'hevc-layered-reconstruct-{label}-{role}.mp4'
            cfg, original, size = inventory(ROOT / source)
            params = configuration(cfg)
            for discard in [False, True]:
                groups = []
                for _ in range(3):
                    group = []
                    for nal in original[0]:
                        assert 16 <= (nal[0] >> 1) & 63 <= 23
                        raw = bytearray(unescape(nal[2:]))
                        raw[0] = (raw[0] & ~64) | (64 if discard else 0)
                        group.append(nal[:2] + escape(bytes(raw)))
                    groups.append(group)
                parameters = [n for kind in [32,33,34] for n in params[kind]]
                stream = b''.join(b'\0\0\0\1' + n for n in parameters + sum(groups, []))
                movie = mux(stream, 8 + cfg[17] % 8, *size,
                            chroma_format=cfg[16] % 4, chroma_depth=8 + cfg[18] % 8,
                            access_units=groups, multilayer=True)
                name = f'{label}-{role}-' + ('discard' if discard else 'flush')
                files[name + '.mp4'] = movie
                stbl = movie
                for kind in [b'moov',b'trak',b'mdia',b'minf',b'stbl']:
                    stbl = child(stbl,kind)
                _, entry = next(boxes(child(stbl,b'stsd')[8:]))
                private = child(entry[78:], b'hvcC')
                packets = [b''.join(len(n).to_bytes(4,'big') + n for n in g) for g in groups]
                track = (uint(0xd7,1) + uint(0x83,1) + element(0x86,b'V_MPEGH/ISO/HEVC')
                         + element(0x63a2,private) + uint(0x23e383,40000000)
                         + element(0xe0,uint(0xb0,size[0]) + uint(0xba,size[1])))
                clusters = b''.join(element(0x1f43b675,uint(0xe7,i*40)
                           + element(0xa3,b'\x81\0\0\x80' + p)) for i,p in enumerate(packets))
                files[name + '.mkv'] = (element(0x1a45dfa3,element(0x4282,b'matroska'))
                    + element(0x18538067,element(0x1549a966,uint(0x2ad7b1,1000000))
                              + element(0x1654ae6b,element(0xae,track)) + clusters))
                seed = next(c['seed'] for c in sources if c['file'] == source)
                cases.append(dict(name=name, source=source, oracle=str(Path(seed).with_suffix('.yuv')),
                                  independent=independent, discard=discard))
                if label == 'pcm' and not independent and discard:
                    files['identity.hvcc'] = cfg
                    files['identity-idr.au'] = packets[0]
                    inter_cfg, inter_groups, _ = inventory(ROOT / 'hevc-layered-reconstruct-inter-prediction.mp4')
                    assert inter_cfg == cfg
                    files['identity-inter.au'] = b''.join(len(n).to_bytes(4, 'big') + n for n in inter_groups[1])
                if label == 'pcm' and not independent and not discard:
                    # Keep the second AU's slice header intact and shorten only
                    # the enhancement PCM entropy. Packet/NAL framing is valid.
                    bad = [list(g) for g in groups]
                    bad[1][-1] = bad[1][-1][:-128]
                    authored = b''.join(b'\0\0\0\1' + n for n in parameters + sum(bad, []))
                    files['pcm-dependent-flush-truncated.mp4'] = mux(
                        authored, 8 + cfg[17] % 8, *size, chroma_format=cfg[16] % 4,
                        chroma_depth=8 + cfg[18] % 8, access_units=bad, multilayer=True)
    files['manifest.json'] = (json.dumps(dict(cases=cases,refusal='pcm-dependent-flush-truncated.mp4',provenance=
        'Three complete target IDR AUs, unchanged owned VPS and PCM entropy; only '
        'prior-output flags are authored. Own MP4/Matroska muxers; no private media, '
        'FFmpeg, external codec process or network.'),indent=2)+'\n').encode()
    return files

if __name__ == '__main__':
    OUT.mkdir(exist_ok=True)
    for name, data in generate().items():
        (OUT / name).write_bytes(data)
