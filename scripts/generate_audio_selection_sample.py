#!/usr/bin/env python3
"""Mux committed synthetic AAC/ALAC packets with owned fixture-only framing.

No encoder, decoder, external muxer or FFmpeg is invoked. Deliberate AAC duration
rounding and priming edits reproduce the original presentation-window regression.
"""
from pathlib import Path
import struct
from avc_fixture_mp4 import elements, vint
from hevc_fixture_mp4 import box, ints, table, header

ROOT = Path(__file__).resolve().parents[1]
OUTPUT = ROOT / 'tests/fixtures/playback-errors'


def boxes(data):
    at = 0
    while at < len(data):
        size, kind = struct.unpack_from('>I4s', data, at)
        if size < 8 or at + size > len(data):
            raise ValueError('invalid fixture atom')
        yield kind, data[at + 8:at + size]
        at += size


def mp4_audio(path):
    data = path.read_bytes()
    movie = dict(boxes(data))[b'moov']
    tracks = [body for kind, body in boxes(movie) if kind == b'trak']
    if len(tracks) != 1:
        raise ValueError('ALAC source must have one synthetic track')
    media = dict(boxes(dict(boxes(tracks[0]))[b'mdia']))
    rate, duration = struct.unpack_from('>II', media[b'mdhd'], 12)
    st = dict(boxes(dict(boxes(media[b'minf']))[b'stbl']))
    default, count = struct.unpack_from('>II', st[b'stsz'], 4)
    sizes = [default] * count if default else list(struct.unpack_from('>' + 'I' * count, st[b'stsz'], 12))
    offset_kind = b'stco' if b'stco' in st else b'co64'
    offset_count = struct.unpack_from('>I', st[offset_kind], 4)[0]
    offsets = struct.unpack_from('>' + ('I' if offset_kind == b'stco' else 'Q') * offset_count, st[offset_kind], 8)
    entries = struct.unpack_from('>I', st[b'stsc'], 4)[0]
    chunks = [struct.unpack_from('>III', st[b'stsc'], 8 + 12 * i) for i in range(entries)]
    packets, cursor = [], 0
    for number, offset in enumerate(offsets, 1):
        mapping = next(row for row in reversed(chunks) if row[0] <= number)
        if mapping[2] != 1:
            raise ValueError('fixture requires one sample description')
        for _ in range(mapping[1]):
            size = sizes[cursor]
            if offset + size > len(data):
                raise ValueError('fixture sample exceeds file')
            packets.append(data[offset:offset + size])
            offset += size
            cursor += 1
    durations = []
    for i in range(struct.unpack_from('>I', st[b'stts'], 4)[0]):
        n, step = struct.unpack_from('>II', st[b'stts'], 8 + 8 * i)
        durations.extend([step] * n)
    if cursor != count or len(durations) != count or sum(durations) != duration:
        raise ValueError('inconsistent fixture sample tables')
    return st[b'stsd'], rate, packets, durations


def aac_entry(rate, channels, asc):
    def descriptor(kind, payload):
        if len(payload) >= 128:
            raise ValueError('fixture descriptor too long')
        return bytes([kind, len(payload)]) + payload
    decoder = descriptor(4, bytes([0x40, 0x15]) + bytes(11) + descriptor(5, asc))
    esds = bytes(4) + descriptor(3, b'\x00\x01\x00' + decoder + descriptor(6, b'\x02'))
    entry = bytearray(28)
    struct.pack_into('>H', entry, 6, 1)
    struct.pack_into('>HH', entry, 16, channels, 16)
    struct.pack_into('>I', entry, 24, rate << 16)
    return ints(0, 1) + box(b'mp4a', bytes(entry) + box(b'esds', esds))


def mono_adts():
    data = (ROOT / 'tests/fixtures/audio/aac-mono-44k.aac').read_bytes()
    packets, at = [], 0
    while at < len(data):
        h = data[at:at + 7]
        if len(h) != 7 or h[0] != 255 or h[1] & 0xf6 != 0xf0 or h[6] & 3:
            raise ValueError('requires complete single-block ADTS')
        size = ((h[3] & 3) << 11) | (h[4] << 3) | (h[5] >> 5)
        if size < 7 or at + size > len(data) or not h[1] & 1:
            raise ValueError('requires unprotected ADTS fixture')
        if (h[2] >> 2) & 15 != 4 or ((h[2] & 1) << 2 | h[3] >> 6) != 1:
            raise ValueError('requires 44.1 kHz mono')
        packets.append(data[at + 7:at + size])
        at += size
    return aac_entry(44100, 1, bytes.fromhex('1208')), 44100, packets, [1024] * len(packets)


