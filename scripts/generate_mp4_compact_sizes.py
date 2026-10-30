#!/usr/bin/env python3
"""Authored valid ISO compact sample-size tables; no encoder or network."""
from pathlib import Path
import hashlib
import json
import struct
from generate_mp4_seek_io_regressions import generate
from generate_mp4_subtitle_negative_clock import text_entry
from generate_mp4_tfhd_defaults_regressions import atoms
from generate_vp9_chroma import atom, word

OUT = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors/mp4-compact-sizes'


def compact(width, sizes):
    if width == 4:
        payload = bytes((sizes[i] << 4) | (sizes[i + 1] if i + 1 < len(sizes) else 0)
                        for i in range(0, len(sizes), 2))
    else:
        payload = b''.join(x.to_bytes(width // 8, 'big') for x in sizes)
    return word(0) + bytes(3) + bytes([width]) + word(len(sizes)) + payload


def main():
    seed = generate()
    files = {n: b for n, b in seed.items() if n != 'three-colors.mp4'}
    parts = list(atoms(seed['three-colors.mp4']))
    prefix = b''.join(atom(k, b) for k, b in parts[:2])
    moov = parts[2][1]
    video = next(b for k, b in atoms(moov) if k == b'trak')
    video_sizes = [len(seed[f'frame-{i}.packet']) for i in range(3)]
    cases = []
    def video_table(data, use_compact):
        out = b''
        for k, b in atoms(data):
            if k in [b'trak', b'mdia', b'minf', b'stbl']:
                b = video_table(b, use_compact)
            elif k == b'stsz' and use_compact:
                k, b = b'stz2', compact(16, video_sizes)
            out += atom(k, b)
        return out
    for mode in ['stsz', 'stz2']:
        n = f'video-{mode}-16.mp4'
        files[n] = prefix + atom(b'moov', video_table(moov, mode == 'stz2'))
        cases.append({'file': n, 'mode': mode, 'target': 'video', 'width': 16})
    for width, texts in [(4, ['A', 'BBB', 'CC']), (4, ['A', 'BBB', 'CC', 'DDDD']),
                         (8, ['A', 'B' * 17, 'C' * 129]),
                         (16, ['A', 'B' * 255, 'C' * 511])]:
        packets = [struct.pack('>H', len(t.encode())) + t.encode() for t in texts]
        clock = 100 if len(texts) == 4 else 50
        sizes = list(map(len, packets))
        for mode in ['stsz', 'stz2']:
            def text_track(data):
                out = b''
                for k, b in atoms(data):
                    if k in [b'mdia', b'minf']:
                        b = text_track(b)
                    elif k == b'tkhd':
                        b = word(0) + b[4:12] + word(2) + b[16:20] + word(2 if clock == 100 else 3) + b[24:]
                    elif k == b'mdhd':
                        b = b[:12] + word(clock, len(texts)) + b[20:]
                    elif k == b'hdlr':
                        b = word(0, 0) + b'sbtl' + bytes(12) + b'Authored text\0'
                    elif k == b'vmhd':
                        k, b = b'nmhd', word(0)
                    elif k == b'stbl':
                        b = atom(b'stsd', word(0, 1) + text_entry())
                        b += atom(b'stts', word(0, 1, len(texts), 1))
                        b += atom(b'stsz', word(0, 0, len(texts), *sizes)) if mode == 'stsz' else atom(b'stz2', compact(width, sizes))
                        b += atom(b'stsc', word(0, 1, 1, len(texts), 1))
                        b += atom(b'stco', word(0, 1, len(prefix) + 8))
                    out += atom(k, b)
                return out
            text = atom(b'trak', text_track(video))
            n = f'text-{mode}-{width}-{len(texts)}.mp4'
            files[n] = prefix + atom(b'mdat', b''.join(packets)) + atom(b'moov', moov + text)
            cases.append({'file': n, 'mode': mode, 'target': 'text', 'width': width,
                          'texts': texts, 'sizes': sizes, 'timescale': clock})
            for i, packet in enumerate(packets):
                files[f'text-{width}-{len(texts)}-{i}.packet'] = packet
    pcm_values = [(i % 7 - 3) * 1000 for i in range(2880)]
    files['pcm.s16le'] = b''.join(struct.pack('<h', x) for x in pcm_values)
    files['pcm.f32le'] = b''.join(struct.pack('<f', x / 32768) for x in pcm_values)
    for mode, width in [('stsz', 4), ('stz2', 4), ('stz2', 8), ('stz2', 16)]:
        def audio_track(data):
            out = b''
            for k, b in atoms(data):
                if k in [b'mdia', b'minf']:
                    b = audio_track(b)
                elif k == b'tkhd':
                    b = word(0) + b[4:12] + word(2) + b[16:-8] + bytes(8)
                elif k == b'mdhd':
                    b = b[:12] + word(48000, 2880) + b[20:]
                elif k == b'hdlr':
                    b = word(0, 0) + b'soun' + bytes(12) + b'Authored PCM\0'
                elif k == b'vmhd':
                    k, b = b'smhd', word(0, 0)
                elif k == b'stbl':
                    entry = atom(b'sowt', bytes(6) + struct.pack('>H', 1) + bytes(8)
                                 + struct.pack('>HHHHI', 1, 16, 0, 0, 48000 << 16))
                    b = atom(b'stsd', word(0, 1) + entry) + atom(b'stts', word(0, 1, 2880, 1))
                    b += atom(b'stsz', word(0, 2, 2880)) if mode == 'stsz' else atom(b'stz2', compact(width, [2] * 2880))
                    b += atom(b'stsc', word(0, 1, 1, 2880, 1)) + atom(b'stco', word(0, 1, len(prefix) + 8))
                out += atom(k, b)
            return out
        name = f'pcm-{mode}-{width}.mp4'
        files[name] = prefix + atom(b'mdat', files['pcm.s16le']) + atom(b'moov', moov + atom(b'trak', audio_track(video)))
        cases.append({'file': name, 'mode': mode, 'target': 'pcm', 'width': width,
                      'frames': 2880, 'sample_rate': 48000, 'channels': 1})
    def mace_movie(mode, zero=False):
        def rewrite(data):
            out = b''
            for k, b in atoms(data):
                if k in [b'moov', b'trak', b'mdia', b'minf', b'stbl']:
                    b = rewrite(b)
                elif k == b'mdat' and b == files['pcm.s16le']:
                    b = bytes(960)
                elif k == b'stsd':
                    b = b[:8] + b''.join(atom(b'MAC3' if ek == b'sowt' else ek, eb)
                                        for ek, eb in atoms(b[8:]))
                elif k == b'stsz' and struct.unpack('>I', b[8:12])[0] == 2880:
                    if mode == 'stsz':
                        b = word(0, 1, 2880)
                    else:
                        entries = [1] * 2880
                        if zero:
                            entries[10] = 0
                        k, b = b'stz2', compact(4, entries)
                out += atom(k, b)
            return out
        return rewrite(files['pcm-stsz-4.mp4'])
    for mode in ['stsz', 'stz2']:
        name = f'mace-{mode}-4.mp4'
        files[name] = mace_movie(mode)
        cases.append({'file': name, 'mode': mode, 'target': 'mace', 'width': 4})
    malformed = []
    files['bad-mace-zero-entry.mp4'] = mace_movie('stz2', True)
    malformed.append({'file': 'bad-mace-zero-entry.mp4', 'damage': 'zero-entry'})
    for damage in ['both', 'missing', 'field-0', 'field-3', 'field-32', 'reserved', 'flags',
                   'version', 'truncated', 'trailing', 'count-large', 'zero-entry']:
        def damaged(data):
            out = b''
            for k, b in atoms(data):
                if k in [b'moov', b'trak', b'mdia', b'minf', b'stbl']:
                    b = damaged(b)
                elif k == b'stz2':
                    if damage == 'both':
                        out += atom(b'stsz', word(0, 0, 3, *video_sizes))
                    elif damage == 'missing':
                        continue
                    elif damage.startswith('field-'):
                        b = b[:7] + bytes([int(damage.split('-')[1])]) + b[8:]
                    elif damage == 'reserved':
                        b = b[:4] + b'\x01' + b[5:]
                    elif damage == 'flags':
                        b = word(1) + b[4:]
                    elif damage == 'version':
                        b = word(1 << 24) + b[4:]
                    elif damage == 'truncated':
                        b = b[:-1]
                    elif damage == 'trailing':
                        b += b'\0'
                    elif damage == 'count-large':
                        b = b[:8] + word(0xffffffff) + b[12:]
                    elif damage == 'zero-entry':
                        b = b[:12] + bytes(2) + b[14:]
                out += atom(k, b)
            return out
        name = f'bad-{damage}.mp4'
        files[name] = damaged(files['video-stz2-16.mp4'])
        malformed.append({'file': name, 'damage': damage})
    # Reuse only the earlier authored open-edit witness: its real text moof
    # populates an empty init table. No external/private source is involved.
    fragment_source = (OUT.parent / 'mp4-subtitle-edits/open-1000.mp4').read_bytes()
    fragmented = []
    for width in [None, 4, 8, 16]:
        def init_table(data):
            out = b''
            for k, b in atoms(data):
                if k in [b'moov', b'trak', b'mdia', b'minf', b'stbl']:
                    b = init_table(b)
                elif k == b'stsz' and struct.unpack('>I', b[8:12])[0] == 0 and width is not None:
                    k, b = b'stz2', compact(width, [])
                out += atom(k, b)
            return out
        name = f'fragment-init-{width if width else "stsz"}.mp4'
        files[name] = init_table(fragment_source)
        fragmented.append(name)
    for i, text in enumerate(['First caption', 'Middle caption', 'Last caption']):
        files[f'fragment-text-{i}.packet'] = struct.pack('>H', len(text)) + text.encode()
    OUT.mkdir(parents=True, exist_ok=True)
    for n, b in files.items():
        (OUT / n).write_bytes(b)
    (OUT / 'manifest.json').write_text(json.dumps({
        'provenance': 'Authored16x16 VP9 and invented ASCII captions. stsz controls and stz2 alternatives contain identical packets/timing. Compact field sizes4/8/16, odd/even nibble counts, byte/word-spanning values; authored60ms mono48kHz PCM formula and zero-coded MACE blocks, with separate malformed-table refusals. No private media/parameters, FFmpeg or network.',
        'reference': 'ISO/IEC14496-12:2015 8.7.3.3 Compact Sample Size Box',
        'cases': cases, 'malformed': malformed, 'fragmented': fragmented,
        'sha256': {n: hashlib.sha256(b).hexdigest() for n, b in files.items()},
    }, indent=2) + '\n')


if __name__ == '__main__':
    main()
