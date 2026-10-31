"""Own zero-core LD-SBR harmonic/energy acceptance, independently synthesized.

Explicit offline generation. No private/ISO media or codec parameter sets,
production decoder, foreign codec, subprocess or network supplies fixture gold.
"""
import json
import math
from pathlib import Path
import struct
from generate_aac_eld_fixtures import mkv
from generate_aac_eld_cldfb_fixtures import synthesis, window
from generate_aac_sbr_dsp_fixtures import header
from generate_aac_sbr_extension_fixtures import crc
from generate_aac_sbr_frequency_oracles import tables
from generate_aac_sbr_history_fixtures import row
from generate_he_aac_packet_fixtures import field, frequency, packed, video_fixture

ROOT = Path(__file__).resolve().parents[1]
FIX = ROOT/'tests/fixtures/playback-errors'
OUT = FIX/'aac-ld-sbr-silent-harmonics'


def generate():
    _, high, _, _ = tables(10, 27, 0, False, 0, 0)
    centers = {(a+b)//2 for a, b in zip(high, high[1:])}
    noise = [complex(*v) for v in struct.iter_unpack('<dd', (FIX/'aac-sbr-noise-protocol.f64le').read_bytes())]
    c = window()
    files, cases, blob = {}, [], bytearray()
    for n in [480, 512]:
        slots = n//32
        for quantized in [24, 36, 48]:
            energy = 2**(6+quantized/2)
            for noise_level in [7, 30]:
                ratio = 2**(6-noise_level)
                # Zero current HF energy; every fine band carries its center
                # sinusoid. Noise remains only on the other QMF channels.
                denominator = energy*(len(centers)+(17-len(centers))*ratio)/(1+ratio)
                boost = min(math.sqrt(17*energy/denominator), 1.584893192)
                sine = math.sqrt(energy/(1+ratio))*boost
                random = math.sqrt(energy*ratio/(1+ratio))*boost
                rows = []
                for t in range(8*slots):
                    values = [0j]*64
                    for k in range(10, 27):
                        if k in centers:
                            phase = [1, 1j, -1, -1j][t % 4]
                            values[k] = sine*(phase.conjugate() if k % 2 else phase)
                        else:
                            values[k] = random*noise[(t*17+k-10+1) % 512]
                    rows.append(values)
                packets = []
                for index in range(8):
                    temporal = index > 0
                    env = row([0]*(len(high)-1) if temporal else [quantized]+[0]*(len(high)-2),
                              temporal, False, False, False)
                    qn = row([0 if temporal else noise_level], temporal, False, False, True)
                    body = '0'+'0'+'00001'+field(temporal, 1)*2+'00'+env+qn+'1'+'1'*(len(high)-1)+'0'
                    # global_gain121,max_sfb0,tns_present0: all core PCM is zero.
                    packets.append(packed(field(121, 8)+field(0, 6)+'0'+field(crc(body), 10)+body))
                for dual in [False, True]:
                    bands = 64 if dual else 32
                    rate, frame_samples = 24000*(1+dual), n*(1+dual)
                    asc = packed(field(31, 5)+field(7, 6)+frequency(24000)+field(1, 4)
                                 +field(n == 480, 1)+'000'+'1'+field(dual, 1)+'1'
                                 +header(0, True)+'0000'+'00')
                    frames = []
                    for wire in packets:
                        frames.append(dict(offset=len(blob), bytes=len(wire)))
                        blob.extend(wire)
                    name = f'n{n}-e{quantized}-q{noise_level}-x{1+dual}'
                    case = dict(name=name, n=n, slots=slots, bands=bands, channels=1,
                                rate=rate, asc=asc.hex(), frames=frames, samples=8*frame_samples,
                                container_rate=rate, container_frame_samples=frame_samples,
                                pcm_offset=0, envelope_level=quantized, noise_level=noise_level,
                                video=name+'.mp4', file=name+'.mkv', golden=name+'.f32le',
                                edges=[frame_samples, 3*frame_samples])
                    pcm = [v/(32768 if dual else 65536) for v in synthesis(rows, bands, c)]
                    assert len(pcm) == case['samples'] and 0 < max(map(abs, pcm)) < 1
                    files[case['golden']] = b''.join(struct.pack('<f', v) for v in pcm)
                    files[case['video']] = video_fixture([case], blob, 1, write=False)['data']
                    files[case['file']] = mkv(asc, packets, rate, frame_samples, 1)
                    cases.append(case)
    files['packets.bin'] = bytes(blob)
    files['manifest.json'] = (json.dumps(dict(cases=cases,
        exports=[dict(c, name=c['name']+'-matroska', video=c['file']) for c in cases],
        scope='24 authored eight-AU480/512 single/dual-rate CRC LD-SBR programmes. Zero AAC core,fine harmonic centers,noise7/30,envelope24/36/48. Independent direct complex CLDFB PCM and energy formulas. Not an ISO conformance verdict or new native tool implementation.'), indent=2)+'\n').encode()
    return files


if __name__ == '__main__':
    OUT.mkdir(exist_ok=True)
    for name, data in generate().items():
        (OUT/name).write_bytes(data)
    print('24 authored silent-core harmonic programmes,48 movies')
