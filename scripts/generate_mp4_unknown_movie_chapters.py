#!/usr/bin/env python3
"""Own short VP9 chapter clock witnesses for unknown/unstated movie durations."""
from pathlib import Path
import struct, json, hashlib
from generate_vp9_chroma import frame, mp4, atom, word
from generate_mp4_tfhd_defaults_regressions import atoms
ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'tests/fixtures/playback-errors/mp4-unknown-movie-chapters'


def generate():
    packet = frame(8, '420', 16, 16)
    bases = {'regular': mp4([packet] * 3, 8, '420', (16, 16), [1, 2, 3]),
             'fragmented': (ROOT / 'tests/fixtures/playback-errors/mp4-tfhd-defaults/tfhd-overrides-trex-nonsync.mp4').read_bytes()}
    files = {'frame.vp9': packet}
    cases = {'ten_mhz': [], 'movie_clock': []}
    for container, base in bases.items():
        for version in [0, 1]:
            for state in ['known', 'unknown', 'zero']:
                for clock, starts in [('ten_mhz', [0, 200000, 400000]), ('movie_clock', [0, 20, 40])]:
                    duration = {'known': 60, 'unknown': (1 << (32 if version == 0 else 64)) - 1, 'zero': 0}[state]
                    chpl = word(0x01000000, 0) + bytes([3])
                    for start, title in zip(starts, [b'Opening', b'Middle', b'End']):
                        chpl += struct.pack('>Q', start) + bytes([len(title)]) + title
                    def rewrite(data):
                        out = b''
                        for kind, body in atoms(data):
                            if kind == b'moov':
                                body = rewrite(body) + atom(b'udta', atom(b'chpl', chpl))
                            elif kind == b'mvhd':
                                assert body[0] == 0 and len(body) == 100
                                if version == 0:
                                    body = body[:12] + word(1000, duration) + body[20:]
                                else:
                                    body = word(0x01000000) + struct.pack('>QQIQ', 0, 0, 1000, duration) + body[20:]
                            out += atom(kind, body)
                        return out
                    name = f'{container}-v{version}-{state}-{clock}.mp4'
                    files[name] = rewrite(base)
                    cases[clock].append(name)
    cases['edited_movie_clock'] = []
    for version in [0, 1]:
        for state in ['known', 'unknown']:
            base = files[f'regular-v{version}-known-movie_clock.mp4']
            def edit(data):
                out = b''
                for kind, body in atoms(data):
                    if kind in [b'moov', b'trak', b'udta']:
                        body = edit(body)
                        if kind == b'trak':
                            body += atom(b'edts', atom(b'elst', word(0, 2, 5000, 0xffffffff, 0x10000, 60, 0, 0x10000)))
                    elif kind == b'mvhd':
                        duration = 5060 if state == 'known' else (1 << (32 if version == 0 else 64)) - 1
                        at, width = (16, 4) if version == 0 else (24, 8)
                        body = body[:at] + duration.to_bytes(width, 'big') + body[at+width:]
                    elif kind == b'chpl':
                        body = word(0x01000000, 0) + bytes([3])
                        for start, title in zip([5000, 5020, 5040], [b'Opening', b'Middle', b'End']):
                            body += struct.pack('>Q', start) + bytes([len(title)]) + title
                    out += atom(kind, body)
                return out
            name = f'edited-v{version}-{state}-movie_clock.mp4'
            files[name] = edit(base)
            cases['edited_movie_clock'].append(name)
    OUT.mkdir(parents=True, exist_ok=True)
    for name, data in files.items():
        (OUT / name).write_bytes(data)
    (OUT / 'manifest.json').write_text(json.dumps({
        'provenance': 'Owned 16x16 VP9, three independent 20ms frames; media clock 50Hz, movie clock 1000Hz. '
                      'mvhd V0/V1 known 60 ticks, all-ones unknown or unstated zero; '
                      'chpl starts 0/20/40ms in 10MHz or legacy movie ticks. No FFmpeg/network/private input.',
        **cases, 'sha256': {n: hashlib.sha256(b).hexdigest() for n, b in files.items()},
    }, indent=2) + '\n')


if __name__ == '__main__':
    generate()
