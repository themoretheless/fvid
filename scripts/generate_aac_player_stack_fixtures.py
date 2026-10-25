"""Own short ER-LC/PS video for the normal-worker decoder-factory stack failure.

Reuses only previously authored synthetic packet/container building blocks.
Generation is separate from tests and requires no external codec or network.
"""
import json
from pathlib import Path
from generate_he_aac_packet_fixtures import video_fixture
from generate_aac_er_pce_fixtures import matroska_audio_fixture

ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / 'tests/fixtures/playback-errors'
OUT = FIXTURES / 'aac-player-factory-stack'


def generate():
    manifest = json.loads((FIXTURES / 'aac-er-pce.json').read_text())
    case = next(c for c in manifest['cases'] if c['name'] == 'ps-aot17-960')
    blob = (FIXTURES / 'aac-er-pce-packets.bin').read_bytes()
    packets = [blob[r['offset']:r['offset'] + r['bytes']] for r in case['frames']]
    mp4 = video_fixture([case], blob, channels=2, filename='ps-er-960.mp4', write=False)['data']
    mkv = matroska_audio_fixture(bytes.fromhex(case['asc']), packets, 48000, 1920, 2)
    metadata = dict(provenance='Own tagged ER-LC/PS packet writer and AVC/MP4/Matroska muxers from the authored AAC ER PCE generator. No private media or codec parameter sets.',
                    failure='Player decoder construction exhausts a normal 2 MiB worker stack while the generic factory retains temporaries for unrelated codecs.',
                    sample_rate=48000, channels=2, sample_frames=4 * 1920,
                    video='ps-er-960.mp4', matroska='ps-er-960.mka')
    return {'ps-er-960.mp4': mp4, 'ps-er-960.mka': mkv,
            'manifest.json': (json.dumps(metadata, indent=2) + '\n').encode()}


if __name__ == '__main__':
    OUT.mkdir(exist_ok=True)
    for name, data in generate().items():
        (OUT / name).write_bytes(data)
