"""Owned fixture-only x264 Matroska reader and AVC MP4 muxer with B-frame PTS."""
import struct
from hevc_fixture_mp4 import box, ints, table, header


def vint(data, pos, identifier=False):
    first = data[pos]
    width = next((n for n in range(1, 9) if first & (1 << (8-n))), None)
    if width is None or pos+width > len(data):
        raise ValueError('invalid EBML integer')
    value = int.from_bytes(data[pos:pos+width], 'big')
    if not identifier:
        value &= (1 << (7*width))-1
    return value, pos+width


def elements(data):
    pos = 0
    while pos < len(data):
        kind, pos = vint(data, pos, True)
        size_start = pos
        size, pos = vint(data, pos)
        if size == (1 << (7*(pos-size_start)))-1:
            if kind != 0x18538067:
                raise ValueError("unknown length is supported only for the fixture Segment")
            size = len(data)-pos
        if size > len(data)-pos:
            raise ValueError(f'truncated fixture EBML element: {kind:x}, size={size}, remaining={len(data)-pos}')
        yield kind, data[pos:pos+size]
        pos += size


def read_mkv(data, rate):
    segment = next(body for kind, body in elements(data) if kind == 0x18538067)
    config = None
    scale = 1000000
    track = None
    frames = []
    for kind, body in elements(segment):
        if kind == 0x1549a966:
            for k, value in elements(body):
                if k == 0x2ad7b1:
                    scale = int.from_bytes(value, 'big')
        if kind == 0x1654ae6b:
            entries = [v for k, v in elements(body) if k == 0xae]
            if len(entries) != 1:
                raise ValueError('fixture must have exactly one AVC track')
            fields = dict(elements(entries[0]))
            if fields.get(0x86) != b'V_MPEG4/ISO/AVC':
                raise ValueError('fixture requires AVC')
            track = int.from_bytes(fields[0xd7], 'big')
            config = fields[0x63a2]
        if kind == 0x1f43b675:
            clock = 0
            for k, value in elements(body):
                if k == 0xe7:
                    clock = int.from_bytes(value, 'big')
                elif k == 0xa3:
                    number, pos = vint(value, 0)
                    if number != track or len(value) < pos+3 or value[pos+2] & 6:
                        raise ValueError('unexpected track, truncated or laced block')
                    timestamp = clock + struct.unpack_from('>h', value, pos)[0]
                    numerator = timestamp * scale * rate
                    pts = (numerator + 500000000)//1000000000
                    if abs(numerator-pts*1000000000) > scale*rate:
                        raise ValueError('fixture timestamp is not on frame grid')
                    frames.append((pts, bool(value[pos+2] & 0x80), value[pos+3:]))
                elif k == 0xa0:
                    raise ValueError('fixture reader requires x264 SimpleBlocks')
    if config is None or not frames or sorted(p for p, _, _ in frames) != list(range(len(frames))):
        raise ValueError('fixture requires AVC configuration and consecutive presentation frames')
    return config, frames


def annexb(config, frames):
    if config[0] != 1 or config[4] & 3 != 3:
        raise ValueError('fixture requires four-byte AVC lengths')
    nals, pos = [], 6
    for count in [config[5] & 31]:
        for _ in range(count):
            size = int.from_bytes(config[pos:pos+2], 'big'); pos += 2
            nals.append(config[pos:pos+size]); pos += size
    count = config[pos]; pos += 1
    for _ in range(count):
        size = int.from_bytes(config[pos:pos+2], 'big'); pos += 2
        nals.append(config[pos:pos+size]); pos += size
    for _, _, packet in frames:
        pos = 0
        while pos < len(packet):
            size = int.from_bytes(packet[pos:pos+4], 'big'); pos += 4
            if size == 0 or pos+size > len(packet):
                raise ValueError('invalid AVC packet')
            nals.append(packet[pos:pos+size]); pos += size
    return b''.join(b'\x00\x00\x00\x01'+nal for nal in nals)


def mux(config, frames, width=64, height=64, rate=30, *, inband_parameters=False):
    entry = bytearray(78)
    entry[6:8] = struct.pack('>H', 1)
    entry[24:28] = struct.pack('>HH', width, height)
    entry[28:36] = ints(72 << 16, 72 << 16)
    entry[40:42] = struct.pack('>H', 1)
    entry[74:78] = struct.pack('>HH', 24, 65535)
    codec = b'avc3' if inband_parameters else b'avc1'
    ftyp = box(b'ftyp', b'isom\x00\x00\x00\x00isomiso6' + codec)
    count = len(frames)
    stbl = table(b'stsd', 1, box(codec, bytes(entry)+box(b'avcC', config)))
    stbl += table(b'stts', 1, ints(count, 1))
    stbl += box(b'ctts', ints(1 << 24, count) + b''.join(struct.pack('>Ii', 1, pts-i) for i, (pts, _, _) in enumerate(frames)))
    stbl += table(b'stsc', 1, ints(1, count, 1))
    stbl += box(b'stsz', ints(0, 0, count) + ints(*(len(packet) for _, _, packet in frames)))
    stbl += table(b'stco', 1, ints(len(ftyp)+8))
    sync = [i+1 for i, (_, key, _) in enumerate(frames) if key]
    stbl += table(b'stss', len(sync), ints(*sync))
    dref = table(b'dref', 1, box(b'url ', ints(1)))
    minf = box(b'vmhd', ints(1)+bytes(8)) + box(b'dinf', dref) + box(b'stbl', stbl)
    mdhd = header(24, {12: rate, 16: count})
    hdlr = ints(0, 0)+b'vide'+bytes(12)+b'FVid synthetic\x00'
    mdia = box(b'mdhd', mdhd)+box(b'hdlr', hdlr)+box(b'minf', minf)
    tkhd = header(84, {0: 7, 12: 1, 20: count, 76: width << 16, 80: height << 16}, 40)
    mvhd = header(100, {12: rate, 16: count, 20: 65536, 24: 0x01000000, 96: 2}, 36)
    return ftyp+box(b'mdat', b''.join(p for _, _, p in frames))+box(b'moov', box(b'mvhd', mvhd)+box(b'trak', box(b'tkhd', tkhd)+box(b'mdia', mdia)))
