#!/usr/bin/env python3
"""Own ER AAC-LC (AOT 17), ER AAC-LTP (AOT 19), and ER AAC-LD (AOT 23) explicit
PCE (`channel_configuration == 0`) packets, MP4/Matroska containers, and scalar
PCM references across 480/512/960/1024 frame lengths, custom PCE speaker positions,
resilience flags, SBR extensions, and Parametric Stereo.
No private media, foreign encoder/decoder, FFmpeg, or network access.
"""
import json, math, struct
from generate_aac_main_tools_fixtures import channel, ics, sc, tuple_bits, f32, Filterbank
from generate_aac_ltp_transition_fixtures import Oracle as LtpOracle
from generate_aac_ld_filterbank_fixtures import window as ld_window
from generate_aac_ld_ltp_fixtures import GAINS
from generate_aac_ps_matroska_fixtures import element, number
from generate_he_aac_packet_fixtures import DEST, field, frequency, packed, video_fixture
from generate_aac_er_ps_fixtures import strip_sbr_crc


def matroska_audio_fixture(asc: bytes, packets: list[bytes], sample_rate: int, frame_samples: int, channels: int) -> bytes:
    audio = element(0xE1, element(0xB5, struct.pack('>d', float(sample_rate))) + number(0x9F, channels))
    track = element(
        0xAE,
        number(0xD7, 1)
        + number(0x73C5, 1)
        + number(0x83, 2)
        + element(0x86, b'A_AAC')
        + element(0x63A2, asc)
        + audio,
    )
    clusters = bytearray()
    for i, pkt in enumerate(packets):
        ns = (i * frame_samples * 1_000_000_000 + sample_rate // 2) // sample_rate
        clusters += element(0x1F43B675, number(0xE7, ns) + element(0xA3, b'\x81\x00\x00\x80' + pkt))
    header = element(0x1A45DFA3, element(0x4282, b'matroska') + number(0x4287, 4) + number(0x4285, 2))
    info = element(0x1549A966, number(0x2AD7B1, 1))
    return bytes(header + element(0x18538067, info + element(0x1654AE6B, track) + clusters))


def build_pce_bits(
    prefix_len: int,
    pce_profile: int,
    rate: int,
    front: list[tuple[bool, int]],
    side: list[tuple[bool, int]],
    back: list[tuple[bool, int]],
    lfe: list[int],
    tag: int = 0,
) -> str:
    bits = (
        field(tag, 4)
        + field(pce_profile, 2)
        + frequency(rate)
        + field(len(front), 4)
        + field(len(side), 4)
        + field(len(back), 4)
        + field(len(lfe), 2)
        + field(0, 3)
        + field(0, 4)
        + '000'
    )
    for group in (front, side, back):
        for is_pair, el_tag in group:
            bits += field(int(is_pair), 1) + field(el_tag, 4)
    for el_tag in lfe:
        bits += field(el_tag, 4)
    total = prefix_len + len(bits)
    pad = (8 - (total % 8)) % 8
    bits += '0' * pad + field(0, 8)
    return bits


def build_er_pce_asc(
    aot: int,
    short_frame: bool,
    rate: int,
    front: list[tuple[bool, int]],
    side: list[tuple[bool, int]],
    back: list[tuple[bool, int]],
    lfe: list[int],
    resilience: tuple[bool, bool, bool] = (False, False, False),
    outer_aot: int | None = None,
    ext_rate: int | None = None,
    sync_sbr: bool = False,
    sync_ps: bool = False,
    pce_profile: int = 1,
    pce_rate: int | None = None,
) -> bytes:
    any_res = any(resilience)
    if outer_aot in (5, 29):
        prefix = field(outer_aot, 5) + frequency(rate) + '0000' + frequency(ext_rate or rate * 2) + field(aot, 5)
    else:
        prefix = field(aot, 5) + frequency(rate) + '0000'
    ga_head = field(int(short_frame), 1) + '0' + field(int(any_res), 1)
    pce = build_pce_bits(
        len(prefix) + len(ga_head),
        pce_profile,
        pce_rate or rate,
        front,
        side,
        back,
        lfe,
    )
    tail = ''
    if any_res:
        tail += ''.join(field(int(flag), 1) for flag in resilience) + '0'
    tail += '00'  # epConfig = 0
    if sync_sbr:
        tail += field(0x2B7, 11) + field(5, 5) + '1' + frequency(ext_rate or rate * 2)
        if sync_ps:
            tail += field(0x548, 11) + '1'
    return packed(prefix + ga_head + pce + tail)


class LdOracle:
    def __init__(self, n: int, high: int, cos: list[list[float]]):
        self.n = n
        self.high = high
        self.cos = cos
        self.pcm: list[float] = []
        self.overlap = [0.0] * n
        self.shape = 0
        self.lag = 0

    def run(self, q, gain, shape, active, update, coefficient):
        n = self.n
        lag = self.lag if update is None else update
        values = [f32(v * 2 ** ((gain - 100) / 4)) for v in q]
        if active:
            estimate = []
            for i in range(2 * n):
                relative = i - n - lag
                absolute = len(self.pcm) + relative
                value = (
                    self.overlap[relative]
                    if relative >= 0
                    else self.pcm[absolute]
                    if absolute >= 0
                    else 0.0
                )
                estimate.append(value * GAINS[coefficient] * ld_window(n, i, self.shape if i < n else shape))
            for k in range(4):
                values[k] = f32(values[k] + sum(v * c for v, c in zip(estimate, self.cos[k])))
        transformed = [2 / n * sum(values[k] * self.cos[k][i] for k in range(8)) for i in range(2 * n)]
        out = [self.overlap[i] + transformed[i] * ld_window(n, i, self.shape) for i in range(n)]
        self.overlap = [transformed[n + i] * ld_window(n, n + i, shape) for i in range(n)]
        self.pcm.extend(out)
        self.shape = shape
        if active:
            self.lag = lag
        return [f32(v / 65536) for v in out]


def ld_body(n: int, count: int, high: int, frame: int, ch: int, lfe: bool = False):
    if lfe:
        count = 1
    gain = 120 + 3 * ch + frame % 4
    shape = (frame + ch) // 2 % 2
    active = frame >= 2 and frame != 4 and not lfe
    update = (1023 - 17 * ch if frame == 2 else 13 + 11 * ch) if active and frame in (2, 6) else None
    coefficient = (frame + ch) % 8
    q = [(frame + ch + k) % 3 - 1 for k in range(8)]
    if lfe:
        q[4:] = [0] * 4
    info = '000' + field(shape, 1) + field(count, 6) + field(int(active), 1)
    if active:
        info += '1' + field(int(update is not None), 1) + (field(update, 10) if update is not None else '') + field(coefficient, 3) + '1' + '0' * (count - 1)
    books = [1] if lfe else [1] + [0] * (count - 2) + [1]
    sections = ''.join(field(book, 4) + field(1, 5) for book in books)
    spectral = tuple_bits(q[:4]) + ('' if lfe else tuple_bits(q[4:]) + tuple_bits([0] * 4) * ((n - high) // 4 - 1))
    return field(gain, 8) + info + sections + sc(0) * sum(book != 0 for book in books) + '000' + spectral, (q, gain, shape, active, update, coefficient)


def main():
    v1024 = json.loads((DEST / 'aac-ps-matrix-controller-oracles.json').read_text())['videos'][0]
    v960 = json.loads((DEST / 'aac-sbr-ps-30-oracles.json').read_text())['videos'][0]
    sbr_by_slots = {
        16: [strip_sbr_crc(bytes.fromhex(h)) for h in v1024['sbr_payloads']],
        15: [strip_sbr_crc(bytes.fromhex(h)) for h in v960['sbr_payloads']],
    }

    blob = bytearray()
    gold = bytearray()
    cases = []

    # 1. Core ER-LC (17) and ER-LTP (19) PCE cases (960 and 1024, various PCE layouts)
    pce_layouts = [
        # (layout_name, front, side, back, lfe, mapping, mask)
        ('mono-tag5', [(False, 5)], [], [], [], [0], 0x4),
        ('stereo-tag9', [(True, 9)], [], [], [], [0, 1], 0x3),
        # 3.1 layout: Front SCE (FC->bit 2), Front CPE (FL/FR->bits 0,1), LFE (bit 3) -> mapping [2, 0, 1, 3], mask 0xf
        ('quad-3p1', [(False, 3), (True, 7)], [], [], [2], [2, 0, 1, 3], 0xF),
    ]

    for aot in (17, 19):
        for n in (960, 1024):
            short_frame = n == 960
            for layout_name, front, side, back, lfe, mapping, mask in pce_layouts:
                width = len(mapping)
                asc = build_er_pce_asc(
                    aot,
                    short_frame,
                    24000,
                    front,
                    side,
                    back,
                    lfe,
                    pce_profile=3 if aot == 19 else 1,
                )
                elements = (
                    [(1 if pair else 0, tag) for pair, tag in front + side + back]
                    + [(3, tag) for tag in lfe]
                )
                banks = [LtpOracle(n) for _ in range(width)]
                rows = []
                packets = []
                ref_start = len(gold)
                for frame in range(6):
                    wire = ''
                    pcm_ch = []
                    ch = 0
                    for kind, el_tag in elements:
                        wire += field(el_tag, 4)
                        if kind == 1:
                            # Independent CPE (common_window = 0)
                            wire += '0'
                            for sub in range(2):
                                q = [[(1 if (frame + ch + k) % 2 == 0 else -1) * (1 if k % 3 != 0 else 0) for k in range(8)]]
                                if aot == 17:
                                    info = ics(0, 2, False)
                                    wire += channel(0, [1, 1], q, info=info)
                                    pcm_ch.append(banks[ch].run(0, 0, q, False, 0, 0, [False, False]))
                                else:
                                    active = frame >= 2
                                    lag = n - 5 * (frame + ch)
                                    coef = (frame + ch) % 8
                                    pcm_ch.append(banks[ch].run(0, 0, q, active, lag, coef, [True, False]))
                                    info = ics(0, 2, False)
                                    if active:
                                        info = info[:-1] + '11' + field(lag, 11) + field(coef, 3) + '10'
                                    wire += channel(0, [1, 1], q, info=info)
                                ch += 1
                        else:
                            q = [[(1 if (frame + ch + k) % 2 == 0 else -1) * (1 if k % 3 != 0 else 0) for k in range(8)]]
                            if kind == 3:
                                q = [[1, -1, 1, -1]]
                            if aot == 17:
                                books = [1] if kind == 3 else [1, 1]
                                info = ics(0, len(books), False)
                                wire += channel(0, books, q, info=info)
                                pcm_ch.append(banks[ch].run(0, 0, q, False, 0, 0, [False] * len(books)))
                            else:
                                active = frame >= 2 and kind != 3
                                lag = n - 5 * (frame + ch)
                                coef = (frame + ch) % 8
                                books = [1] if kind == 3 else [1, 1]
                                used = [True] if kind == 3 else [True, False]
                                pcm_ch.append(banks[ch].run(0, 0, q, active, lag, coef, used))
                                info = ics(0, len(books), False)
                                if active:
                                    info = info[:-1] + '11' + field(lag, 11) + field(coef, 3) + ''.join('1' if u else '0' for u in used)
                                wire += channel(0, books, q, info=info)
                            ch += 1
                    raw = packed(wire)
                    packets.append(raw)
                    rows.append(dict(offset=len(blob), bytes=len(raw)))
                    blob.extend(raw)
                    for i in range(n):
                        row_out = [0.0] * width
                        for src_ch, dst_ch in enumerate(mapping):
                            row_out[dst_ch] = pcm_ch[src_ch][i]
                        gold.extend(struct.pack('<' + 'f' * width, *row_out))

                name = f'aot{aot}-{n}-{layout_name}'
                mp4_file = f'aac-er-pce-{name}-synthetic.mp4'
                mkv_file = f'aac-er-pce-{name}-synthetic.mkv'
                case = dict(
                    kind='core',
                    name=name,
                    aot=aot,
                    n=n,
                    rate=24000,
                    output_rate=24000,
                    channels=width,
                    mask=mask,
                    asc=asc.hex(),
                    frames=rows,
                    reference_offset=ref_start,
                    reference_bytes=len(gold) - ref_start,
                    container_rate=24000,
                    container_frame_samples=n,
                    samples=6 * n,
                    pcm_offset=0,
                    slots=n // 64,
                    bands=32,
                )
                case['video'] = video_fixture([case], blob, channels=width, filename=mp4_file)
                mkv_bytes = matroska_audio_fixture(asc, packets, 24000, n, width)
                (DEST / mkv_file).write_bytes(mkv_bytes)
                case['mkv'] = mkv_file
                cases.append(case)

    # 2. ER-LD (23) PCE cases (480 and 512)
    ld_pce_layouts = [
        ('mono-tag6', [(False, 6)], [], [], [], [0], 0x4),
        ('stereo-tag11', [(True, 11)], [], [], [], [0, 1], 0x3),
        ('quad-3p1', [(False, 4), (True, 8)], [], [], [1], [2, 0, 1, 3], 0xF),
    ]
    for n in (480, 512):
        rate = 24000
        count, high = (30, 448) if n == 480 else (31, 480)
        bins = list(range(4)) + list(range(high, high + 4))
        cos = [[math.cos(math.pi / n * (i + 0.5 + n / 2) * (k + 0.5)) for i in range(2 * n)] for k in bins]
        for layout_name, front, side, back, lfe, mapping, mask in ld_pce_layouts:
            width = len(mapping)
            asc = build_er_pce_asc(23, n == 480, rate, front, side, back, lfe, pce_profile=1)
            elements = (
                [(1 if pair else 0, tag) for pair, tag in front + side + back]
                + [(3, tag) for tag in lfe]
            )
            banks = [LdOracle(n, high, cos) for _ in range(width)]
            rows = []
            packets = []
            ref_start = len(gold)
            for frame in range(6):
                wire = ''
                pcm_ch = []
                ch = 0
                for kind, el_tag in elements:
                    wire += field(el_tag, 4) + ('0' if kind == 1 else '')
                    for _ in range(2 if kind == 1 else 1):
                        encoded, params = ld_body(n, count, high, frame, ch, lfe=(kind == 3))
                        wire += encoded
                        pcm_ch.append(banks[ch].run(*params))
                        ch += 1
                raw = packed(wire)
                packets.append(raw)
                rows.append(dict(offset=len(blob), bytes=len(raw)))
                blob.extend(raw)
                for i in range(n):
                    row_out = [0.0] * width
                    for src_ch, dst_ch in enumerate(mapping):
                        row_out[dst_ch] = pcm_ch[src_ch][i]
                    gold.extend(struct.pack('<' + 'f' * width, *row_out))

            name = f'aot23-{n}-{layout_name}'
            mp4_file = f'aac-er-pce-{name}-synthetic.mp4'
            mkv_file = f'aac-er-pce-{name}-synthetic.mkv'
            case = dict(
                kind='ld',
                name=name,
                aot=23,
                n=n,
                rate=rate,
                output_rate=rate,
                channels=width,
                mask=mask,
                asc=asc.hex(),
                frames=rows,
                reference_offset=ref_start,
                reference_bytes=len(gold) - ref_start,
                container_rate=rate,
                container_frame_samples=n,
                samples=6 * n,
                pcm_offset=0,
                slots=n // 64,
                bands=32,
            )
            case['video'] = video_fixture([case], blob, channels=width, filename=mp4_file)
            mkv_bytes = matroska_audio_fixture(asc, packets, rate, n, width)
            (DEST / mkv_file).write_bytes(mkv_bytes)
            case['mkv'] = mkv_file
            cases.append(case)

    # 3. ER-LC (17) and ER-LTP (19) PCE + PS (sole normal front mono SCE with non-zero tag)
    for aot in (17, 19):
        for slots in (15, 16):
            n = slots * 64
            sbr_list = sbr_by_slots[slots]
            asc = build_er_pce_asc(
                aot,
                slots == 15,
                24000,
                [(False, 5)],
                [],
                [],
                [],
                outer_aot=29,
                ext_rate=48000,
                pce_profile=3 if aot == 19 else 1,
            )
            rows = []
            packets = []
            for frame in range(4):
                q = [[1, -1, 1, -1, 0, 0, 0, 0] if frame % 2 else [-1, 1, -1, 1, 0, 0, 0, 0]]
                info = ics(0, 2, False)
                if aot == 19 and frame >= 2:
                    info = info[:-1] + '11' + field(n - 7, 11) + field(frame % 8, 3) + '10'
                core_bits = field(5, 4) + channel(0, [1, 1], q, info=info)
                raw_sbr = sbr_list[frame % 3]
                extra_bits = ''.join(field(b, 8) for b in raw_sbr)
                pkt = packed(core_bits + extra_bits)
                packets.append(pkt)
                rows.append(dict(offset=len(blob), bytes=len(pkt), payload=raw_sbr.hex()))
                blob.extend(pkt)
            # Also build equivalent indexed channel_configuration=1 ASC and packets (tag=0) as oracle!
            control_asc = packed(
                field(29, 5) + frequency(24000) + '0001' + frequency(48000) + field(aot, 5) + field(int(slots == 15), 1) + '0000'
            )
            control_rows = []
            for frame in range(4):
                q = [[1, -1, 1, -1, 0, 0, 0, 0] if frame % 2 else [-1, 1, -1, 1, 0, 0, 0, 0]]
                info = ics(0, 2, False)
                if aot == 19 and frame >= 2:
                    info = info[:-1] + '11' + field(n - 7, 11) + field(frame % 8, 3) + '10'
                core_bits = '0000' + channel(0, [1, 1], q, info=info)
                raw_sbr = sbr_list[frame % 3]
                extra_bits = ''.join(field(b, 8) for b in raw_sbr)
                pkt = packed(core_bits + extra_bits)
                control_rows.append(dict(offset=len(blob), bytes=len(pkt)))
                blob.extend(pkt)

            name = f'ps-aot{aot}-{n}'
            mp4_file = f'aac-er-pce-{name}-synthetic.mp4'
            mkv_file = f'aac-er-pce-{name}-synthetic.mkv'
            case = dict(
                kind='ps',
                name=name,
                aot=aot,
                n=n,
                rate=24000,
                output_rate=48000,
                channels=2,
                mask=0x3,
                asc=asc.hex(),
                control_asc=control_asc.hex(),
                frames=rows,
                control_frames=control_rows,
                container_rate=48000,
                container_frame_samples=n * 2,
                samples=3 * n * 2,
                pcm_offset=0,
                slots=slots,
                bands=64,
            )
            case['video'] = video_fixture([case], blob, channels=2, filename=mp4_file)
            mkv_bytes = matroska_audio_fixture(asc, packets, 48000, n * 2, 2)
            (DEST / mkv_file).write_bytes(mkv_bytes)
            case['mkv'] = mkv_file
            cases.append(case)

    (DEST / 'aac-er-pce-packets.bin').write_bytes(blob)
    (DEST / 'aac-er-pce-reference.f32le').write_bytes(gold)
    (DEST / 'aac-er-pce.json').write_text(
        json.dumps(
            dict(
                provenance='Own ER AAC-LC (AOT 17), ER AAC-LTP (AOT 19), and ER AAC-LD (AOT 23) explicit PCE configurations, packets, MP4/Matroska containers, and scalar PCM references. No private media, FFmpeg, network, or foreign decoder.',
                cases=cases,
            ),
            indent=2,
        )
        + '\n'
    )
    print(f'generated {len(cases)} ER/LD PCE cases')


if __name__ == '__main__':
    main()
