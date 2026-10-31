#!/usr/bin/env python3
"""Author a manual MACE seek workload, separately from ordinary tests."""
import hashlib
import json
import sys
from pathlib import Path
from generate_mp4_fragment_description_regressions import atoms
from generate_vp9_chroma import atom, word


def generate(count):
    assert 40 <= count <= 1000000
    source = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors/mp4-mace-transitions/control-0.mp4'
    original = dict(atoms(source.read_bytes()))
    # The original authored video and its chunk offsets are unchanged. Replace
    # the final 80-byte arithmetic MACE payload with count fresh two-byte blocks.
    video = original[b'mdat'][:-80]
    payload = bytes((j * 13 + 7) % 256 for j in range(count * 2))
    offset = len(atom(b'ftyp', original[b'ftyp'])) + 8 + len(video)
    duration = count * 36

    def walk(data, audio=False):
        out = b''
        for kind, body in atoms(data):
            if kind == b'trak':
                body = walk(body, b'soun' in body)
            elif kind in [b'moov', b'mdia', b'minf', b'stbl']:
                body = walk(body, audio)
                if kind == b'stbl' and audio:
                    # An explicit zero CTTS run is equivalent to absence but
                    # exercises the supported expanded-index representation.
                    body += atom(b'ctts', word(0, 1, count * 6, 0))
            elif audio and kind == b'stts':
                body = word(0, 1, count * 6, 6)
            elif audio and kind == b'stsz':
                body = word(0, 1, count * 6)
            elif audio and kind == b'stsc':
                body = word(0, 1, 1, count * 6, 1)
            elif audio and kind == b'stco':
                body = word(0, 1, offset)
            elif kind == b'mvhd':
                body = bytearray(body)
                body[16:20] = word(max(int.from_bytes(body[16:20], 'big'), (duration * 50 + 47999) // 48000))
            elif audio and kind in [b'tkhd', b'mdhd']:
                body = bytearray(body)
                if kind == b'tkhd':
                    body[20:24] = word((duration * 50 + 47999) // 48000)
                else:
                    body[16:20] = word(duration)
            out += atom(kind, body)
        return out

    data = atom(b'ftyp', original[b'ftyp']) + atom(b'mdat', video + payload) + atom(b'moov', walk(original[b'moov']))
    return data


if __name__ == '__main__':
    path = Path(sys.argv[1])
    count = int(sys.argv[2]) if len(sys.argv) > 2 else 160000
    data = generate(count)
    path.write_bytes(data)
    path.with_suffix('.json').write_text(json.dumps({
        'provenance': 'Authored arithmetic MACE blocks and existing authored neutral VP9 only; no private media, encoder or network. Manual benchmark input.',
        'blocks': count, 'timescale': 48000, 'block_ticks': 36,
        'sha256': hashlib.sha256(data).hexdigest(), 'bytes': len(data)
    }, indent=2) + '\n')
