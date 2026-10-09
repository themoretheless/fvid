#!/usr/bin/env python3
"""Own independent CCE/SBR videos and direct-cosine CCE core PCM. Offline."""
import hashlib
import json
import struct
from generate_he_aac_dependent_coupling_fixtures import (
    DEST, field, frequency, packed, info, target, word, codes, lens,
    mono_payload, pair_payload, tables, core, video_fixture,
)


def program(prefix, stereo, tags):
    bits = (prefix + field(0, 4) + field(1, 2) + frequency(24000)
            + field(1, 4) + field(0, 4) + field(0, 4) + field(0, 2)
            + field(0, 3) + field(len(tags), 4) + '000'
            + field(stereo, 1) + field(0, 4))
    bits += ''.join('1' + field(tag, 4) for tag in tags)
    return bits + '0' * (-len(bits) % 8) + field(0, 8)


def config(slots, rate, kind, stereo, tags):
    ga = field(slots == 15, 1) + '00'
    prefix = (field(5, 5) + frequency(24000) + '0000' + frequency(rate)
              + field(2, 5) + ga) if kind == 'explicit' else (
                  field(2, 5) + frequency(24000) + '0000' + ga)
    bits = program(prefix, stereo, tags)
    if kind == 'sync':
        bits += field(0x2b7, 11) + field(5, 5) + '1' + frequency(rate)
    return packed(bits)


def fill(raw):
    if not raw:
        return ''
    size = len(raw)
    return ('110' + (field(size, 4) if size < 15 else '1111' + field(size - 14, 8))
            + ''.join(field(b, 8) for b in raw))


def packet(frame, stereo, tags, target_raw, cce_rows, missing=False):
    bits = program('101', stereo, tags)
    bits += ('00100001' + info() + '00' + target(True) * 2) if stereo else (
        '0000000' + target(False))
    bits += fill(target_raw)
    # Alternate wire order: histories belong to instance tags, not parse order.
    for row in cce_rows[::(-1 if frame % 2 else 1)]:
        tag = row['tag']
        bits += ('010' + field(tag, 4) + '1' + field(0, 3) + field(stereo, 1)
                 + field(missing and tag == tags[-1], 4)
                 + (field(3, 2) if stereo else '') + '0000')
        bits += field(140, 8) + info() + field(1, 4) + field(1, 5) + word(60) + '000'
        index = 80 if (frame + row['phase']) % 2 == 0 else 0
        bits += field(codes[index], lens[index])
        if stereo:
            # Independent gain lists have no common-gain flag; first gain is 1.
            bits += word(64 if tag == tags[0] else 68)
        bits += fill(bytes.fromhex(row['sbr']))
    return packed(bits + '111')


def main():
    blob, pcm, cases = bytearray(), bytearray(), []
    nhigh = len(tables(10, 27, 0, False, 0, 0)[1]) - 1
    core_offsets = {}
    for slots in [15, 16]:
        values = core(slots * 64, False, 1)
        for phase in [0, 1]:
            data = values if phase == 0 else b''.join(
                struct.pack('<f', -struct.unpack('<f', values[i:i + 4])[0])
                for i in range(0, len(values), 4))
            core_offsets[slots, phase] = [len(pcm), len(data) // 4]
            pcm.extend(data)
        for stereo in [False, True]:
            for tags in [[1], [15], [1, 15]]:
                for target_fill in [False, True]:
                    for cce_fill in [False, True]:
                        frames = []
                        for frame in range(6):
                            raw = (pair_payload(nhigh, 2, True, frame, False) if stereo
                                   else mono_payload(nhigh, 2, True, frame)) if target_fill else b''
                            rows = []
                            for i, tag in enumerate(tags):
                                # Missing whole FIL in the middle retains syntax and QMF history.
                                cce_raw = mono_payload(nhigh, 2, True, frame) if cce_fill and frame != 2 else b''
                                rows.append(dict(tag=tag, phase=i, sbr=cce_raw.hex(),
                                                 right_gain=2 ** (-.5 if i == 0 else -1.),
                                                 core_pcm=core_offsets[slots, i]))
                            data = packet(frame, stereo, tags, raw, rows)
                            frames.append(dict(offset=len(blob), bytes=len(data), sbr=raw.hex(), cce=rows))
                            blob.extend(data)
                        for rate, bands in [(24000, 32), (48000, 64)]:
                            for kind in ['explicit', 'sync', 'implicit']:
                                if kind == 'implicit' and bands == 32:
                                    continue
                                case = dict(slots=slots, bands=bands, frames=frames,
                                            asc=config(slots, rate, kind, stereo, tags).hex(),
                                            pcm_offset=0, samples=slots * bands * 12,
                                            channels=2 if stereo else 1, tags=tags, kind=kind,
                                            output_rate=rate)
                                case['video'] = video_fixture(
                                    [case], blob, channels=case['channels'],
                                    filename=f'he-aac-independent-sbr-{slots*64}-{int(stereo)}-{len(tags)}-{tags[0]}-{int(target_fill)}-{int(cce_fill)}-{kind}-synthetic.mp4'
                                ) if bands == 64 else None
                                cases.append(case)
    invalid = []
    for failure in ['missing-target', 'crc']:
        tags, frames = [1, 15], []
        for frame in range(3):
            raw = pair_payload(nhigh, 2, True, frame, False)
            rows = []
            for i, tag in enumerate(tags):
                cce_raw = bytearray(mono_payload(nhigh, 2, True, frame))
                if failure == 'crc' and frame == 1 and tag == 1:
                    cce_raw[0] ^= 1  # second wire CCE's protected CRC bit
                rows.append(dict(tag=tag, phase=i, sbr=cce_raw.hex(),
                                 right_gain=2**(-.5 if i == 0 else -1.),
                                 core_pcm=core_offsets[16, i]))
            data = packet(frame, True, tags, raw, rows, missing=failure == 'missing-target' and frame == 1)
            frames.append(dict(offset=len(blob), bytes=len(data), sbr=raw.hex(), cce=rows))
            blob.extend(data)
        case = dict(slots=16, bands=64, frames=frames, asc=config(16, 48000, 'explicit', True, tags).hex(),
                    pcm_offset=0, samples=6144, channels=2, output_rate=48000)
        case['video'] = video_fixture([case], blob, channels=2,
                                     filename=f'he-aac-independent-sbr-{failure}-synthetic.mp4')
        case['error'] = 'AAC coupling target is absent' if failure == 'missing-target' else 'SBR CRC'
        invalid.append(case)
    (DEST / 'he-aac-independent-sbr-packets.bin').write_bytes(blob)
    (DEST / 'he-aac-independent-sbr-core.f32le').write_bytes(pcm)
    (DEST / 'he-aac-independent-sbr-oracles.json').write_text(json.dumps(dict(
        kind='own independent CCE core PCM; per-element SBR stage composition', cases=cases, invalid=invalid,
        packet_sha256=hashlib.sha256(blob).hexdigest(), core_sha256=hashlib.sha256(pcm).hexdigest()), indent=2) + '\n')
    print(len(cases), 'independent CCE cases;', sum(c['video'] is not None for c in cases), 'acceptance videos')


if __name__ == '__main__':
    main()
