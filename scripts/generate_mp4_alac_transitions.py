"""Re-encode owned analytic PCM as ALAC raw elements; no external codec."""
import struct
from generate_mp4_fragment_description_regressions import atoms
from generate_vp9_chroma import atom, word


def convert(data, compressed=False):
    top = dict(atoms(data))
    audio = next(b for k, b in atoms(top[b'moov']) if k == b'trak' and b'soun' in b)
    mdia = dict(atoms(audio))[b'mdia']
    stbl = dict(atoms(dict(atoms(mdia))[b'minf']))[b'stbl']
    tables = dict(atoms(stbl))
    offsets = struct.unpack('>6I', tables[b'stco'][8:])
    sizes = struct.unpack('>6I', tables[b'stsz'][12:])
    packets = []
    entries = []
    for rate, channels, depth in [(48000, 2, 16), (32000, 1, 16), (44100, 2, 24)]:
        entry = bytearray(28)
        entry[6:8] = struct.pack('>H', 1)
        entry[16:20] = struct.pack('>HH', channels, depth)
        entry[24:28] = word(rate << 16)
        cookie = bytearray(24)
        cookie[:4] = word(rate // 50)
        cookie[5:10] = bytes([depth, 40, 10, 14, channels])
        cookie[20:24] = word(rate)
        entries.append(atom(b'alac', entry + atom(b'alac', word(0) + cookie)))
    for i, (offset, size) in enumerate(zip(offsets, sizes)):
        raw = data[offset:offset + size]
        channels, depth = [(2, 16), (1, 16), (2, 24)][i % 3]
        if i % 3 == 1:
            values = [int(v * 32768) for (v,) in struct.iter_unpack('<f', raw)]
        else:
            stride = depth // 8
            values = [int.from_bytes(raw[j:j + stride], 'little', signed=True) for j in range(0, len(raw), stride)]
        count = len(values) // channels
        bits = ('001' if channels == 2 else '000') + '0000' + '0' * 12 + ('1000' if compressed else '1001') + f'{count:032b}'
        if compressed:
            from generate_alac_multichannel_fixtures import rice_plane
            bits += '0' * 16  # no stereo matrix
            bits += ('1111' + '0001' + '100' + '00000') * channels
            stats = dict(ordinary=0, escapes=0, runs=0, run_samples=0, resumed_run_samples=0)
            for channel in range(channels):
                plane = values[channel::channels]
                residual = [plane[0]] + [b - a for a, b in zip(plane, plane[1:])]
                bits += rice_plane(residual, depth + channels - 1, stats)
        else:
            bits += ''.join(f'{v & ((1 << depth) - 1):0{depth}b}' for v in values)
        bits += '111'
        bits += '0' * (-len(bits) % 8)
        packets.append(int(bits, 2).to_bytes(len(bits) // 8, 'big'))
    def rewrite(body, selected=False):
        out = b''
        for k, b in atoms(body):
            if k == b'trak':
                b = rewrite(b, b'soun' in b)
            elif k in [b'moov', b'mdia', b'minf', b'stbl']:
                b = rewrite(b, selected)
            elif selected and k == b'stsd':
                b = word(0, 3) + b''.join(entries)
            elif selected and k == b'stsz':
                b = word(0, 0, 6, *map(len, packets))
            elif selected and k == b'stco':
                current = offsets[0]
                new = []
                for p in packets:
                    new.append(current)
                    current += len(p)
                b = word(0, 6, *new)
            out += atom(k, b)
        return out
    header = atom(b'ftyp', top[b'ftyp'])
    video_bytes = offsets[0] - len(header) - 8
    return header + atom(b'mdat', top[b'mdat'][:video_bytes] + b''.join(packets)) + atom(b'moov', rewrite(top[b'moov']))
