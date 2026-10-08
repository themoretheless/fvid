#!/usr/bin/env python3
"""Original indexed AAC-LC 7.1 Top + direct cosine PCM, no external codec.

Run explicitly; ordinary tests only read checked-in artifacts. The element
sequence is FC, FL/FR, BL/BR, LFE, TFL/TFR (MPEG channelConfiguration 14).
"""
from pathlib import Path
import hashlib, json, math, re, struct

root = Path(__file__).resolve().parents[1]
folder = root / 'tests/fixtures/playback-errors'
tables = (root / 'crates/fvid-media/src/owned_aac/aac_huffman_tables.rs').read_text()
def table(name):
    body = re.search(r'const ' + name + r':[^=]+ = \[(.*?)\];', tables, re.S)[1]
    return [int(n, 0) for n in re.findall(r'0x[0-9a-fA-F]+|\d+', body)]
codes, lengths = table('SPECTRUM_CODEBOOK1_CODES'), table('SPECTRUM_CODEBOOK1_LENS')
sc, sl = table('SCF_CODEBOOK_CODES'), table('SCF_CODEBOOK_LENS')
def pack(fields):
    bits = ''.join(f'{v:0{w}b}' for v, w in fields)
    bits += '0' * (-len(bits) % 8)
    return int(bits, 2).to_bytes(len(bits) // 8, 'big')
def element(tag, data):
    width = next(w for w in range(1, 9) if len(data) < (1 << (7*w)) - 1)
    return tag.to_bytes((tag.bit_length()+7)//8, 'big') + ((1 << (7*width)) | len(data)).to_bytes(width, 'big') + data
def number(tag, value):
    return element(tag, value.to_bytes(max(1, (value.bit_length()+7)//8), 'big'))
def crc(data):
    value = 255
    for byte in data:
        for shift in range(7, -1, -1):
            feedback = ((value >> 7) ^ (byte >> shift)) & 1
            value = ((value << 1) & 255) ^ (7 if feedback else 0)
    return value
manifest = {'cases': {}, 'element_order': ['FC', 'FL', 'FR', 'BL', 'BR', 'LFE', 'TFL', 'TFR'],
            'pcm_order': ['FL', 'FR', 'FC', 'LFE', 'BL', 'BR', 'TFL', 'TFR'], 'mask': 0x503f}
for rate, index in [(48000, 3), (44100, 4)]:
    for size in [1024, 960]:
        for common in [False, True]:
            name = f'{rate}-{size}-' + ('common' if common else 'separate')
            asc = pack([(2, 5), (index, 4), (14, 4), (int(size == 960), 1), (0, 1), (0, 1)])
            packets, pcm = [], bytearray()
            overlap = [[0.0]*size for _ in range(8)]
            for frame in range(6):
                fields, channel = [], 0
                for kind, tag in [(0, 0), (1, 0), (1, 1), (3, 0), (1, 2)]:
                    fields += [(kind, 3), (tag, 4)]
                    shared = kind == 1 and common
                    if kind == 1:
                        fields += [(int(shared), 1)]
                    if shared:
                        fields += [(0, 1), (0, 2), (0, 1), (1, 6), (0, 1), (0, 2)]
                    for _ in range(2 if kind == 1 else 1):
                        fields += [(140+4*channel, 8)]
                        if not shared:
                            fields += [(0, 1), (0, 2), (0, 1), (1, 6), (0, 1)]
                        fields += [(1, 4), (1, 5), (sc[60], sl[60]), (0, 1), (0, 1), (0, 1)]
                        symbol = 80 if (frame+channel) % 2 == 0 else 0
                        fields += [(codes[symbol], lengths[symbol])]
                        channel += 1
                assert channel == 8
                packets.append(pack(fields + [(7, 3)]))
                signals = []
                for channel in range(8):
                    amplitude = 1024.0 * 2**channel * (1 if (frame+channel)%2 == 0 else -1)
                    block = [sum(amplitude*math.cos(math.pi/size*(t+0.5+size/2)*(k+0.5)) for k in range(4))*2/size/65536*math.sin(math.pi/(2*size)*(t+0.5)) for t in range(2*size)]
                    signals.append([block[i]+overlap[channel][i] for i in range(size)])
                    overlap[channel] = block[size:]
                for i in range(size):
                    for channel in [1, 2, 0, 5, 3, 4, 6, 7]:
                        pcm += struct.pack('<f', signals[channel][i])
            # One cluster per packet, nanosecond timebase; floor positions ensure
            # the timeline uses exact integral sample boundaries at both rates.
            header = element(0x1a45dfa3, element(0x4282, b'matroska') + number(0x4287, 4) + number(0x4285, 2))
            info = element(0x1549a966, number(0x2ad7b1, 1))
            audio = element(0xe1, element(0xb5, struct.pack('>d', rate)) + number(0x9f, 8))
            track = element(0xae, number(0xd7, 1) + number(0x73c5, 1) + number(0x83, 2) + element(0x86, b'A_AAC') + element(0x63a2, asc) + audio)
            clusters = b''.join(element(0x1f43b675, number(0xe7, frame*size*1_000_000_000//rate) + element(0xa3, b'\x81\x00\x00\x80' + packet)) for frame, packet in enumerate(packets))
            mka = header + element(0x18538067, info + element(0x1654ae6b, track) + clusters)
            # Same original packets with an independently authored explicit PCE.
            # Front FC/CPE0/CPE2, back CPE1, LFE0. Only front CPE2 is top.
            fields = [(2, 5), (index, 4), (0, 4), (int(size == 960), 1), (0, 1), (0, 1),
                      (0, 4), (1, 2), (index, 4), (3, 4), (0, 4), (1, 4), (1, 2),
                      (0, 3), (0, 4), (0, 1), (0, 1), (0, 1),
                      (0, 1), (0, 4), (1, 1), (0, 4), (1, 1), (2, 4),
                      (1, 1), (1, 4), (0, 4)]
            used = sum(w for _, w in fields)
            if used % 8: fields += [(0, 8-used%8)]
            comment = bytes([0xac, 0x04])
            comment += bytes([crc(comment)])
            fields += [(3, 8)] + [(byte, 8) for byte in comment]
            explicit = pack(fields)
            baseline_track = element(0xae, number(0xd7, 1) + number(0x73c5, 1) + number(0x83, 2) + element(0x86, b'A_AAC') + element(0x63a2, explicit) + audio)
            baseline = header + element(0x18538067, info + element(0x1654ae6b, baseline_track) + clusters)
            video = f'YUV4MPEG2 W16 H16 F{rate}:{size} Ip A1:1 C420jpeg\n'.encode()
            for frame in range(6):
                video += b'FRAME\n' + bytes(16+(x*3+y*5+frame*11)%200 for y in range(16) for x in range(16)) + bytes([128])*128
            artifacts = {}
            for suffix, data in [('.mka', mka), ('.baseline.mka', baseline), ('.f32le', pcm), ('.y4m', video), ('.asc', asc)]:
                path = folder / ('aac-top-config-' + name + suffix)
                path.write_bytes(data)
                artifacts[suffix] = {'file': path.name, 'sha256': hashlib.sha256(data).hexdigest()}
            manifest['cases'][name] = {'rate': rate, 'samples': size, 'common_window': common, 'artifacts': artifacts}
(folder / 'aac-top-config-generated.json').write_text(json.dumps(manifest, indent=2)+'\n')
