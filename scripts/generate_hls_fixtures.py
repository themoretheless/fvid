#!/usr/bin/env python3
"""Split the committed synthetic fragmented video; no encoder or network needed."""
from pathlib import Path
import struct
root = Path(__file__).resolve().parents[1]
source = (root / 'tests/fixtures/fragmented/video.mp4').read_bytes()
out = root / 'tests/fixtures/playback-errors/hls'
out.mkdir(parents=True, exist_ok=True)
boxes = []
at = 0
while at < len(source):
    size, kind = struct.unpack_from('>I4s', source, at)
    if size == 1:
        size = struct.unpack_from('>Q', source, at + 8)[0]
    elif size == 0:
        size = len(source) - at
    assert size >= 8 and at + size <= len(source)
    boxes.append((kind, source[at:at + size]))
    at += size
init = b''.join(data for kind, data in boxes if kind in (b'ftyp', b'moov'))
fragments = []
for index, (kind, data) in enumerate(boxes):
    if kind == b'moof':
        assert boxes[index + 1][0] == b'mdat'
        fragments.append(data + boxes[index + 1][1])
assert len(fragments) == 5
(out / 'init.mp4').write_bytes(init)
(out / 'objects.mp4').write_bytes(init + b''.join(fragments))
header = '#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:1\n'
media = header + '#EXT-X-MAP:URI="init.mp4"\n'
ranges = header + f'#EXT-X-MAP:URI="objects.mp4",BYTERANGE="{len(init)}@0"\n'
offset = len(init)
for index, fragment in enumerate(fragments):
    name = f'segment-{index}.m4s'
    (out / name).write_bytes(fragment)
    media += f'#EXTINF:0.2,\n{name}\n'
    byte_range = f'{len(fragment)}@{offset}' if index == 0 else str(len(fragment))
    ranges += f'#EXTINF:0.2,\n#EXT-X-BYTERANGE:{byte_range}\nobjects.mp4\n'
    offset += len(fragment)
(out / 'media.m3u8').write_text(media + '#EXT-X-ENDLIST\n')
(out / 'byterange.m3u8').write_text(ranges + '#EXT-X-ENDLIST\n')
(out / 'master.m3u8').write_text('#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=200000,CODECS="avc1.64000d"\nmedia.m3u8\n')
