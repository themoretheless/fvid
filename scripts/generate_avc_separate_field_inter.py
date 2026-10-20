"""Own separate-colour-plane PAFF I/P/B/P field streams with per-plane motion, parity, residual and direct scaling."""
import itertools, json, struct
from pathlib import Path
from generate_avc_pcm_formats import Writer
from generate_avc_mbaff_direct_samples import CabacWriter
from generate_avc_field_b_cabac_mixed_samples import mb_type
from generate_avc_monochrome_fractional import prediction
from avc_fixture_mp4 import mux

OUT = Path(__file__).resolve().parents[1] / "tests/fixtures/playback-errors/avc-separate-field-inter"
VECTORS = [(8, -4), (-7, 5), (3, 6)]
P_WEIGHTS = [(3, -5), (-1, 60), (2, -12)]
B_WEIGHTS = [
    [(3, -5), (-1, 90)],
    [(2, -8), (2, 12)],
    [(-1, 64), (5, -20)],
]


def configuration(depth, cabac, weighted=False, bipred=0):
    b = Writer()
    for v in [244, 0, 41]:
        b.u(v, 8)
    b.ue(0)
    b.ue(3)
    b.u(1)  # separate_colour_plane_flag
    b.ue(depth - 8)
    b.ue(0)
    b.u(0)
    b.u(0)
    b.ue(0)
    b.ue(0)
    b.ue(0)
    b.ue(2)  # max_num_ref_frames = 2
    b.u(0)
    b.ue(1)  # pic_width_in_mbs_minus1 = 1 (32)
    b.ue(0)  # pic_height_in_map_units_minus1 = 0 (32 frame height, 16 field height)
    b.u(0)  # frame_mbs_only_flag = 0
    b.u(0)  # mb_adaptive_frame_field_flag = 0
    b.u(1)  # direct_8x8_inference_flag = 1
    b.u(1)  # frame_cropping_flag = 1
    for _ in range(4):
        b.ue(1)
    b.u(0)
    sps = b.nal(0x67)
    b = Writer()
    b.ue(0)
    b.ue(0)
    b.u(int(cabac))
    b.u(0)
    b.ue(0)
    b.ue(0)
    b.ue(0)
    b.u(int(weighted))
    b.u(bipred, 2)
    b.se(0)
    b.se(0)
    b.se(0)
    b.u(1)
    b.u(0)
    b.u(0)
    pps = b.nal(0x68)
    return (
        bytes([1, 244, 0, 41, 255, 225])
        + len(sps).to_bytes(2, "big")
        + sps
        + bytes([1])
        + len(pps).to_bytes(2, "big")
        + pps
    )


def i_field_plane(source, depth, cabac, plane, bottom, index):
    b = Writer()
    b.ue(0)
    b.ue(2)
    b.ue(0)
    b.u(plane, 2)
    b.u(0, 4)
    b.u(1)
    b.u(int(bottom))
    if index == 0:
        b.ue(0)
    b.u(index, 4)
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
    for mb in range(2):
        if cabac:
            c.decision(3 + int(mb > 0), 1)
            b.bits.extend(c.finish())
            b.align()
        else:
            b.ue(25)
            b.align()
        for y in range(16):
            for x in range(16):
                b.u(source[(2 * y + int(bottom)) * 32 + mb * 16 + x], depth)
        if cabac:
            c.low = 0
            c.range = 510
            c.width = 9
            if mb == 0:
                c.range -= 2
                c.renormalize()
    if cabac:
        b.bits.extend(c.finish())
    nal = b.nal(0x65 if index == 0 else 0x41, trailing=not cabac)
    return len(nal).to_bytes(4, "big") + nal


