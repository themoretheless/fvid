#!/usr/bin/env python3
"""Owned in-band Annex F VPS alignment updates; saved reset families are inputs."""
import json
import struct
from generate_he_aac_packet_fixtures import DEST, boxes
from generate_hevc_colour_plane_fixtures import child, escape, unescape
from hevc_fixture_mp4 import box


def parts(data, parameter_type=32):
    roots = dict(boxes(data))
    stbl = child(child(child(child(roots[b'moov'], b'trak'), b'mdia'), b'minf'), b'stbl')
    entry = next(boxes(child(stbl, b'stsd')[8:]))[1]
    config = child(entry[78:], b'hvcC')
    at = 23
    vps = None
    for _ in range(config[22]):
        header = config[at:at+3]; at += 3
        for _ in range(int.from_bytes(header[1:], 'big')):
            n = int.from_bytes(config[at:at+2], 'big'); at += 2
            if header[0] & 63 == parameter_type: vps = config[at:at+n]
            at += n
    assert at == len(config) and vps is not None
    table = child(stbl, b'stsz')
    constant, count = struct.unpack_from('>II', table, 4)
    assert constant == 0 and count == 35
    sizes = struct.unpack('>35I', table[12:])
    at = struct.unpack_from('>I', child(stbl, b'stco'), 8)[0]
    packets = []
    for n in sizes:
        packets.append(data[at:at+n]); at += n
    return roots, vps, packets


def main():
    cases = []
    for kind, picture, label, after in [('msb-reset', 16, 'bad-in-sequence', False), ('full-reset', 17, 'at-idr', False), ('msb-reset', 16, 'bad-after-slices', True), ('baseline', 16, 'annex-a-after-slices', True), ('baseline', 17, 'annex-a-to-reset-at-idr', False), ('msb-reset', 16, 'reserved-after-slices', True)]:
        source = f'hevc-poc-vps-aligned-{kind}-synthetic.mp4'
        roots, original_vps, packets = parts((DEST / source).read_bytes())
        _, replacement, _ = parts((DEST / f'hevc-poc-vps-unaligned-{kind}-synthetic.mp4').read_bytes())
        if label == 'reserved-after-slices':
            raw = bytearray(unescape(original_vps[2:]))
            raw[2] ^= 1  # reserved 0xffff field must be ignored (7.4.3.1)
            replacement = original_vps[:2] + escape(bytes(raw))
        opaque = None
        activation = label == 'annex-a-to-reset-at-idr'
        if activation:
            full = (DEST / 'hevc-poc-vps-unaligned-full-reset-synthetic.mp4').read_bytes()
            _, replacement, full_packets = parts(full)
            _, pps, _ = parts(full, 34)
            packets[picture:] = full_packets[picture:]
            packets[picture] = len(pps).to_bytes(4, 'big') + pps + packets[picture]
            _, opaque, _ = parts((DEST / 'hevc-poc-vps-aligned-bad-vps-alignment-synthetic.mp4').read_bytes())
        update = len(replacement).to_bytes(4, 'big') + replacement
        packets[picture] = packets[picture] + update if after else update + packets[picture]
        def rewrite(tag, payload):
            if tag in (b'moov', b'trak', b'mdia', b'minf', b'stbl'):
                payload = b''.join(rewrite(t, p) for t, p in boxes(payload))
            elif tag == b'stsd' and opaque is not None:
                def configuration(data):
                    result = bytearray(data[:23]); at = 23
                    for _ in range(data[22]):
                        header = data[at:at+3]; at += 3; result += header
                        for _ in range(int.from_bytes(header[1:], 'big')):
                            n = int.from_bytes(data[at:at+2], 'big'); at += 2
                            nal = data[at:at+n]; at += n
                            if header[0] & 63 == 32: nal = opaque
                            result += len(nal).to_bytes(2, 'big') + nal
                    assert at == len(data)
                    return bytes(result)
                payload = payload[:8] + b''.join(box(t, p[:78] + b''.join(box(k, configuration(v) if k == b'hvcC' else v) for k, v in boxes(p[78:]))) for t, p in boxes(payload[8:]))
            elif tag == b'stss' and activation:
                payload = bytes(4) + struct.pack('>IIII', 3, 1, 18, 34)
            elif tag == b'stsz':
                payload = bytes(8) + struct.pack('>I', 35) + b''.join(struct.pack('>I', len(p)) for p in packets)
            return box(tag, payload)
        name = f'hevc-poc-vps-alignment-update-{label}-synthetic.mp4'
        (DEST / name).write_bytes(box(b'ftyp', roots[b'ftyp']) + box(b'mdat', b''.join(packets)) + rewrite(b'moov', roots[b'moov']))
        cases.append(dict(file=name, seed=source, picture=picture,
                          expectation='invalid' if label.startswith('bad-') else 'acceptance',
                          error=('HEVC changed VPS follows picture slices' if after else 'HEVC active VPS changed within sequence') if label.startswith('bad-') else None))
    (DEST / 'hevc-poc-vps-alignment-updates.json').write_text(json.dumps(dict(cases=cases,
        provenance='Original owned reset streams; in-band VPS/PPS replacement only, including IDR activation after an opaque Annex A tail. No private media, external codec, FFmpeg or network.'), indent=2) + '\n')

if __name__ == '__main__': main()