def stereo_matroska():
    data = (ROOT / 'tests/fixtures/audio/aac-stereo.mka').read_bytes()
    segment = next(body for kind, body in elements(data) if kind == 0x18538067)
    packets, asc, number = [], None, None
    for kind, body in elements(segment):
        if kind == 0x1654ae6b:
            tracks = [value for key, value in elements(body) if key == 0xae]
            if len(tracks) != 1:
                raise ValueError('requires one synthetic AAC track')
            fields = dict(elements(tracks[0]))
            if fields[0x86] != b'A_AAC':
                raise ValueError('requires AAC')
            number = int.from_bytes(fields[0xd7], 'big')
            asc = fields[0x63a2]
        if kind == 0x1f43b675:
            for key, value in elements(body):
                if key == 0xa0:
                    value = dict(elements(value))[0xa1]
                elif key != 0xa3:
                    continue
                track, pos = vint(value, 0)
                if track != number or len(value) < pos + 3 or value[pos + 2] & 6:
                    raise ValueError('requires complete unlaced fixture blocks')
                packets.append(value[pos + 3:])
    if len(packets) != 48 or asc is None or asc[:2] != bytes.fromhex('1190'):
        raise ValueError('requires 48 packets of 48 kHz stereo AAC-LC')
    # Preserve the regression's deliberately shortened interior/final packets.
    durations = [1024] * 48
    durations[1], durations[-1] = 1016, 912
    return aac_entry(48000, 2, asc), 48000, packets, durations


def mux(tracks, edits):
    scale = 7056000  # Common exact clock for 44.1/48 kHz and millisecond edits.
    ftyp = box(b'ftyp', b'M4A \0\0\0\0M4A isommp42')
    payload, children, movie_duration = b'', [], 0
    for index, ((stsd, rate, packets, durations), edit) in enumerate(zip(tracks, edits), 1):
        offset = len(ftyp) + 8 + len(payload)
        payload += b''.join(packets)
        runs = []
        for step in durations:
            if runs and runs[-1][1] == step:
                runs[-1] = (runs[-1][0] + 1, step)
            else:
                runs.append((1, step))
        stbl = box(b'stsd', stsd) + table(b'stts', len(runs), b''.join(ints(*run) for run in runs))
        stbl += table(b'stsc', 1, ints(1, len(packets), 1))
        stbl += box(b'stsz', ints(0, 0, len(packets)) + ints(*(len(p) for p in packets)))
        stbl += table(b'stco', 1, ints(offset))
        minf = box(b'smhd', bytes(8)) + box(b'dinf', table(b'dref', 1, box(b'url ', ints(1)))) + box(b'stbl', stbl)
        mdhd = header(24, {12: rate, 16: sum(durations)})
        mdia = box(b'mdhd', mdhd) + box(b'hdlr', ints(0, 0) + b'soun' + bytes(12) + b'FVid synthetic\0') + box(b'minf', minf)
        span = sum(durations) * scale // rate if edit is None else edit[0] * scale // rate
        tkhd = bytearray(header(84, {0: 7, 12: index, 20: span}, 40))
        struct.pack_into('>H', tkhd, 36, 256)
        trak = box(b'tkhd', bytes(tkhd)) + box(b'mdia', mdia)
        if edit is not None:
            trak += box(b'edts', table(b'elst', 1, ints(span, edit[1], 65536)))
        children.append(box(b'trak', trak))
        movie_duration = max(movie_duration, span)
    mvhd = header(100, {12: scale, 16: movie_duration, 20: 65536, 24: 0x01000000, 96: len(tracks) + 1}, 36)
    return ftyp + box(b'mdat', payload) + box(b'moov', box(b'mvhd', mvhd) + b''.join(children))


def main():
    alac = mp4_audio(ROOT / 'tests/fixtures/alac/stereo-24.m4a')
    mono, stereo = mono_adts(), stereo_matroska()
    short_stereo = (stereo[0], stereo[1], stereo[2][:9], [1024] * 8 + [32])
    fixtures = {
        'alac-two-tracks.m4a': mux([alac, alac], [None, None]),
        'aac-rounded-two-tracks.m4a': mux([mono, stereo], [(7168, 0), (48008, 1008)]),
        'aac-two-tracks.m4a': mux([mono, short_stereo], [(7168, 0), (7200, 1024)]),
        'aac-no-edit.m4a': mux([stereo], [None]),
    }
    for name, data in fixtures.items():
        (OUTPUT / name).write_bytes(data)
        print(name, len(data))


if __name__ == '__main__':
    main()
