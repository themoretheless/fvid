"""Owned AVC PCM and PPS scaling syntax; no external tools or private input."""
import json
from pathlib import Path
from avc_fixture_mp4 import mux
from generate_avc_colour_formats import avc_configuration, make_pcm_packets
from generate_avc_mbaff_direct_samples import Writer

OUT = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors/avc-parameter-prefix-order'

def parameters(config):
    size = int.from_bytes(config[6:8], 'big')
    sps = config[8:8 + size]
    at = 9 + size
    count = int.from_bytes(config[at:at + 2], 'big')
    return [sps, config[at + 2:at + 2 + count]]

def scaling_pps():
    b = Writer()
    b.ue(0); b.ue(0); b.u(0); b.u(0); b.ue(0)
    b.ue(0); b.ue(0); b.u(0); b.u(0, 2)
    b.se(0); b.se(0); b.se(0); b.u(1); b.u(0); b.u(0)
    b.u(1)  # transform_8x8_mode_flag
    b.u(1)  # pic_scaling_matrix_present_flag
    for _ in range(12):
        b.u(0)  # 4:4:4 has six 8x8 lists, unlike the old monochrome SPS.
    b.se(0)
    return b.nal(0x68)

def oracle(planes, yd, cd):
    data = bytearray()
    for picture in planes:
        for p, depth in zip(picture, [yd, cd, cd]):
            for sample in p:
                data.extend(bytes([sample]) if depth == 8 else sample.to_bytes(2, 'little'))
    return bytes(data)

def generate():
    files, cases = {}, []
    for yd, cd in [(8, 8), (10, 12), (14, 8)]:
        stem = f'y{yd}-c{cd}'
        old_cfg = avc_configuration(0, yd, cd, False)
        new_cfg = avc_configuration(3, yd, cd, False)
        new_sps, _ = parameters(new_cfg)
        new_pps = scaling_pps()
        new_cfg = new_cfg[:6] + len(new_sps).to_bytes(2, 'big') + new_sps + b'\x01' + len(new_pps).to_bytes(2, 'big') + new_pps
        old_packets, old_planes = make_pcm_packets(0, yd, cd)
        new_packets, new_planes = make_pcm_packets(3, yd, cd)
        for label, data in [('old.avcc', old_cfg), ('new.avcc', new_cfg),
                            ('old.yuv', oracle(old_planes, yd, cd)), ('new.yuv', oracle(new_planes, yd, cd))]:
            files[f'{stem}-{label}'] = data
        for reverse in [False, True]:
            units = [new_pps, new_sps] if reverse else [new_sps, new_pps]
            prefix = b''.join(len(n).to_bytes(4, 'big') + n for n in units)
            for separate in [False, True]:
                packets = list(old_packets + new_packets)
                if separate:
                    packets.insert(2, prefix)
                else:
                    packets[2] = prefix + packets[2]
                frames = [(i, i in [0, 2 + int(separate)], p) for i, p in enumerate(packets)]
                name = stem + ('-pps-sps' if reverse else '-sps-pps') + ('-prefix' if separate else '-inline')
                files[f'{name}.mp4'] = mux(old_cfg, frames, 32, 16, 25, inband_parameters=True)
                cases.append(dict(name=name, stem=stem, ydepth=yd, cdepth=cd, reverse=reverse, separate=separate))
    # Both IDs fit; contextual syntax is absent and must still be rejected.
    files['truncated-pps.packet'] = b'\x00\x00\x00\x02\x68\xc0'
    files['manifest.json'] = (json.dumps(cases, indent=2) + '\n').encode()
    return files

if __name__ == '__main__':
    OUT.mkdir(exist_ok=True)
    for name, data in generate().items():
        (OUT / name).write_bytes(data)
