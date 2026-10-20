"""Own separate-colour-plane PCM streams; no external codec or source media."""
import json
import struct
from pathlib import Path
from generate_avc_pcm_formats import configuration
from generate_avc_mbaff_direct_samples import Writer
from avc_fixture_mp4 import mux
OUT = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors/avc-separate-pcm'

def generate():
    OUT.mkdir(exist_ok=True)
    cases = []
    for depth in (8, 10, 14):
        for reverse in (False, True):
            name = f'separate-pcm-{depth}-{int(reverse)}'
            config = configuration(3, depth, depth, True, separate=True)
            (OUT / (name + '.avcc')).write_bytes(config)
            records = []
            samples = []
            for frame in range(2):
                nals = []
                planes = [[(x * 31 + y * 73 + c * 137 + frame * 19) & ((1 << depth)-1)
                           for y in range(16) for x in range(32)] for c in range(3)]
                for c in range(3):
                    for mb in range(2):
                        b = Writer(); b.ue(mb); b.ue(2); b.ue(0); b.u(c, 2); b.u(frame, 4)
                        if frame == 0: b.ue(0)
                        b.u(frame * 2, 4)
                        if frame == 0: b.u(0); b.u(0)
                        else: b.u(0)
                        b.se(0); b.ue(1); b.ue(25); b.align()
                        for y in range(16):
                            for x in range(16): b.u(planes[c][y * 32 + mb * 16 + x], depth)
                        nal = b.nal(0x65 if frame == 0 else 0x41)
                        nals.append(len(nal).to_bytes(4, 'big') + nal)
                if reverse: nals.reverse()
                packet = b''.join(nals)
                (OUT / f'{name}-{frame}.packet').write_bytes(packet)
                records.append((frame, frame == 0, packet))
                samples.extend(sum(planes, []))
            (OUT / (name + '.mp4')).write_bytes(mux(config, records, 32, 16, 25))
            (OUT / (name + '.y16')).write_bytes(struct.pack('<' + str(len(samples)) + 'H', *samples))
            cases.append(name)
    (OUT / 'manifest.json').write_text(json.dumps({'provenance': __doc__, 'cases': cases}, indent=2)+'\n')
if __name__ == '__main__': generate()
