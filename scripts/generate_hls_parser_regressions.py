#!/usr/bin/env python3
"""HLS parser failures around a committed one-second synthetic fMP4 seed.

Never reads private media and does not invoke an external tool or network.
"""
import hashlib
import json
import struct
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "tests/fixtures/playback-errors/hls-parser"


def main():
    source = (ROOT / "tests/fixtures/fragmented/video.mp4").read_bytes()
    at = 0
    while at < len(source):
        size, tag = struct.unpack_from(">I4s", source, at)
        if size == 1:
            size = struct.unpack_from(">Q", source, at + 8)[0]
        elif size == 0:
            size = len(source) - at
        assert size >= 8 and at + size <= len(source)
        if tag == b"moof":
            break
        at += size
    assert 0 < at < len(source)
    OUT.mkdir(parents=True, exist_ok=True)
    assets = {"control.mp4": source, "init.mp4": source[:at], "segment.m4s": source[at:]}
    for name, data in assets.items():
        (OUT / name).write_bytes(data)
    header = '#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:1\n'
    media = header + '#EXT-X-MAP:URI="init.mp4"\n#EXTINF:1,\nsegment.m4s\n#EXT-X-ENDLIST\n'
    cases = {"control.m3u8": media}
    for name, tag in {
        "version": "#EXT-X-VERSION:6",
        "target-duration": "#EXT-X-TARGETDURATION:2",
        "media-sequence": "#EXT-X-MEDIA-SEQUENCE:1\n#EXT-X-MEDIA-SEQUENCE:2",
        "endlist": "#EXT-X-ENDLIST",
        "independent-segments": "#EXT-X-INDEPENDENT-SEGMENTS\n#EXT-X-INDEPENDENT-SEGMENTS",
    }.items():
        cases[f"duplicate-{name}.m3u8"] = media.replace('#EXT-X-MAP:', tag + '\n#EXT-X-MAP:')
    cases['duplicate-endlist.m3u8'] = media + '#EXT-X-ENDLIST\n'
    cases["mixed-media-rendition.m3u8"] = media.replace('#EXT-X-MAP:',
        '#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID="a",NAME="a",URI="audio.m3u8"\n#EXT-X-MAP:')
    cases["mixed-master-sequence.m3u8"] = (
        '#EXTM3U\n#EXT-X-MEDIA-SEQUENCE:0\n#EXT-X-STREAM-INF:BANDWIDTH=1000\ncontrol.m3u8\n')
    for name, original, replacement in [
        ("signed-target", 'TARGETDURATION:1', 'TARGETDURATION:+1'),
        ("signed-sequence", '#EXT-X-MAP:', '#EXT-X-MEDIA-SEQUENCE:+0\n#EXT-X-MAP:'),
    ]:
        cases[name + '.m3u8'] = media.replace(original, replacement)
    cases["signed-bandwidth.m3u8"] = '#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=+1000\ncontrol.m3u8\n'
    cases['exponent-duration.m3u8'] = media.replace('#EXTINF:1,', '#EXTINF:1e0,')
    cases['signed-duration.m3u8'] = media.replace('#EXTINF:1,', '#EXTINF:+1,')
    cases['signed-range-length.m3u8'] = media.replace('URI="init.mp4"',
        f'URI="init.mp4",BYTERANGE="+{at}@0"')
    cases['signed-range-offset.m3u8'] = media.replace('URI="init.mp4"',
        f'URI="init.mp4",BYTERANGE="{at}@+0"')
    for name, text in cases.items():
        (OUT / name).write_text(text)
    inventory = {name: hashlib.sha256((OUT / name).read_bytes()).hexdigest()
                 for name in sorted([*assets, *cases])}
    (OUT / 'manifest.json').write_text(json.dumps({
        'provenance': 'Committed synthetic fragmented/video.mp4: 25 pictures, one second. Metadata-only HLS mutations. No private input, external tool or network.',
        'files': inventory,
    }, indent=2) + '\n')


if __name__ == '__main__':
    main()
