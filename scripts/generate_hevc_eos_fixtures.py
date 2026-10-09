#!/usr/bin/env python3
"""Offline EOS boundary added to our short authored open-GOP fixture."""
import json
import struct
import subprocess
from generate_he_aac_packet_fixtures import DEST, boxes
from hevc_fixture_mp4 import box

def child(data, tag):
    return next(p for t, p in boxes(data) if t == tag)

def main():
    seed = DEST.parent / 'hevc' / 'weighted-tmvp.mp4'
    data = seed.read_bytes()
    metadata = json.loads(subprocess.check_output(['cargo', 'run', '--quiet', '--locked', '--offline', '--no-default-features', '--features', 'media,player', '--example', 'hevc_colour_plane_fixture_meta', '--', str(seed)]))
    cra = next(i for i, p in enumerate(metadata['packets']) if p[0]['kind'] == 21)
    assert cra > 0 and metadata['packets'][cra+1][0]['kind'] in (8, 9)
    root = dict(boxes(data))
    stbl = child(child(child(child(root[b'moov'], b'trak'), b'mdia'), b'minf'), b'stbl')
    sizes = child(stbl, b'stsz')
    constant, count = struct.unpack_from('>II', sizes, 4)
    assert constant == 0
    lengths = struct.unpack('>'+str(count)+'I', sizes[12:])
    chunks = child(stbl, b'stco')
    assert struct.unpack_from('>I', chunks, 4)[0] == 1
    at = struct.unpack_from('>I', chunks, 8)[0]
    original = []
    for n in lengths:
        original.append(data[at:at+n]); at += n
    for label, nal in [('valid', bytes([72, 1, 128])), ('bad-trailing', bytes([72, 1, 0])), ('bad-temporal', bytes([72, 2, 128])), ('bla-w-lp', None)]:
        packets = original.copy()
        if label == 'bla-w-lp':
            packet = bytearray(packets[cra])
            assert (packet[4] >> 1) & 63 == 21
            packet[4] = (packet[4] & 129) | (16 << 1)
            packets[cra] = bytes(packet)
        else:
            packets[cra-1] += len(nal).to_bytes(4, 'big') + nal
        def rewrite(tag, payload):
            if tag in (b'moov', b'trak', b'mdia', b'minf', b'stbl'):
                payload = b''.join(rewrite(t, p) for t, p in boxes(payload))
            elif tag == b'stsz':
                payload = payload[:12] + b''.join(struct.pack('>I', len(p)) for p in packets)
            elif tag == b'stco':
                payload = payload[:8] + struct.pack('>I', len(root[b'ftyp'])+16)
            return box(tag, payload)
        movie = box(b'ftyp', root[b'ftyp']) + box(b'mdat', b''.join(packets)) + rewrite(b'moov', root[b'moov'])
        name = 'hevc-bla-w-lp-synthetic.mp4' if label == 'bla-w-lp' else f'hevc-eos-before-cra-{label}-synthetic.mp4'
        (DEST / name).write_bytes(movie)

if __name__ == '__main__':
    main()
