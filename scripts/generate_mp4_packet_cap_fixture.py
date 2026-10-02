#!/usr/bin/env python3
"""Two-track MP4 from the authored temporal AVC seed, no external tools.

Tracks share the synthetic mdat bytes and timestamps, exposing global DTS ties.
Generation is separate from ordinary regression execution.
"""
import pathlib
import struct

ROOT = pathlib.Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors'


def boxes(data):
    offset = 0
    while offset < len(data):
        size, tag = struct.unpack_from('>I4s', data, offset)
        assert size >= 8 and offset + size <= len(data)
        yield tag, data[offset + 8:offset + size]
        offset += size


def box(tag, body):
    return struct.pack('>I4s', len(body) + 8, tag) + body


def main():
    data = (ROOT / 'avc-slice-lists-temporal.mp4').read_bytes()
    # Seed stores moov after mdat; duplicating metadata cannot shift payload offsets.
    parts = list(boxes(data))
    assert [tag for tag, _ in parts].index(b'mdat') < [tag for tag, _ in parts].index(b'moov')
    output = bytearray()
    for tag, body in parts:
        if tag == b'moov':
            children = list(boxes(body))
            tracks = [body for tag, body in children if tag == b'trak']
            assert len(tracks) == 1
            duplicate = bytearray()
            for child, payload in boxes(tracks[0]):
                if child == b'tkhd':
                    payload = bytearray(payload)
                    offset = 20 if payload[0] == 1 else 12
                    struct.pack_into('>I', payload, offset, 2)
                duplicate += box(child, payload)
            rebuilt = bytearray()
            for child, payload in children:
                if child == b'mvhd':
                    payload = payload[:-4] + struct.pack('>I', 3)
                rebuilt += box(child, payload)
            body = rebuilt + box(b'trak', duplicate)
        output += box(tag, body)
    (ROOT / 'mp4-global-packet-cap.mp4').write_bytes(output)
    # A separate authored tag-edit regression; no codec bytes are changed.
    tagged = bytearray()
    for tag, body in boxes(output):
        if tag == b'moov':
            body = b''.join(box(t, p) for t, p in boxes(body) if t != b'udta')
            entries = b''.join(box(t, box(b'data', b'\x00\x00\x00\x01' + bytes(4) + v.encode()))
                               for t, v in [(b'\xa9nam', 'Synthetic original title'),
                                            (b'\xa9ART', 'Synthetic original artist'),
                                            (b'\xa9alb', 'Synthetic retained album')])
            body += box(b'udta', box(b'meta', bytes(4) + box(b'ilst', entries)))
        tagged += box(tag, body)
    (ROOT / 'mp4-container-tag-edits.mp4').write_bytes(tagged)
    track_tagged = bytearray()
    for tag, body in boxes(tagged):
        if tag == b'moov':
            children = bytearray()
            index = 0
            for child, payload in boxes(body):
                if child == b'trak':
                    rebuilt = bytearray()
                    for field, value in boxes(payload):
                        if field == b'udta':
                            continue
                        if field == b'mdia':
                            media = bytearray()
                            for kind, contents in boxes(value):
                                if kind == b'mdhd':
                                    contents = bytearray(contents)
                                    code = 0
                                    for letter in ['eng', 'fra'][index]:
                                        code = (code << 5) | (ord(letter) - 96)
                                    struct.pack_into('>H', contents, 20 if contents[0] == 0 else 32, code)
                                media += box(kind, contents)
                            value = media
                        rebuilt += box(field, value)
                    rebuilt += box(b'udta', box(b'name', f'Synthetic track {index}'.encode()))
                    payload = rebuilt
                    index += 1
                children += box(child, payload)
            assert index == 2
            body = children
        track_tagged += box(tag, body)
    (ROOT / 'mp4-track-tag-edits.mp4').write_bytes(track_tagged)


if __name__ == '__main__':
    main()