def p_field_plane(
    plane,
    index,
    bottom,
    cabac,
    vector,
    weighted,
    residual=None,
    opposite=False,
    frame_num=1,
    poc_base=2,
):
    b = Writer()
    b.ue(0)
    b.ue(0)
    b.ue(0)
    b.u(plane, 2)
    b.u(frame_num, 4)
    b.u(1)
    b.u(int(bottom))
    b.u(index + poc_base, 4)
    b.u(0)
    b.u(1)
    b.ue(0)
    b.ue(2 if opposite else 1)
    b.ue(3)
    if weighted == "identity":
        b.ue(0)
        b.u(0)
    elif weighted:
        w, o = P_WEIGHTS[plane]
        b.ue(1)
        b.u(1)
        b.se(w)
        b.se(o)
    b.u(0)
    if cabac:
        b.ue(0)
    b.se(0)
    b.ue(1)
    skip = vector == (0, 0) and residual is None
    if cabac:
        while len(b.bits) % 8:
            b.u(1)
        c = CabacWriter(0, 26)
        for mb in range(2):
            c.decision(11 + int(mb > 0 and not skip), int(skip))
            if not skip:
                for ctx in [14, 15, 16]:
                    c.decision(ctx, 0)
                for component in range(2):
                    c.mvd(
                        component,
                        vector[component] if mb == 0 else 0,
                        abs(vector[component]) if mb else 0,
                    )
                if residual is None:
                    for ctx in [73, 74, 75, 76] if mb == 0 else [74, 74, 76, 76]:
                        c.decision(ctx, 0)
                else:
                    for _ in range(4):
                        c.decision(73, 1)
                    c.decision(60, 0)
                    for block in range(16):
                        bx = (block & 1) + ((block >> 2) & 1) * 2
                        by = ((block >> 1) & 1) + (block >> 3) * 2
                        inc = int(bx > 0 or mb > 0) + 2 * int(by > 0)
                        c.decision(93 + inc, 1)
                        if residual == "ac":
                            c.decision(306, 0)
                            c.decision(307, 1)
                            c.decision(368, 1)
                        else:
                            c.decision(306, 1)
                            c.decision(367, 1)
                        c.decision(248, 0)
                        c.bypass((mb + block + plane) % 2)
            if mb == 0:
                c.range -= 2
                c.renormalize()
        b.bits.extend(c.finish())
    elif skip:
        b.ue(2)
    else:
        for mb in range(2):
            b.ue(0)
            b.ue(0)
            b.se(vector[0] if mb == 0 else 0)
            b.se(vector[1] if mb == 0 else 0)
            b.ue(9 if residual else 0)
            if residual:
                b.se(0)
                for block in range(16):
                    b.u(1, 2)
                    b.u((mb + block + plane) % 2)
                    b.u(3, 3) if residual == "ac" else b.u(1)
    nal = b.nal(0x41, trailing=not cabac)
    return len(nal).to_bytes(4, "big") + nal


def b_field_plane(plane, index, bottom, cabac, mode, weight, vectors):
    b = Writer()
    b.ue(0)
    b.ue(1)
    b.ue(0)
    b.u(plane, 2)
    b.u(2, 4)
    b.u(1)
    b.u(int(bottom))
    b.u(2 + index, 4)
    b.u(int(mode not in ["temporal", "skip"]))
    b.u(0)
    for difference in [3, 1]:
        b.u(1)
        b.ue(0)
        b.ue(difference)
        b.ue(3)
    if weight == 1:
        b.ue(1)
        for w, offset in B_WEIGHTS[plane]:
            b.u(1)
            b.se(w)
            b.se(offset)
    if cabac:
        b.ue(0)
    b.se(0)
    b.ue(1)
    code = {"bi": 3, "spatial": 0, "temporal": 0, "skip": 0}[mode]
    if cabac:
        while len(b.bits) % 8:
            b.u(1)
        c = CabacWriter(0, 26)
        for mb in range(2):
            c.decision(24 + int(mb > 0 and mode != "skip"), int(mode == "skip"))
            if mode != "skip":
                mb_type(c, code, int(mb > 0 and code != 0))
                if code == 3:
                    for l in [0, 1]:
                        for component in range(2):
                            c.mvd(
                                component,
                                vectors[l][component] if mb == 0 else 0,
                                abs(vectors[l][component]) if mb else 0,
                            )
                for ctx in [73, 74, 75, 76] if mb == 0 else [74, 74, 76, 76]:
                    c.decision(ctx, 0)
            if mb == 0:
                c.range -= 2
                c.renormalize()
        b.bits.extend(c.finish())
    elif mode == "skip":
        b.ue(2)
    else:
        for mb in range(2):
            b.ue(0)
            b.ue(code)
            if code == 3:
                for l in [0, 1]:
                    b.se(vectors[l][0] if mb == 0 else 0)
                    b.se(vectors[l][1] if mb == 0 else 0)
            b.ue(0)
    nal = b.nal(0x01, trailing=not cabac)
    return len(nal).to_bytes(4, "big") + nal


def weave(fields):
    return [
        v
        for plane in range(3)
        for y in range(16)
        for bottom in [False, True]
        for v in fields[plane][bottom][y * 32 : (y + 1) * 32]
    ]


