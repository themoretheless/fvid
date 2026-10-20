"""Owned High 4:4:4 Predictive non-separated progressive CABAC and MBAFF CAVLC/CABAC PCM fixtures."""
import itertools
import json
import struct
from pathlib import Path
from generate_avc_component_fields import configuration as mbaff_configuration
from generate_avc_mbaff_direct_samples import CabacWriter
from generate_avc_pcm_formats import Writer, configuration as prog_configuration, mux

OUT = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors/avc-444-pcm'
MODES = {
    'frame': [False] * 4,
    'field': [True] * 4,
    'columns': [False, True, False, True],
    'checker': [False, True, True, False],
}


def progressive_cabac_picture(yd, cd, aso, index):
    planes = [
        [
            (x * 31 + y * 7 + p * 43 + index * 17) & ((1 << depth) - 1)
            for y in range(16)
            for x in range(32)
        ]
        for p, depth in enumerate([yd, cd, cd])
    ]
    for p, depth in enumerate([yd, cd, cd]):
        maximum = (1 << depth) - 1
        planes[p][:4] = [0, maximum, 1, maximum - 1]
    nals = []
    if aso:
        for mb in range(2):
            b = Writer()
            b.ue(mb)
            b.ue(2)
            b.ue(0)
            b.u(index, 4)
            if index == 0:
                b.ue(0)
            b.u(index * 2, 4)
            if index == 0:
                b.u(0)
                b.u(0)
            else:
                b.u(0)
            b.se(0)
            b.ue(1)
            while len(b.bits) % 8:
                b.u(1)
            c = CabacWriter(-1, 26)
            c.decision(3, 1)
            b.bits.extend(c.finish())
            b.align()
            for p, depth in enumerate([yd, cd, cd]):
                for y in range(16):
                    for x in range(16):
                        b.u(planes[p][y * 32 + mb * 16 + x], depth)
            c.low = 0
            c.range = 510
            c.width = 9
            b.bits.extend(c.finish())
            nals.append(b.nal(0x65 if index == 0 else 0x41, trailing=False))
        nals.reverse()
    else:
        b = Writer()
        b.ue(0)
        b.ue(2)
        b.ue(0)
        b.u(index, 4)
        if index == 0:
            b.ue(0)
        b.u(index * 2, 4)
        if index == 0:
            b.u(0)
            b.u(0)
        else:
            b.u(0)
        b.se(0)
        b.ue(1)
        while len(b.bits) % 8:
            b.u(1)
        c = CabacWriter(-1, 26)
        for mb in range(2):
            if mb > 0:
                c.range -= 2
                c.renormalize()
            c.decision(3 + int(mb > 0), 1)
            b.bits.extend(c.finish())
            b.align()
            for p, depth in enumerate([yd, cd, cd]):
                for y in range(16):
                    for x in range(16):
                        b.u(planes[p][y * 32 + mb * 16 + x], depth)
            c.low = 0
            c.range = 510
            c.width = 9
        b.bits.extend(c.finish())
        nals.append(b.nal(0x65 if index == 0 else 0x41, trailing=False))
    return b''.join(len(n).to_bytes(4, 'big') + n for n in nals), sum(planes, [])


