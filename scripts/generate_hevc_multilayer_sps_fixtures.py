#!/usr/bin/env python3
"""Own SPS Annex F extension on synthetic I/P/B seeds; offline, no codec tools."""
import json
import struct
from generate_he_aac_packet_fixtures import DEST, boxes
from hevc_fixture_mp4 import box
from generate_hevc_colour_plane_fixtures import bits, unescape, escape, packed


def extension(nal, constraint, range_extension, future, malformed, three_d):
    value = bits(unescape(nal[2:]))
    syntax = value[:value.rfind('1')]
    assert syntax[-1] == '0', 'seed SPS must have no extension'
    flags = str(int(range_extension)) + '1' + str(int(three_d)) + '0'
    flags += '0001' if future else '0000'
    body = ('0' * 9 if range_extension else '')
    if malformed != 'missing-constraint':
        body += str(int(constraint))
    body += '0010110010' if future else ''
    stop = '' if malformed == 'missing-stop' else '1'
    return nal[:2] + escape(packed(syntax[:-1] + '1' + flags + body + stop))


def main():
    cases = []
    for seed in ['main-ipb', 'main10-ipb']:
        data = (DEST.parent / 'hevc' / f'{seed}.mp4').read_bytes()
        roots = list(boxes(data))
        assert roots[-1][0] == b'moov'
        for kind, constraint, rext, future, malformed, three_d in [
            ('unconstrained', False, False, False, None, False),
            ('constrained', True, False, False, None, False),
            ('range', True, True, False, None, False),
            ('range-future', True, True, True, None, False),
            ('missing-constraint', False, True, False, 'missing-constraint', False),
            ('missing-stop', False, True, False, 'missing-stop', False),
            ('three-d', True, True, False, None, True),
        ]:
            def config(data):
                out = bytearray(data[:23])
                at = 23
                changed = 0
                for _ in range(data[22]):
                    header = data[at:at+3]
                    at += 3
                    out += header
                    for _ in range(int.from_bytes(header[1:], 'big')):
                        n = int.from_bytes(data[at:at+2], 'big')
                        at += 2
                        nal = data[at:at+n]
                        at += n
                        if header[0] & 63 == 33:
                            nal = extension(nal, constraint, rext, future, malformed, three_d)
                            changed += 1
                        out += struct.pack('>H', len(nal)) + nal
                assert at == len(data) and changed == 1
                return bytes(out)

            def rewrite(tag, payload):
                if tag in (b'moov', b'trak', b'mdia', b'minf', b'stbl'):
                    payload = b''.join(rewrite(t, p) for t, p in boxes(payload))
                elif tag == b'stsd':
                    payload = payload[:8] + b''.join(
                        box(t, p[:78] + b''.join(
                            box(k, config(v) if k == b'hvcC' else v)
                            for k, v in boxes(p[78:])
                        )) for t, p in boxes(payload[8:])
                    )
                return box(tag, payload)

            name = f'hevc-multilayer-sps-{seed}-{kind}-synthetic.mp4'
            (DEST / name).write_bytes(b''.join(rewrite(t, p) for t, p in roots))
            cases.append(dict(file=name, seed=seed, kind=kind, constraint=constraint,
                              expectation='invalid' if malformed else 'refusal' if three_d else 'acceptance'))
    (DEST / 'hevc-multilayer-sps.json').write_text(json.dumps(dict(
        cases=cases,
        provenance='Original synthetic I/P/B seeds documented in fixtures/hevc/README.md. '
        'Own SPS multilayer constraint, range-extension ordering, future-tail and malformed '
        'RBSP rewriting. This tests base-layer playback, not inter-layer prediction or '
        '3D acceptance. No private media, parameter sets, external tool or network.'
    ), indent=2) + '\n')
    print('Multilayer SPS cases:', len(cases))


if __name__ == '__main__':
    main()
