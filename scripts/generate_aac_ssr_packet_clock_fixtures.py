"""Own SSR seek reproducers with variable, quantized and nanosecond clocks."""
import json
from pathlib import Path
from generate_he_aac_packet_fixtures import video_fixture
from generate_aac_er_pce_fixtures import matroska_audio_fixture
ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / 'tests/fixtures/playback-errors'
OUT = SOURCE / 'aac-ssr-packet-clock'


def generate():
    ordinary = json.loads((SOURCE / 'aac-ssr.json').read_text())
    coupling = json.loads((SOURCE / 'aac-ssr-coupling.json').read_text())
    alignment = json.loads((SOURCE / 'aac-ssr-alignment.json').read_text())
    selected = [('mono', 'aac-ssr', ordinary['cases'][0]),
                ('stereo', 'aac-ssr', ordinary['cases'][4]),
                ('coupled', 'aac-ssr-coupling', coupling['cases'][-1]),
                ('source-behind', 'aac-ssr-alignment', next(c for c in alignment['cases'] if c['name'] == 'source-behind')),
                ('late-source', 'aac-ssr-alignment', next(c for c in alignment['cases'] if c['name'] == 'late-source-behind')),
                ('independent-pair', 'aac-ssr-alignment', alignment['channels_cases'][0])]
    files = {}
    cases = []
    for name, owner, original in selected:
        blob = (SOURCE / (owner + '-packets.bin')).read_bytes()
        packets = [blob[r['offset']:r['offset'] + r['bytes']] for r in original['frames']]
        count = len(packets)
        native = name + '-variable.mp4'
        files[native] = video_fixture([original], blob, original['channels'], filename=native, write=False)['data']
        points = [i * 1024 * 1000 // 24000 for i in range(count + 1)]
        durations = [b - a for a, b in zip(points, points[1:])]
        coarse_case = dict(original, sample_entry_rate=24000, container_rate=1000, container_frame_samples=max(durations), durations=durations)
        coarse = name + '-coarse.mp4'
        files[coarse] = video_fixture([coarse_case], blob, original['channels'], filename=coarse, write=False)['data']
        mkv = name + '-fixed.mka'
        files[mkv] = matroska_audio_fixture(bytes.fromhex(original['asc']), packets, 24000, 1024, original['channels'])
        cases.append(dict(name=name, control=original['video']['file'], variable=native, coarse=coarse, matroska=mkv, channels=original['channels'], rate=24000, samples=count * 1024))
    files['manifest.json'] = (json.dumps(dict(cases=cases, provenance='Own SSR gain/window/coupling payload writers and own AVC/MP4/Matroska muxers; variable PQF windows, quantized 1 kHz MP4 and nanosecond Matroska. No private media, external codec, process or network.'), indent=2) + '\n').encode()
    return files


if __name__ == '__main__':
    OUT.mkdir(exist_ok=True)
    for name, data in generate().items():
        (OUT / name).write_bytes(data)
