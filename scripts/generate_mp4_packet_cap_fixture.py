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


if __name__ == '__main__':
    main()
