"""Own ELD AU-origin/EOF markers, independent full convolution oracle.

No private/ISO media or parameter sets,native/foreign decoder,network or process.
An inactive hypothetical next AU is only a mathematical oracle convenience:
its leading quarter-window is identically zero for every possible spectrum.
"""
import json
import math
from pathlib import Path
import struct
from generate_aac_eld_fixtures import generate, window, mkv
from generate_he_aac_packet_fixtures import video_fixture

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT/'tests/fixtures/playback-errors/aac-eld-au-origin'


def main():
    OUT.mkdir(exist_ok=True)
    authored = generate(True, [(n, 24000, ch, 'base', 0, 0, False)
                              for n in [480, 512] for ch in [1, 2]], frame_count=8)
    models = json.loads(authored['manifest.json'])['cases']
    source = authored['packets.bin']
    files, cases, blob = {}, [], bytearray()
    for model in models:
        n, channels = model['n'], model['channels']
        w = window(n)
        assert len(w) == 4*n and all(v == 0 for v in w[:n//4])
        kernels = []
        for ch in range(channels):
            # Existing own base AU0: eight spectral coefficients in two bands,
            # codebook1,global_gain140,ordinary SF140,no MS/PNS/TNS/resilience.
            spectrum = [1024*((k+ch) % 3-1) for k in range(8)]
            kernels.append([-sum(v*math.cos(math.pi/n*(t+.5-n/2)*(k+.5))
                                 for k, v in enumerate(spectrum))*w[t]/n/32768
                            for t in range(4*n)])
        def packet(index):
            row = model['frames'][index]
            return source[row['offset']:row['offset']+row['bytes']]
        signal, silence = packet(0), packet(7)
        for label, active in [('first', [0]), ('last', [7]), ('both', [0, 7])]:
            packets = [signal if i in active else silence for i in range(8)]
            frames = []
            for wire in packets:
                frames.append(dict(offset=len(blob), bytes=len(wire)))
                blob.extend(wire)
            # Superpose complete finite impulse responses on a global time
            # axis,then select the AU origin. This does not use a production
            # overlap buffer or shift/crop a truncated decoded file.
            pcm = []
            for t in range(n//4, 8*n+n//4):
                for kernel in kernels:
                    pcm.append(sum(kernel[t-i*n] for i in active if 0 <= t-i*n < 4*n))
            name = f'n{n}-c{channels}-{label}'
            case = dict(model, name=name, frames=frames, samples=8*n,
                        video=name+'.mp4', matroska=name+'.mkv', file=name+'.mkv',
                        golden=name+'.f32le', recipe=label, active_aus=active,
                        output_origin=n//4, edges=[n, 7*n], pcm_offset=0)
            if label == 'last':
                assert max(map(abs, pcm[-n//4*channels:])) > 1e-6
            files[case['golden']] = b''.join(struct.pack('<f', v) for v in pcm)
            files[case['video']] = video_fixture([case], blob, channels, write=False)['data']
            files[case['file']] = mkv(bytes.fromhex(case['asc']), packets, 24000, n, channels)
            cases.append(case)
    files['packets.bin'] = bytes(blob)
    files['manifest.json'] = (json.dumps(dict(cases=cases,
        exports=[dict(c, name=c['name']+'-matroska', video=c['file']) for c in cases],
        scope='12 own eight-AU480/512 mono/stereo ELD markers. Isolated first,last and combined activation. Full direct four-block cosine/window convolution at causal AU origin N/4,including a nonzero final quarter. No source video/audio/codec parameter sets,production or foreign decoder.'), indent=2)+'\n').encode()
    for name, data in files.items():
        (OUT/name).write_bytes(data)
    print('12 authored AU-origin programmes,24 movies')


if __name__ == '__main__':
    main()