def mbaff_picture(yd, cd, cabac, topology, aso, index):
    planes = [
        [
            (x * 31 + y * 73 + p * 43 + index * 17) & ((1 << depth) - 1)
            for y in range(64)
            for x in range(32)
        ]
        for p, depth in enumerate([yd, cd, cd])
    ]
    for p, depth in enumerate([yd, cd, cd]):
        maximum = (1 << depth) - 1
        planes[p][:4] = [0, maximum, 1, maximum - 1]
    nals = []
    for pair, field in enumerate(MODES[topology]):
        b = Writer()
        b.ue(pair)
        b.ue(2)
        b.ue(0)
        b.u(index, 4)
        b.u(0)
        if index == 0:
            b.ue(0)
        b.u(index * 2, 4)
        if index == 0:
            b.u(0)
            b.u(0)
        else:
            b.u(0)
        b.se(0)
        b.ue(1)
        if cabac:
            while len(b.bits) % 8:
                b.u(1)
            c = CabacWriter(-1, 26)
        for parity in range(2):
            if parity == 0:
                if cabac:
                    c.decision(70, int(field))
                else:
                    b.u(int(field))
            if cabac:
                c.decision(3 + int(parity == 1 and not field), 1)
                b.bits.extend(c.finish())
                b.align()
            else:
                b.ue(25)
                b.align()
            for p, depth in enumerate([yd, cd, cd]):
                for row in range(16):
                    y = pair // 2 * 32 + (row * 2 + parity if field else row + parity * 16)
                    for col in range(16):
                        b.u(planes[p][y * 32 + pair % 2 * 16 + col], depth)
            if cabac:
                c.low = 0
                c.range = 510
                c.width = 9
        if cabac:
            b.bits.extend(c.finish())
        nals.append(b.nal(0x65 if index == 0 else 0x41, trailing=not cabac))
    if aso:
        nals.reverse()
    return b''.join(len(n).to_bytes(4, 'big') + n for n in nals), sum(planes, [])


def single_slice_mbaff_cabac_picture(yd, cd, topology, index):
    planes = [
        [
            (x * 37 + y * 67 + p * 53 + index * 23) & ((1 << depth) - 1)
            for y in range(64)
            for x in range(32)
        ]
        for p, depth in enumerate([yd, cd, cd])
    ]
    for p, depth in enumerate([yd, cd, cd]):
        maximum = (1 << depth) - 1
        planes[p][:4] = [0, maximum, 1, maximum - 1]
    modes = MODES[topology]
    b = Writer()
    b.ue(0)
    b.ue(2)
    b.ue(0)
    b.u(index, 4)
    b.u(0)
    if index == 0:
        b.ue(0)
    b.u(index * 2, 4)
    if index == 0:
        b.u(0)
        b.u(0)
    else:
        b.u(0)
    b.se(0)
    b.ue(1)
    while len(b.bits) % 8:
        b.u(1)
    c = CabacWriter(-1, 26)
    for pair, field in enumerate(modes):
        if pair > 0:
            c.range -= 2
            c.renormalize()
        left_field = pair % 2 != 0 and modes[pair - 1]
        top_field = pair >= 2 and modes[pair - 2]
        c.decision(70 + int(left_field) + int(top_field), int(field))
        for parity in range(2):
            address = pair * 2 + parity
            left_avail = pair % 2 != 0
            top_avail = (not field and parity == 1) or pair >= 2
            c.decision(3 + int(left_avail) + int(top_avail), 1)
            b.bits.extend(c.finish())
            b.align()
            for p, depth in enumerate([yd, cd, cd]):
                for row in range(16):
                    y = pair // 2 * 32 + (row * 2 + parity if field else row + parity * 16)
                    for col in range(16):
                        b.u(planes[p][y * 32 + pair % 2 * 16 + col], depth)
            c.low = 0
            c.range = 510
            c.width = 9
            assert address < 8
    b.bits.extend(c.finish())
    nal = b.nal(0x65 if index == 0 else 0x41, trailing=False)
    return len(nal).to_bytes(4, 'big') + nal, sum(planes, [])


