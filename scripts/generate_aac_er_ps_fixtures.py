#!/usr/bin/env python3
"""Own ER AAC-LC (AOT 17) and ER AAC-LTP (AOT 19) Parametric Stereo packets,
MP4/Matroska containers, and independent reference payloads across 960/1024
frame lengths, 24/48 kHz output rates, and explicit/sync/implicit signalling.
No private media, foreign encoder/decoder, FFmpeg, or network access.
"""
import json, struct
from generate_aac_main_tools_fixtures import channel, ics, Filterbank
from generate_aac_ltp_transition_fixtures import Oracle
from generate_aac_ps_matroska_fixtures import element, number
from generate_he_aac_packet_fixtures import DEST, field, frequency, packed, video_fixture


def strip_sbr_crc(raw: bytes) -> bytes:
    """Convert an extension_type=14 (CRC) SBR payload to extension_type=13 (no CRC)."""
    bits = ''.join(f'{b:08b}' for b in raw)
    kind = int(bits[:4], 2)
    if kind == 13:
        return raw
    assert kind == 14
    body = bits[14:]
    # Strip old byte-alignment zero padding (at most 7 zero bits).
    # In our authored PS fixtures, body ends with '0' (reserved_ps) + fill '1's
    # inside ps_data, followed by 0..7 '0' bits of sbr_extension_data alignment.
    # Actually, let's compute the exact alignment shift: removing 10 CRC bits
    # shifts bit length by -10 (i.e. -2 mod 8), so we add 2 zero bits before byte packing!
    return packed('1101' + body + '00')


def build_asc(aot: int, slots: int, rate: int, signal: str) -> bytes:
    ga = field(slots == 15, 1) + '0000'
    if signal == 'explicit':
        bits = field(29, 5) + frequency(24000) + '0001' + frequency(rate) + field(aot, 5) + ga
    elif signal == 'sync':
        bits = (
            field(aot, 5) + frequency(24000) + '0001' + ga
            + field(0x2b7, 11) + field(5, 5) + '1' + frequency(rate)
            + field(0x548, 11) + '1'
        )
    elif signal == 'implicit':
        bits = (
            field(aot, 5) + frequency(24000) + '0001' + ga
            + field(0x2b7, 11) + field(5, 5) + '1' + frequency(rate)
        )
    else:
        raise ValueError(signal)
    return packed(bits)


