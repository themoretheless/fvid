"""Owned muxer for single-layer, no-reordering HM fixture streams (not general muxing)."""
import re
import struct


def box(kind, body):
    return struct.pack('>I4s', len(body) + 8, kind) + body


def ints(*values):
    return struct.pack('>' + 'I' * len(values), *values)


def table(kind, count, entries):
    return box(kind, ints(0, count) + entries)


def header(size, values, matrix_offset=None):
    data = bytearray(size)
    for offset, value in values.items():
        data[offset:offset + 4] = ints(value)
    if matrix_offset is not None:
        for offset, value in ((0, 65536), (16, 65536), (32, 1 << 30)):
            data[matrix_offset + offset:matrix_offset + offset + 4] = ints(value)
    return bytes(data)


def mux(stream, depth, width=64, height=64, rate=25):
    units = [n for n in re.split(b'\x00\x00\x00?\x01', stream) if n]
    parameters = {kind: [] for kind in (32, 33, 34)}
    samples, current, sync = [], [], []
    for nal in units:
        kind = (nal[0] >> 1) & 63
        if kind in parameters:
            if nal not in parameters[kind]:
                parameters[kind].append(nal)
            continue
        if kind <= 31:
            if len(nal) < 3:
                raise ValueError('truncated slice')
            if nal[2] & 128:
                if current:
                    samples.append(b''.join(current))
                    current = []
                if 16 <= kind <= 23:
                    sync.append(len(samples) + 1)
            elif not current:
                raise ValueError('missing first slice')
            current.append(ints(len(nal)) + nal)
        elif kind not in (35, 36, 37, 38, 39, 40):
            raise ValueError(f'unsupported NAL {kind}')
    if current:
        samples.append(b''.join(current))
    if not samples or any(len(parameters[k]) != 1 for k in parameters):
        raise ValueError('requires one VPS/SPS/PPS and nonempty pictures')
    # SPS begins with one byte of layer/nesting information, then the 12-byte PTL.
    sps = parameters[33][0][2:]
    sps = re.sub(b'\x00\x00\x03', b'\x00\x00', sps)
    if (sps[0] >> 1) & 7:
        raise ValueError('fixture muxer requires one temporal layer')
    hvcc = bytes([1]) + sps[1:13] + bytes.fromhex('f000fcfd') + bytes([0xf8 | (depth - 8)] * 2) + b'\x00\x00' + bytes([0x0f, 3])
    for kind, nals in parameters.items():
        hvcc += bytes([0x80 | kind]) + struct.pack('>H', len(nals))
        for nal in nals:
            hvcc += struct.pack('>H', len(nal)) + nal
    entry = bytearray(78)
    entry[6:8] = struct.pack('>H', 1)
    entry[24:28] = struct.pack('>HH', width, height)
    entry[28:36] = ints(72 << 16, 72 << 16)
    entry[40:42] = struct.pack('>H', 1)
    entry[74:78] = struct.pack('>HH', 24, 65535)
    ftyp = box(b'ftyp', b'isom\x00\x00\x00\x00isomiso6hvc1')
    count = len(samples)
    stbl = table(b'stsd', 1, box(b'hvc1', bytes(entry) + box(b'hvcC', hvcc)))
    stbl += table(b'stts', 1, ints(count, 1))
    stbl += table(b'stsc', 1, ints(1, count, 1))
    stbl += box(b'stsz', ints(0, 0, count) + ints(*(len(s) for s in samples)))
    stbl += table(b'stco', 1, ints(len(ftyp) + 8))
    stbl += table(b'stss', len(sync), ints(*sync))
    dref = table(b'dref', 1, box(b'url ', ints(1)))
    minf = box(b'vmhd', ints(1) + bytes(8)) + box(b'dinf', dref) + box(b'stbl', stbl)
    mdhd = header(24, {12: rate, 16: count})
    hdlr = ints(0, 0) + b'vide' + bytes(12) + b'FVid synthetic\x00'
    mdia = box(b'mdhd', mdhd) + box(b'hdlr', hdlr) + box(b'minf', minf)
    tkhd = header(84, {0: 7, 12: 1, 20: count, 76: width << 16, 80: height << 16}, 40)
    mvhd = header(100, {12: rate, 16: count, 20: 65536, 24: 0x01000000, 96: 2}, 36)
    moov = box(b'mvhd', mvhd) + box(b'trak', box(b'tkhd', tkhd) + box(b'mdia', mdia))
    return ftyp + box(b'mdat', b''.join(samples)) + box(b'moov', moov)
