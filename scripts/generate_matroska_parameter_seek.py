"""Owned AVC/HEVC parameter-history seek wrappers and independent packed pixels."""
import json
from pathlib import Path
from generate_ffv1_gray_fixtures import element, uint
from generate_hevc_matroska_formats import boxes, child
from generate_avc_colour_formats import make_pcm_packets, pack_planar_oracle
import generate_avc_parameter_prefix_order as avc
import generate_hevc_parameter_prefix_order as hevc

ROOT = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors'
OUT = ROOT / 'matroska-parameter-seek'

def inventory(data):
    stbl = data
    for tag in [b'moov', b'trak', b'mdia', b'minf', b'stbl']:
        stbl = child(stbl, tag)
    codec, entry = next(boxes(child(stbl, b'stsd')[8:]))
    private = child(entry[78:], b'avcC' if codec == b'avc3' else b'hvcC')
    size = [int.from_bytes(entry[i:i+2], 'big') for i in [24, 26]]
    sizes = child(stbl, b'stsz'); count = int.from_bytes(sizes[8:12], 'big')
    assert sizes[4:8] == bytes(4)
    at = int.from_bytes(child(stbl, b'stco')[8:12], 'big')
    sync = child(stbl, b'stss')
    keys = {int.from_bytes(sync[i:i+4], 'big') - 1 for i in range(8, len(sync), 4)}
    packets = []
    for i in range(count):
        n = int.from_bytes(sizes[12+i*4:16+i*4], 'big')
        packets.append(data[at:at+n]); at += n
    return private, packets, size, keys

def movie(codec, private, packets, size, keys, companion):
    video = uint(0xB0, size[0]) + uint(0xBA, size[1])
    track = uint(0xD7, 1) + uint(0x83, 1) + element(0x86, codec.encode())
    track += element(0x63A2, private) + uint(0x23E383, 40_000_000) + element(0xE0, video)
    tracks = element(0xAE, track)
    if companion:
        tracks += element(0xAE, uint(0xD7, 2) + uint(0x83, 17) + element(0x86, b'S_TEXT/UTF8'))
    clusters = []
    for i, packet in enumerate(packets):
        blocks = b''
        if companion:
            blocks += element(0xA3, b'\x82\0\0\x80not a video NAL packet')
        blocks += element(0xA3, b'\x81\0\0' + (b'\x80' if i in keys else b'\0') + packet)
        clusters.append(element(0x1F43B675, uint(0xE7, i * 40) + blocks))
    return element(0x1A45DFA3, element(0x4282, b'matroska')) + element(
        0x18538067, element(0x1549A966, uint(0x2AD7B1, 1_000_000))
        + element(0x1654AE6B, tracks) + b''.join(clusters))

def generate():
    files, cases, controls = {}, [], []
    for family, module in [('avc', avc), ('hevc', hevc)]:
        sources = module.generate()
        manifest = json.loads(sources['manifest.json'])
        for c in manifest:
            name = c['name']; private, packets, size, keys = inventory(sources[f'{name}.mp4'])
            codec = 'V_MPEG4/ISO/AVC' if family == 'avc' else 'V_MPEGH/ISO/HEVC'
            if family == 'avc':
                old_count = new_count = 2
                expected = []
                for label, fmt in [('old', 0), ('new', 3)]:
                    _, pictures = make_pcm_packets(fmt, c['ydepth'], c['cdepth'])
                    for i, planes in enumerate(pictures):
                        values, _, _, depth, _ = pack_planar_oracle(fmt, c['ydepth'], c['cdepth'], [0]*4, planes)
                        path = f'avc-{c["stem"]}-{label}-{i}.raw'
                        files[path] = bytes(values) if depth == 8 else b''.join(v.to_bytes(2, 'little') for v in values)
                        expected.append(dict(oracle=path, depth=depth))
            else:
                old_count, new_count = c['old_frames'], c['new_frames']
                expected = []
                for label, stem, count, depth in [('old', hevc.OLD, old_count, 8), ('new', hevc.NEW, new_count, 10)]:
                    raw = (ROOT / f'{stem}.yuv').read_bytes(); stride = len(raw) // count
                    assert len(raw) == count * stride
                    for i in range(count):
                        path = f'hevc-{label}-{i}.raw'; files[path] = raw[i*stride:(i+1)*stride]
                        expected.append(dict(oracle=path, depth=depth))
            prefix = int(c['separate'])
            for i, frame in enumerate(expected):
                index = i + (prefix if i >= old_count else 0)
                frame['start_ns'] = index * 40_000_000
                frame['end_ns'] = (index + (2 if prefix and i + 1 == old_count else 1)) * 40_000_000
            for companion in [False, True]:
                file = f'{family}-{name}' + ('-companion' if companion else '') + '.mkv'
                files[file] = movie(codec, private, packets, size, keys, companion)
                cases.append(dict(file=file, codec=family, size=size, companion=companion,
                                  new_start=old_count, target_ns=(old_count+prefix)*40_000_000, frames=expected))
            if name == ('y8-c8-sps-pps-prefix' if family == 'avc' else '32-33-34-prefix'):
                for mode in ['skip-pictures', 'refuse-parameters']:
                    changed = list(packets)
                    if mode == 'skip-pictures':
                        # Header-only VCL NALs are not decodable pictures. A seek
                        # may ignore their entropy, but must receive later PS.
                        for i in range(old_count):
                            nal = (b'\x65\x80' if i == 0 else b'\x41\x80') if family == 'avc' else b'\x26\x01'
                            changed[i] = len(nal).to_bytes(4, 'big') + nal
                    else:
                        units = []; packet = changed[old_count]; at = 0
                        while at < len(packet):
                            n = int.from_bytes(packet[at:at+4], 'big'); at += 4
                            nal = packet[at:at+n]; at += n
                            if (nal[0] & 31 if family == 'avc' else nal[0] >> 1) == (8 if family == 'avc' else 34):
                                # Valid IDs, missing contextual PPS body.
                                nal = b'\x68\xc0' if family == 'avc' else b'\x44\x01\xc0'
                            units.append(len(nal).to_bytes(4, 'big') + nal)
                        changed[old_count] = b''.join(units)
                    file = f'{family}-{mode}.mkv'
                    files[file] = movie(codec, private, changed, size, keys, True)
                    controls.append(dict(file=file, codec=family, mode=mode, size=size,
                                         new_start=old_count, target_ns=(old_count+prefix)*40_000_000, frames=expected))
    assert len(cases) == 36
    files['manifest.json'] = (json.dumps(cases, indent=2) + '\n').encode()
    assert len(controls) == 4
    files['controls.json'] = (json.dumps(controls, indent=2) + '\n').encode()
    return files

if __name__ == '__main__':
    OUT.mkdir(exist_ok=True)
    for name, data in generate().items():
        (OUT / name).write_bytes(data)