def generate():
    OUT.mkdir(exist_ok=True)
    cases = []
    p_motion_witnesses = 0
    p_parity_witnesses = 0
    p_scan_witnesses = 0
    b_motion_witnesses = 0
    for depth, cabac, reverse in itertools.product([8, 10, 14], [False, True], [False, True]):
        maximum = (1 << depth) - 1
        scale = 1 << (depth - 8)
        sources = [
            [(x * 179 + y * 911 + p * 43 + x * y * 13) & maximum for y in range(32) for x in range(32)]
            for p in range(3)
        ]
        order = [True, False] if reverse else [False, True]
        i_packets = [bytearray(), bytearray()]
        i_fields = [{}, {}, {}]
        for index, bottom in enumerate(order):
            for plane in range(3):
                i_packets[index].extend(i_field_plane(sources[plane], depth, cabac, plane, bottom, index))
                i_fields[plane][bottom] = [
                    v
                    for y in range(16)
                    for v in sources[plane][(2 * y + int(bottom)) * 32 : (2 * y + int(bottom) + 1) * 32]
                ]
        i_woven = weave(i_fields)

        # Matrix A: P fields with opposite/same parity, explicit weights, and none/dc/ac field residual, followed by P-skip echo
        for opposite, weighted, residual in itertools.product([False, True], [False, True], [None, "dc", "ac"]):
            res_tag = residual or "none"
            name = f"separate-field-p-{depth}-e{int(cabac)}-r{int(reverse)}-o{int(opposite)}-w{int(weighted)}-{res_tag}"
            config = configuration(depth, cabac, weighted=weighted)
            packets = [bytes(p) for p in i_packets]
            p_packets = [bytearray(), bytearray()]
            echo_packets = [bytearray(), bytearray()]
            p_fields = [{}, {}, {}]
            for index, bottom in enumerate(order):
                for plane, vector in enumerate(VECTORS):
                    p_packets[index].extend(
                        p_field_plane(
                            plane,
                            index,
                            bottom,
                            cabac,
                            vector,
                            weighted,
                            residual=residual,
                            opposite=opposite,
                            frame_num=1,
                            poc_base=2,
                        )
                    )
                    echo_packets[index].extend(
                        p_field_plane(
                            plane,
                            index,
                            bottom,
                            cabac,
                            (0, 0),
                            "identity" if weighted else False,
                            residual=None,
                            opposite=False,
                            frame_num=2,
                            poc_base=4,
                        )
                    )
                    ref_parity = (not bottom) if opposite else bottom
                    base_pred = prediction(i_fields[plane][ref_parity], depth, vector)
                    if weighted:
                        w, o = P_WEIGHTS[plane]
                        base_pred = [max(0, min(maximum, ((w * v + 1) >> 1) + o * scale)) for v in base_pred]
                    if residual is not None:
                        reconstructed = []
                        for fy in range(16):
                            for fx in range(32):
                                mb = fx // 16
                                bx = (fx % 16) // 4
                                sign = 1 if (mb + bx + plane) % 2 == 0 else -1
                                if residual == "dc":
                                    delta = (sign * 13 * 16 * scale + 32) // 64
                                else:
                                    delta = ([2, 1, -1, -2][fy % 4] * sign * 8 * 16 * scale + 32) // 64
                                    wrong = ([2, 1, -1, -2][fx % 4] * sign * 8 * 16 * scale + 32) // 64
                                    v_ok = max(0, min(maximum, base_pred[fy * 32 + fx] + delta))
                                    v_bad = max(0, min(maximum, base_pred[fy * 32 + fx] + wrong))
                                    p_scan_witnesses += int(v_ok != v_bad)
                                reconstructed.append(max(0, min(maximum, base_pred[fy * 32 + fx] + delta)))
                        base_pred = reconstructed
                    if plane:
                        wrong_mv = prediction(i_fields[plane][ref_parity], depth, VECTORS[0])
                        p_motion_witnesses += sum(a != b for a, b in zip(base_pred, wrong_mv))
                    if opposite:
                        same_pred = prediction(i_fields[plane][bottom], depth, vector)
                        p_parity_witnesses += sum(a != b for a, b in zip(base_pred, same_pred))
                    p_fields[plane][bottom] = base_pred
            packets.extend(bytes(p) for p in p_packets)
            packets.extend(bytes(p) for p in echo_packets)
            p_woven = weave(p_fields)
            for i, data in enumerate(packets):
                (OUT / f"{name}-{i}.packet").write_bytes(data)
            frames = [(i, i == 0, p) for i, p in enumerate(packets)]
            golden = i_woven + p_woven + p_woven
            for ext, data in [
                ("avcc", config),
                ("mp4", mux(config, frames, 32, 32, 50)),
                ("y16", struct.pack(f"<{len(golden)}H", *golden)),
            ]:
                (OUT / f"{name}.{ext}").write_bytes(data)
            cases.append(
                dict(
                    name=name,
                    kind="p",
                    depth=depth,
                    cabac=cabac,
                    bottom_first=reverse,
                    opposite_parity=opposite,
                    weighted=weighted,
                    residual=res_tag,
                    decode_order=[0, 1, 2],
                )
            )

        # Matrix B: Reordered I0 / P6 / B2 / P8 field pairs with moving co-located P6 and independent per-plane direct scaling
        for weight, mode in itertools.product([0, 1, 2], ["temporal", "skip", "spatial", "bi"]):
            name = f"separate-field-b-{depth}-e{int(cabac)}-r{int(reverse)}-w{weight}-{mode}"
            config = configuration(depth, cabac, bipred=weight)
            packets = [bytes(p) for p in i_packets]
            p_packets = [bytearray(), bytearray()]
            b_packets = [bytearray(), bytearray()]
            echo_packets = [bytearray(), bytearray()]
            future_fields = [{}, {}, {}]
            target_fields = [{}, {}, {}]
            for index, bottom in enumerate(order):
                for plane, vector in enumerate(VECTORS):
                    scaled = [(85 * v + 128) // 256 for v in vector]
                    back = [scaled[i] - vector[i] for i in range(2)]
                    vectors = [[0, 0], [0, 0]] if mode == "spatial" else [scaled, back]
                    p_packets[index].extend(
                        p_field_plane(
                            plane,
                            index,
                            bottom,
                            cabac,
                            vector,
                            False,
                            residual=None,
                            opposite=False,
                            frame_num=1,
                            poc_base=6,
                        )
                    )
                    b_packets[index].extend(
                        b_field_plane(plane, index, bottom, cabac, mode, weight, vectors)
                    )
                    echo_packets[index].extend(
                        p_field_plane(
                            plane,
                            index,
                            bottom,
                            cabac,
                            (0, 0),
                            False,
                            residual=None,
                            opposite=False,
                            frame_num=2,
                            poc_base=8,
                        )
                    )
                    future = prediction(i_fields[plane][bottom], depth, vector)
                    future_fields[plane][bottom] = future
                    a = prediction(i_fields[plane][bottom], depth, vectors[0])
                    z = prediction(future, depth, vectors[1])
                    w0, o0 = B_WEIGHTS[plane][0]
                    w1, o1 = B_WEIGHTS[plane][1]
                    target = []
                    for u, v in zip(a, z):
                        if weight == 0:
                            sample = (u + v + 1) // 2
                        elif weight == 2:
                            sample = (43 * u + 21 * v + 32) // 64
                        else:
                            sample = ((w0 * u + w1 * v + 2) >> 2) + (((o0 + o1) * scale + 1) >> 1)
                        target.append(max(0, min(maximum, sample)))
                    if mode in ["temporal", "skip"] and weight == 0:
                        zero = [(u + v + 1) // 2 for u, v in zip(i_fields[plane][bottom], future)]
                        b_motion_witnesses += sum(u != v for u, v in zip(target, zero))
                    target_fields[plane][bottom] = target
            packets.extend(bytes(p) for p in p_packets)
            packets.extend(bytes(p) for p in b_packets)
            packets.extend(bytes(p) for p in echo_packets)
            future_woven = weave(future_fields)
            target_woven = weave(target_fields)
            for i, data in enumerate(packets):
                (OUT / f"{name}-{i}.packet").write_bytes(data)
            frames = [
                (pts, i == 0, p)
                for i, (pts, p) in enumerate(zip([0, 1, 4, 5, 2, 3, 6, 7], packets))
            ]
            # Golden stores presentation order: pair0 (I0), pair1 (B2), pair2 (P6), pair3 (P8)
            golden = i_woven + target_woven + future_woven + future_woven
            for ext, data in [
                ("avcc", config),
                ("mp4", mux(config, frames, 32, 32, 50)),
                ("y16", struct.pack(f"<{len(golden)}H", *golden)),
            ]:
                (OUT / f"{name}.{ext}").write_bytes(data)
            cases.append(
                dict(
                    name=name,
                    kind="b",
                    depth=depth,
                    cabac=cabac,
                    bottom_first=reverse,
                    weight=weight,
                    mode=mode,
                    decode_order=[0, 2, 1, 3],
                )
            )
    assert len(cases) == 288
    assert p_motion_witnesses > 0 and p_parity_witnesses > 0 and p_scan_witnesses > 0 and b_motion_witnesses > 0
    (OUT / "manifest.json").write_text(json.dumps(cases, indent=2) + "\n")
    (OUT / "coverage.json").write_text(
        json.dumps(
            dict(
                cases=len(cases),
                p_motion_witnesses=p_motion_witnesses,
                p_parity_witnesses=p_parity_witnesses,
                p_scan_witnesses=p_scan_witnesses,
                b_motion_witnesses=b_motion_witnesses,
            ),
            indent=2,
        )
        + "\n"
    )


if __name__ == "__main__":
    generate()
