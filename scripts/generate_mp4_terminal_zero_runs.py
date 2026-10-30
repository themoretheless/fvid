#!/usr/bin/env python3
"""Authored STTS grouping witnesses, with real packet/block granularity."""
from pathlib import Path
import hashlib
import json
import struct
from generate_mp4_seek_io_regressions import generate
from generate_mp4_compact_sizes import compact
from generate_mp4_tfhd_defaults_regressions import atoms
from generate_vp9_chroma import atom, word

OUT = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors/mp4-terminal-zero-runs'


def main():
    seed = generate()
    files = {n: b for n, b in seed.items() if n != 'three-colors.mp4'}
    valid = []
    refused = []
    programs = [
        ('positive-grouped', [(3, 1)], None),
        ('positive-split', [(1, 1), (2, 1)], None),
        ('single-zero-grouped', [(2, 1), (1, 0)], None),
        ('single-zero-split', [(1, 1), (1, 1), (1, 0)], None),
        ('two-zero-grouped', [(1, 1), (2, 0)], 'new'),
        ('all-zero-grouped', [(3, 0)], 'new'),
        ('two-zero-split', [(1, 1), (1, 0), (1, 0)], 'existing'),
        ('all-zero-split', [(1, 0), (2, 0)], 'existing'),
        ('middle-zero', [(1, 0), (2, 1)], 'existing'),
    ]
    for size_mode in ['stsz', 'stz2']:
        for label, rows, category in programs:
            duration = sum(n * d for n, d in rows)
            def rewrite(data):
                out = b''
                for k, b in atoms(data):
                    if k in [b'moov', b'trak', b'mdia', b'minf', b'stbl']:
                        b = rewrite(b)
                    elif k == b'stts':
                        b = word(0, len(rows)) + b''.join(word(n, d) for n, d in rows)
                    elif k in [b'mvhd', b'mdhd']:
                        b = b[:16] + word(duration) + b[20:]
                    elif k == b'tkhd':
                        b = b[:20] + word(duration) + b[24:]
                    elif k == b'stsz' and size_mode == 'stz2':
                        k, b = b'stz2', compact(16, [len(seed[f'frame-{i}.packet']) for i in range(3)])
                    out += atom(k, b)
                return out
            name = f'{size_mode}-{label}.mp4'
            files[name] = rewrite(seed['three-colors.mp4'])
            c = {'file': name, 'rows': rows, 'duration_ticks': duration, 'size_mode': size_mode}
            if category:
                c['category'] = category; refused.append(c)
            else:
                c['durations'] = [d for n, d in rows for _ in range(n)]
                c['visible'] = 2 if c['durations'][-1] == 0 else 3
                valid.append(c)
    audio = []
    parent = OUT.parent / 'mp4-compact-sizes'
    files['pcm.s16le'] = (parent / 'pcm.s16le').read_bytes()
    files['mace.packet'] = bytes(2)
    for codec, unit in [('pcm', 1), ('mace', 6)]:
        source = (parent / f'{codec}-stsz-4.mp4').read_bytes()
        cases = [('last-zero-packet', [(2880 - unit, 1), (unit, 0)], None),
                 ('one-uniform-zero-packet', [(unit, 0)], None),
                 ('two-zero-packets', [(2880 - 2 * unit, 1), (2 * unit, 0)], 'new'),
                 ('all-zero-packets', [(2880, 0)], 'new'),
                 ('middle-zero-packet', [(unit, 0), (2880 - unit, 1)], 'existing')]
        for label, rows, category in cases:
            count = sum(n for n, _ in rows)
            duration = sum(n * d for n, d in rows)
            def sound(data):
                out = b''
                for k, b in atoms(data):
                    if k in [b'mdia', b'minf', b'stbl']:
                        b = sound(b)
                    elif k == b'mdhd':
                        b = b[:16] + word(duration) + b[20:]
                    elif k == b'tkhd':
                        b = b[:20] + word((duration * 50 + 47999) // 48000) + b[24:]
                    elif k == b'stts':
                        b = word(0, len(rows)) + b''.join(word(n, d) for n, d in rows)
                    elif k == b'stsz':
                        b = b[:8] + word(count)
                    elif k == b'stsc':
                        b = word(0, 1, 1, count, 1)
                    out += atom(k, b)
                return out
            def movie(data):
                out = b''
                for k, b in atoms(data):
                    if k == b'moov':
                        b = movie(b)
                    elif k == b'trak':
                        mdia = next(v for t, v in atoms(b) if t == b'mdia')
                        handler = next(v for t, v in atoms(mdia) if t == b'hdlr')[8:12]
                        if handler == b'soun': b = sound(b)
                    out += atom(k, b)
                return out
            name = f'{codec}-{label}.mp4'
            files[name] = movie(source)
            c = {'file': name, 'codec': codec, 'unit': unit, 'rows': rows,
                 'raw_count': count, 'packet_count': count // unit, 'audio_duration_ticks': duration}
            if category:
                c['category'] = category; refused.append(c)
            else: audio.append(c)
    OUT.mkdir(parents=True, exist_ok=True)
    for n, b in files.items():
        (OUT / n).write_bytes(b)
    (OUT / 'manifest.json').write_text(json.dumps({
        'provenance': 'Only authored16x16 VP9 and previously authored60ms PCM/MACE controls. Logical per-sample durations are encoded with grouped/split STTS rows; valid zero tails retain one actual packet, including one six-sample MACE block. No private media/parameters, FFmpeg or network.',
        'reference': 'https://b.goeswhere.com/ISO_IEC_14496-12_2015.pdf#page=49',
        'valid_video': valid, 'valid_audio': audio, 'refused': refused,
        'sha256': {n: hashlib.sha256(b).hexdigest() for n, b in files.items()},
    }, indent=2) + '\n')


if __name__ == '__main__':
    main()