def main():
    v1024 = json.loads((DEST / 'aac-ps-matrix-controller-oracles.json').read_text())['videos'][0]
    v960 = json.loads((DEST / 'aac-sbr-ps-30-oracles.json').read_text())['videos'][0]
    payloads_by_slots = {
        16: [strip_sbr_crc(bytes.fromhex(h)) for h in v1024['sbr_payloads']],
        15: [strip_sbr_crc(bytes.fromhex(h)) for h in v960['sbr_payloads']],
    }
    blob = bytearray()
    cases = []

    for aot in (17, 19):
        for slots in (16, 15):
            n = slots * 64
            sbr_list = payloads_by_slots[slots]
            rows = []
            bad_rows = []
            core_pcm = []
            inactive_pcm = []
            fb = Filterbank(n) if aot == 17 else None
            ltp_oracle = Oracle(n) if aot == 19 else None
            ltp_mutant = Oracle(n) if aot == 19 else None

            for frame in range(6):
                q = [[1, -1, 1, -1, 0, 0, 0, 0] if frame % 2 else [-1, 1, -1, 1, 0, 0, 0, 0]]
                if aot == 17:
                    info = ics(0, 2, False)
                    spectra = [[float(v * 1024) for v in q[0]] + [0.0] * (n - 8)]
                    core_pcm.extend(fb.run(0, spectra, 0))
                    active = False
                    lag = 0
                    coefficient = 0
                else:
                    active = frame >= 3
                    lag = n - 7 * (frame % 3)
                    coefficient = frame % 8
                    core_pcm.extend(ltp_oracle.run(0, 0, q, active, lag, coefficient, [True, False]))
                    inactive_pcm.extend(ltp_mutant.run(0, 0, q, False, lag, coefficient, [True, False]))
                    info = ics(0, 2, False)
                    if active:
                        info = info[:-1] + '11' + field(lag, 11) + field(coefficient, 3) + '10'

                core_bits = '0000' + channel(0, [1, 1], q, info=info)
                raw_sbr = sbr_list[frame % 3]
                extra_bits = ''.join(field(b, 8) for b in raw_sbr)
                good_pkt = packed(core_bits + extra_bits)
                bad_pkt = packed(core_bits + '1110' + extra_bits[4:])
                rows.append(dict(
                    offset=len(blob),
                    bytes=len(good_pkt),
                    payload=raw_sbr.hex(),
                    active=active,
                    lag=lag,
                    coefficient=coefficient,
                ))
                blob.extend(good_pkt)
                bad_rows.append(dict(
                    offset=len(blob),
                    bytes=len(bad_pkt),
                ))
                blob.extend(bad_pkt)

            core_file = f'aac-er-ps-aot{aot}-{n}-core.f32le'
            (DEST / core_file).write_bytes(struct.pack('<' + str(len(core_pcm)) + 'f', *core_pcm))
            inactive_file = None
            if aot == 19:
                inactive_file = f'aac-er-ps-aot{aot}-{n}-inactive.f32le'
                (DEST / inactive_file).write_bytes(struct.pack('<' + str(len(inactive_pcm)) + 'f', *inactive_pcm))

            for rate, bands in ((24000, 32), (48000, 64)):
                for signal in ('explicit', 'sync', 'implicit'):
                    asc = build_asc(aot, slots, rate, signal)
                    name = f'aot{aot}-{n}-{rate}-{signal}'
                    frame_samples = slots * bands * 2
                    c = dict(
                        name=name,
                        aot=aot,
                        slots=slots,
                        frame_samples=n,
                        rate=rate,
                        bands=bands,
                        signal=signal,
                        asc=asc.hex(),
                        frames=rows,
                        bad_frames=bad_rows,
                        core_reference=core_file,
                        inactive_reference=inactive_file,
                        container_rate=rate,
                        container_frame_samples=frame_samples,
                        pcm_offset=0,
                        samples=6 * frame_samples * 2,
                    )
                    c['video'] = video_fixture(
                        [dict(c)],
                        blob,
                        channels=2,
                        filename=f'aac-er-ps-{name}-synthetic.mp4',
                    )
                    c['bad_video'] = video_fixture(
                        [dict(c, frames=[rows[0], bad_rows[1]])],
                        blob,
                        channels=2,
                        filename=f'aac-er-ps-{name}-crc-malformed-synthetic.mp4',
                    )
                    audio = element(0xe1, element(0xb5, struct.pack('>d', rate)) + number(0x9f, 2))
                    track = element(
                        0xae,
                        number(0xd7, 1)
                        + number(0x73c5, 1)
                        + number(0x83, 2)
                        + element(0x86, b'A_AAC')
                        + element(0x63a2, asc)
                        + audio,
                    )
                    clusters = bytearray()
                    for i, row in enumerate(rows):
                        pkt = blob[row['offset']:row['offset'] + row['bytes']]
                        ns = (i * frame_samples * 1_000_000_000 + rate // 2) // rate
                        clusters += element(0x1f43b675, number(0xe7, ns) + element(0xa3, b'\x81\x00\x00\x80' + pkt))
                    header = element(0x1a45dfa3, element(0x4282, b'matroska') + number(0x4287, 4) + number(0x4285, 2))
                    info = element(0x1549a966, number(0x2ad7b1, 1))
                    mka = header + element(0x18538067, info + element(0x1654ae6b, track) + clusters)
                    mka_name = f'aac-er-ps-{name}-synthetic.mka'
                    (DEST / mka_name).write_bytes(mka)
                    c['matroska'] = mka_name
                    cases.append(c)

    (DEST / 'aac-er-ps-packets.bin').write_bytes(blob)
    (DEST / 'aac-er-ps.json').write_text(
        json.dumps(
            dict(
                cases=cases,
                provenance='Own sparse nonzero ER AAC-LC (AOT 17) and ER AAC-LTP (AOT 19) mono cores with authored SBR/PS extensions (no CRC), across 960/1024 frame lengths, 24/48 kHz output rates, and explicit/sync/implicit signalling. Independent scalar core/LTP oracle + PS QMF/decorrelation reference. No private media, foreign codec, FFmpeg, or network.',
            ),
            indent=2,
        )
        + '\n'
    )
    print(len(cases), 'ER AAC PS cases generated')


if __name__ == '__main__':
    main()