def unsupported_cases():
    records = []
    # 1. Progressive CABAC 4:4:4 compressed I_16x16 macroblock
    b = Writer()
    b.ue(0)
    b.ue(2)
    b.ue(0)
    b.u(0, 4)
    b.ue(0)
    b.u(0, 4)
    b.u(0)
    b.u(0)
    b.se(0)
    b.ue(1)
    while len(b.bits) % 8:
        b.u(1)
    c = CabacWriter(-1, 26)
    c.decision(3, 1)
    c.range -= 2
    c.renormalize()
    for ctx, v in [(6, 0), (7, 0), (9, 1), (10, 0), (64, 0), (60, 0), (88, 0)]:
        c.decision(ctx, v)
    b.bits.extend(c.finish())
    nal = b.nal(0x65, trailing=False)
    packet = len(nal).to_bytes(4, 'big') + nal
    config = (OUT / 'prog-pcm-f3-y8-c8-e1-a0-crop0.avcc').read_bytes()
    name = 'compressed-prog-f3-y8-c8-e1'
    for ext, data in [
        ('avcc', config),
        ('packet', packet),
        ('mp4', mux(config, [(0, True, packet)], 32, 16, 25)),
    ]:
        (OUT / f'{name}.{ext}').write_bytes(data)
    records.append(dict(
        name=name,
        kind='progressive',
        cabac=True,
        field=False,
        expect='AVC non-4:2:0 CABAC non-PCM macroblocks are not connected',
    ))
    # 2. MBAFF CAVLC & CABAC 4:4:4 compressed first pair
    for cabac, field in itertools.product([False, True], [False, True]):
        topology = 'field' if field else 'frame'
        base = f'mbaff-pcm-f3-y8-c8-e{int(cabac)}-{topology}-a0-crop0'
        name = f'compressed-{base}'
        b = Writer()
        b.ue(0)
        b.ue(2)
        b.ue(0)
        b.u(0, 4)
        b.u(0)
        b.ue(0)
        b.u(0, 4)
        b.u(0)
        b.u(0)
        b.se(0)
        b.ue(1)
        if cabac:
            while len(b.bits) % 8:
                b.u(1)
            c = CabacWriter(-1, 26)
            c.decision(70, int(field))
        for parity in range(2):
            if not cabac:
                if parity == 0:
                    b.u(int(field))
                b.ue(3)
                b.ue(0)
                b.se(0)
                b.u(1)
            else:
                c.decision(3 + int(parity == 1 and not field), 1)
                c.range -= 2
                c.renormalize()
                for ctx, v in [(6, 0), (7, 0), (9, 1), (10, 0), (64, 0), (60, 0)]:
                    c.decision(ctx, v)
                c.decision(86 if parity == 1 and not field else 88, 0)
        if cabac:
            b.bits.extend(c.finish())
        nal = b.nal(0x65, trailing=not cabac)
        original = (OUT / f'{base}-0.packet').read_bytes()
        length = int.from_bytes(original[:4], 'big')
        packet = len(nal).to_bytes(4, 'big') + nal + original[4 + length:]
        config = (OUT / f'{base}.avcc').read_bytes()
        for ext, data in [
            ('avcc', config),
            ('packet', packet),
            ('mp4', mux(config, [(0, True, packet)], 32, 64, 25)),
        ]:
            (OUT / f'{name}.{ext}').write_bytes(data)
        records.append(dict(
            name=name,
            kind='mbaff',
            cabac=cabac,
            field=field,
            expect='PCM fast path encountered compressed MBAFF syntax',
        ))
    (OUT / 'unsupported.json').write_text(json.dumps(records, indent=2) + '\n')


