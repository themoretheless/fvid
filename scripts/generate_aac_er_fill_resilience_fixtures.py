"""Own HCR/RVLC packets with valid fill and a specifically invalid fill nibble.

Re-muxes previously authored synthetic entropy payloads; never private media.
"""
import json
from pathlib import Path
from generate_he_aac_packet_fixtures import packed, video_fixture

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / 'tests/fixtures/playback-errors'
OUT = SOURCE / 'aac-er-fill-resilience'


def generate():
    files = {}
    cases = []
    for tool in ['hcr', 'rvlc']:
        manifest = json.loads((SOURCE / f'aac-{tool}.json').read_text())
        blob = (SOURCE / f'aac-{tool}-packets.bin').read_bytes()
        for original in manifest['cases']:
            case = dict(original)
            name = tool + '-' + case['name']
            for variant in ['fill', 'invalid-nibble']:
                modified = bytearray()
                rows = []
                for row in original['frames']:
                    raw = blob[row['offset']:row['offset'] + row['bytes']]
                    bits = ''.join(f'{v:08b}' for v in raw)
                    end = len(bits) - row['padding']
                    # ER extension_payload begins immediately after spectral
                    # data. 0 is EXT_FILL; EXT_FILL_DATA requires a zero nibble.
                    packet = packed(bits[:end] + ('00000000' if variant == 'fill' else '00011111'))
                    rows.append(dict(offset=len(modified), bytes=len(packet)))
                    modified.extend(packet)
                derived = dict(case, frames=rows)
                filename = name + '-' + variant + '.mp4'
                files[filename] = video_fixture([derived], modified, case['channels'], filename=filename, write=False)['data']
            cases.append(dict(name=name, tool=tool, control=case['video']['file'],
                              fill=name + '-fill.mp4', malformed=name + '-invalid-nibble.mp4',
                              error='invalid AAC fill-data nibble'))
    files['manifest.json'] = (json.dumps(dict(cases=cases, provenance='Own ER HCR/RVLC entropy payloads and own AVC/MP4 muxer; valid EXT_FILL versus invalid EXT_FILL_DATA nibble. No private media, external decoder, process or network.'), indent=2) + '\n').encode()
    return files


if __name__ == '__main__':
    OUT.mkdir(exist_ok=True)
    for name, data in generate().items():
        (OUT / name).write_bytes(data)
