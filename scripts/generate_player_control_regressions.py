#!/usr/bin/env python3
"""Short authored clips carrying adversarial playback-control text/metadata."""
import hashlib
import json
from pathlib import Path
from generate_audio_resample_window_fixture import ebml
from generate_vp9_chroma import frame

OUT = Path(__file__).resolve().parents[1] / "tests/fixtures/playback-errors/player-controls"


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    clip = b"YUV4MPEG2 W2 H2 F25:1 Ip C420jpeg\n" + b"".join(
        b"FRAME\n" + bytes([32 + i * 16]) * 4 + bytes([128, 128]) for i in range(3))
    files = {"control.y4m": clip, "clip with spaces.y4m": clip, "clip@name.y4m": clip}
    files['overflow-clock.srt'] = (
        '1\n9223372036854775807:00:00,000 --> 9223372036854775807:00:01,000\nOverflow\n\n'
        '2\n00:00:00,000 --> 00:00:00,040\nValid\n').encode()
    files['invalid-time.smil'] = b'control.y4m@NaN\n'
    files['invalid-time.edl'] = b'control.y4m 0 inf\n'
    files['quoted-path.smil'] = b'src="clip with spaces.y4m" begin=0.04\n'
    files['quoted-path.edl'] = b'"clip with spaces.y4m" 0.04 0.08\n'
    files['numeric-controls.json'] = (json.dumps({
        'display_delta_us': -(1 << 63),
        'sync_local_us': (1 << 63) - 1, 'sync_peer_us': -(1 << 63),
        'duration_us': 1_000_000_000_000_000_000,
        'provenance': 'Boundary inputs for public playback-control APIs, alongside an authored decodable control clip. These qualify control arithmetic, not a GUI session or a claimed long-duration video.',
    }, indent=2) + '\n').encode()
    metadata = {'CHAPTER_ART_URL': 'a' * 47 + '\u20ac', 'MEDIA_FINGERPRINT': 'a' * 11 + '\U0001f600'}
    tags = b''.join(ebml('7373', ebml('67c8', ebml('45a3', key.encode())
                                    + ebml('4487', value.encode()))) for key, value in metadata.items())
    track = ebml('ae', ebml('d7', b'\1') + ebml('83', b'\1') + ebml('86', b'V_VP9')
                 + ebml('e0', ebml('b0', b'\x40') + ebml('ba', b'\x40')))
    cluster = ebml('1f43b675', ebml('e7', b'\0') + ebml('a3', b'\x81\0\0\x80' + frame(8, '420')))
    files['unicode-osd.webm'] = ebml('1a45dfa3', ebml('4282', b'webm')) + ebml('18538067',
        ebml('1549a966', ebml('2ad7b1', (1_000_000).to_bytes(3, 'big')))
        + ebml('1654ae6b', track) + ebml('1254c367', tags) + cluster)
    for name, data in files.items():
        (OUT / name).write_bytes(data)
    (OUT / 'manifest.json').write_text(json.dumps({
        'provenance': 'Authored raw 2x2 frames and authored neutral VP9 picture with UTF-8 text tags; malformed text timelines are sidecars. No private inputs, external codec or network.',
        'files': {name: hashlib.sha256(data).hexdigest() for name, data in sorted(files.items())},
    }, indent=2) + '\n')


if __name__ == '__main__':
    main()