def generate():
    OUT.mkdir(exist_ok=True)
    cases = []
    golden = bytearray()
    for (yd, cd), aso, cropped in itertools.product([(8, 8), (10, 12), (14, 8)], [False, True], [False, True]):
        name = f'prog-pcm-f3-y{yd}-c{cd}-e1-a{int(aso)}-crop{int(cropped)}'
        config = prog_configuration(3, yd, cd, cropped, cabac=True, height_mbs=1)
        (OUT / f'{name}.avcc').write_bytes(config)
        packets = []
        records = []
        for i in range(2):
            packet, samples = progressive_cabac_picture(yd, cd, aso, i)
            (OUT / f'{name}-{i}.packet').write_bytes(packet)
            packets.append((i, i == 0, packet))
            at = len(golden) // 2
            golden.extend(struct.pack('<' + str(len(samples)) + 'H', *samples))
            records.append(dict(packet=f'{name}-{i}.packet', offset=at))
        (OUT / f'{name}.mp4').write_bytes(mux(config, packets, 32, 16, 25))
        cases.append(dict(
            name=name,
            kind='progressive',
            cabac=True,
            topology='frame',
            aso=aso,
            single_slice=not aso,
            luma_depth=yd,
            chroma_depth=cd,
            width=32,
            height=16,
            crop=[1, 1, 1, 1] if cropped else [0, 0, 0, 0],
            frames=records,
        ))
    for (yd, cd), cabac, topology, aso, cropped in itertools.product(
        [(8, 8), (10, 12), (14, 8)],
        [False, True],
        MODES,
        [False, True],
        [False, True],
    ):
        name = f'mbaff-pcm-f3-y{yd}-c{cd}-e{int(cabac)}-{topology}-a{int(aso)}-crop{int(cropped)}'
        config = mbaff_configuration(3, yd, cd, cropped, cabac=cabac, height_mbs=2, mbaff=True)
        (OUT / f'{name}.avcc').write_bytes(config)
        packets = []
        records = []
        for i in range(2):
            packet, samples = mbaff_picture(yd, cd, cabac, topology, aso, i)
            (OUT / f'{name}-{i}.packet').write_bytes(packet)
            packets.append((i, i == 0, packet))
            at = len(golden) // 2
            golden.extend(struct.pack('<' + str(len(samples)) + 'H', *samples))
            records.append(dict(packet=f'{name}-{i}.packet', offset=at))
        (OUT / f'{name}.mp4').write_bytes(mux(config, packets, 32, 64, 25))
        cases.append(dict(
            name=name,
            kind='mbaff',
            cabac=cabac,
            topology=topology,
            aso=aso,
            single_slice=False,
            luma_depth=yd,
            chroma_depth=cd,
            width=32,
            height=64,
            crop=[1, 1, 2, 2] if cropped else [0, 0, 0, 0],
            frames=records,
        ))
    for (yd, cd), topology, cropped in itertools.product(
        [(8, 8), (10, 12), (14, 8)],
        MODES,
        [False, True],
    ):
        name = f'mbaff-pcm-f3-y{yd}-c{cd}-e1-{topology}-single-crop{int(cropped)}'
        config = mbaff_configuration(3, yd, cd, cropped, cabac=True, height_mbs=2, mbaff=True)
        (OUT / f'{name}.avcc').write_bytes(config)
        packets = []
        records = []
        for i in range(2):
            packet, samples = single_slice_mbaff_cabac_picture(yd, cd, topology, i)
            (OUT / f'{name}-{i}.packet').write_bytes(packet)
            packets.append((i, i == 0, packet))
            at = len(golden) // 2
            golden.extend(struct.pack('<' + str(len(samples)) + 'H', *samples))
            records.append(dict(packet=f'{name}-{i}.packet', offset=at))
        (OUT / f'{name}.mp4').write_bytes(mux(config, packets, 32, 64, 25))
        cases.append(dict(
            name=name,
            kind='mbaff',
            cabac=True,
            topology=topology,
            aso=False,
            single_slice=True,
            luma_depth=yd,
            chroma_depth=cd,
            width=32,
            height=64,
            crop=[1, 1, 2, 2] if cropped else [0, 0, 0, 0],
            frames=records,
        ))
    assert len(cases) == 132
    (OUT / 'samples.y16').write_bytes(golden)
    (OUT / 'manifest.json').write_text(json.dumps(dict(
        provenance='Owned High 4:4:4 Predictive non-separated progressive CABAC and MBAFF CAVLC/CABAC PCM fixtures. No external codec or private media.',
        cases=cases,
    ), indent=2) + '\n')
    unsupported_cases()


if __name__ == '__main__':
    generate()
