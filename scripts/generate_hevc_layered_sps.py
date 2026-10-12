#!/usr/bin/env python3
"""Original Annex F SPS/VPS metadata and short synthetic MP4 reproducers.

No private media, network or external codec. Picture decoding is deliberately
not an acceptance expectation: these exercise resolved parameter syntax.
"""
import json
import struct
from pathlib import Path
from generate_he_aac_packet_fixtures import boxes
from hevc_fixture_mp4 import box
from generate_hevc_colour_plane_fixtures import escape, packed

D = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors'


def ue(v):
    s = f'{v+1:b}'
    return '0' * (len(s)-1) + s


def profile(id=1):
    return f'{id:08b}' + '0' * 80


def ptl(present, sub=2, level=120):
    return (profile() if present else '') + f'{level:08b}' + ('0' * 16 if sub else '')


def fmt(chroma=1, separate=False, depth=(10, 10), size=(256, 192)):
    return dict(chroma=chroma, separate=separate, depth=list(depth), size=list(size), crop=[1, 2, 3, 4])


BASE = dict(chroma=1, separate=False, depth=[8, 8], size=[128, 128], crop=[0]*4)


def representation(f):
    return (f'{f["size"][0]:016b}{f["size"][1]:016b}' + '1' + f'{f["chroma"]:02b}' +
            (str(int(f['separate'])) if f['chroma'] == 3 else '') +
            f'{f["depth"][0]-8:04b}{f["depth"][1]-8:04b}' + '1' + ''.join(ue(v) for v in f['crop']))


def vps(target, independent=False, one_format=False, bad_geometry=False, wide=False, dpb=6, short_temporal=False, base_buffers=16, base_latency=0, base_format=None, dependency_type=2, poc_absent=False, output_highest=False, alternative_output=False, ordering_rows=None):
    if bad_geometry:
        target = dict(target, size=[255, 192])
    base_format = BASE if base_format is None else base_format
    formats = [base_format] if one_format else [base_format, target, fmt(3, True, (16, 12))]
    # Two layers (IDs 0 and 3), three temporal sublayers, base ordering 16/2/0.
    b = '0000' + '11' + '000001' + '0101' + '1'*16 + ptl(True)
    b += '0' + ue(base_buffers-1) + ue(2) + ue(base_latency)
    b += '000011' + ue(1) + '1001' + '0' + '1'
    b += '1' * ((-len(b)) % 8)
    b += ptl(False, level=90) + '0' + '0'*16 + '1' + '000011' + '0000'
    b += '0' if independent else '1'
    if independent:
        b += ue(0)  # no additional layer sets for the two independent roots
    b += ('1'+'000'+'001' if short_temporal else '0') + '00'
    if output_highest and independent:
        raise ValueError('highest-only fixture requires a dependent layer')
    # Both layers remain necessary in the dependent highest-output operation.
    b += ue(1) + ue(0) + ('01' if output_highest else '00') + '11'
    if output_highest:
        b += str(int(alternative_output))
    b += ue(len(formats)-1) + ''.join(representation(f) for f in formats)
    if len(formats) > 1:
        b += '1' + '01'  # explicit dependent-layer representation index 1
    b += '01'  # max_one_active_ref=0, aligned POC=1
    if independent:
        b += str(int(poc_absent))  # poc_lsb_not_present_flag for layer 3
    b += '1'  # explicit per-temporal DPB syntax
    rows = []
    for tid in range(2 if short_temporal else 3):
        if tid:
            b += '1'
        counts = [4+tid, dpb+tid]
        reorder = (300 if wide else 1)+tid
        latency = 0 if tid == 0 else tid+1
        if ordering_rows is not None:
            counts, reorder, latency = ordering_rows[tid]
        b += ''.join(ue(v-1) for v in counts) + ue(reorder) + ue(latency)
        rows.append([counts[1], reorder, latency])
    b += ue(0) + '1' + f'{dependency_type:02b}' + ue(0) + '0' + '0' + '1'
    return b'\x40\x01' + escape(packed(b)), formats, rows


def scaling_data():
    # First 4x4 list is 17 everywhere; other lists use their defaults.
    return '1' + ue(17) + '1'*15 + '01'*19  # initial coefficient 8 + signed delta 9


def sps(multilayer=True, independent=False, update=None, scaling='disabled',
        reference=0, rps=0, pcm=False, scc=False, bad_vps=False, sub=2, nesting=True, buffers=16, reorder=2, latency=0, coded_depth=(8, 8), coded_size=(128, 128), coded_chroma=1):
    b = ('0001' if bad_vps else '0000') + ('111' if multilayer else f'{sub:03b}')
    if not multilayer:
        b += str(int(nesting)) + ptl(True, sub=sub)
    b += ue(0)
    if multilayer:
        b += '0' if update is None else '1' + f'{update:08b}'
    else:
        b += ue(coded_chroma) + ('0' if coded_chroma==3 else '') + ue(coded_size[0]) + ue(coded_size[1]) + '0' + ue(coded_depth[0]-8) + ue(coded_depth[1]-8)
    b += ue(4)  # eight POC LSB bits
    if not multilayer:
        b += '0' + ue(buffers-1) + ue(reorder) + ue(latency)
    b += ue(0) + ue(2) + ue(0) + ue(3) + ue(2) + ue(2)
    b += str(int(scaling != 'disabled'))
    if scaling != 'disabled':
        if multilayer:
            b += str(int(scaling == 'infer'))
        if scaling == 'infer':
            b += f'{reference:06b}'
        else:
            b += str(int(scaling == 'explicit'))
            if scaling == 'explicit':
                b += scaling_data()
    b += '01' + str(int(pcm))  # AMP=0, SAO=1, optional PCM
    if pcm:
        b += '10011001' + ue(0) + ue(1) + '1'
    b += ue(int(rps != 0))
    if rps:
        b += ue(rps) + ue(0) + (ue(0)+'1')*rps
    b += '1' + ue(2) + f'{1:08b}'+'1'+f'{7:08b}'+'0'  # two long-term entries
    b += '110'  # temporal MVP, strong smoothing, no VUI
    if scc:
        b += '1' + '11010001' + '0'*9 + '1'
        b += '01' + ue(2) + ue(1) + '1' + ue(1)
        for values, depth in [([3, 9], 16), ([4, 10], 12), ([5, 11], 12)]:
            b += ''.join(f'{v:0{depth}b}' for v in values)
        b += '00' + '0' + '010101'
    else:
        b += '1' + '01000000' + '1'
    b += '1'
    layer = 0 if independent else 3
    return ((33 << 9) | (layer << 3) | 1).to_bytes(2, 'big') + escape(packed(b))


def movie(vps_nal, sps_nal):
    seed = (D.parent/'hevc/main-ipb.mp4').read_bytes()
    roots = list(boxes(seed))
    assert roots[-1][0] == b'moov'

    def config(data):
        out = bytearray(data[:23])
        at = 23
        for _ in range(data[22]):
            head = data[at:at+3]
            at += 3
            units = []
            for _ in range(int.from_bytes(head[1:], 'big')):
                n = int.from_bytes(data[at:at+2], 'big')
                at += 2
                unit = data[at:at+n]
                at += n
                units.append(vps_nal if head[0]&63 == 32 else unit)
            if head[0]&63 == 33:
                units.append(sps_nal)
            out += head[:1] + struct.pack('>H', len(units))
            out += b''.join(struct.pack('>H', len(u))+u for u in units)
        assert at == len(data)
        return bytes(out)

    def rewrite(tag, payload):
        if tag in (b'moov', b'trak', b'mdia', b'minf', b'stbl'):
            payload = b''.join(rewrite(t, p) for t, p in boxes(payload))
        elif tag == b'stsd':
            payload = payload[:8]+b''.join(box(t, p[:78]+b''.join(
                box(k, config(v) if k == b'hvcC' else v) for k, v in boxes(p[78:])
            )) for t, p in boxes(payload[8:]))
        return box(tag, payload)
    return b''.join(rewrite(t, p) for t, p in roots)


def main():
    cases = []
    definitions = [(f'c{c}-d{d}', fmt(c, depth=(d, d)), {}, {}, None)
                   for c in range(4) for d in (8, 10, 16)]
    definitions += [(f'separate-d{d}', fmt(3, True, (d, d)), {}, {}, None) for d in (8, 10, 16)]
    target = fmt()
    definitions += [
        ('update', target, {}, dict(update=2), None),
        ('shared-base', target, {}, dict(multilayer=False, independent=True), None),
        ('independent', target, dict(independent=True), dict(multilayer=False), None),
        ('infer-scaling', target, {}, dict(scaling='infer'), None),
        ('explicit-scaling', target, {}, dict(scaling='explicit'), None),
        ('default-scaling', target, {}, dict(scaling='default'), None),
        ('rps-bound', target, {}, dict(rps=7), None),
        ('pcm', target, {}, dict(pcm=True), None),
        ('scc', target, {}, dict(update=2, scc=True), None),
        ('wide-reorder', target, dict(wide=True), {}, None),
        ('short-temporal', target, dict(short_temporal=True), {}, None),
        ('bad-update', target, {}, dict(update=3), 'representation index'),
        ('bad-single-update', target, dict(one_format=True), dict(update=0), 'single VPS representation'),
        ('bad-geometry', target, dict(bad_geometry=True), {}, 'block geometry'),
        ('bad-vps', target, {}, dict(bad_vps=True), 'different VPS'),
        ('bad-independent', target, dict(independent=True), {}, 'layer dependencies'),
        ('bad-dependent', target, {}, dict(multilayer=False), 'layer dependencies'),
        ('bad-scaling-self', target, {}, dict(scaling='infer', reference=3), 'available reference'),
        ('bad-scaling-absent', target, {}, dict(scaling='infer', reference=7), 'available reference'),
        ('bad-scaling-reserved', target, {}, dict(scaling='infer', reference=63), 'available reference'),
        ('bad-rps', target, {}, dict(rps=8), 'reference set value'),
        ('bad-pcm-depth', fmt(depth=(8, 8)), {}, dict(pcm=True), 'PCM parameters'),
        ('bad-temporal', target, dict(independent=True), dict(multilayer=False, sub=3), 'temporal layers'),
        ('bad-nesting', target, dict(independent=True), dict(multilayer=False, nesting=False), 'nesting contradicts VPS'),
        ('bad-shared-format', fmt(size=(64, 64)), {}, dict(multilayer=False, independent=True), 'format exceeds'),
        ('bad-dpb', target, dict(dpb=17), {}, 'per-layer DPB'),
        ('bad-coded-buffers', target, dict(independent=True, base_buffers=8), dict(multilayer=False), 'ordering exceeds'),
        ('bad-coded-reorder', target, dict(independent=True), dict(multilayer=False, reorder=3), 'ordering exceeds'),
        ('bad-coded-latency', target, dict(independent=True, base_latency=2), dict(multilayer=False), 'ordering exceeds'),
    ]
    for label, target, vo, so, error in definitions:
        vps_nal, formats, rows = vps(target, **vo)
        sps_nal = sps(**so)
        prefix = f'hevc-layered-sps-{label}'
        (D/(prefix+'.vps')).write_bytes(vps_nal)
        (D/(prefix+'.sps')).write_bytes(sps_nal)
        (D/(prefix+'-synthetic.mp4')).write_bytes(movie(vps_nal, sps_nal))
        shared = so.get('independent', False)
        independent = label == 'independent'
        index = so.get('update', 1)
        expected = BASE if independent else formats[min(index, len(formats)-1)]
        cases.append(dict(prefix=prefix, error=error, expectation='invalid-metadata' if error else 'metadata-acceptance', format=expected, rows=rows,
                          shared=shared, independent=independent,
                          multilayer=so.get('multilayer', True), representation=None if independent else index,
                          scaling=so.get('scaling', 'disabled'), rps=so.get('rps', 0),
                          pcm=so.get('pcm', False), scc=so.get('scc', False)))
    # An active base SPS with a distinguishable non-default 4x4 scaling matrix.
    (D/'hevc-layered-sps-active-base.sps').write_bytes(sps(multilayer=False, independent=True, scaling='explicit', buffers=4, latency=1))
    (D/'hevc-layered-sps-disabled-base.sps').write_bytes(sps(multilayer=False, independent=True, buffers=4, latency=1))
    (D/'hevc-layered-sps.json').write_text(json.dumps(dict(cases=cases,
        provenance='Original owned VPS/SPS syntax. MP4s retain the synthetic 17-frame '
        'Main I/P/B seed samples and append the authored SPS in hvcC. Tests parse '
        'configuration directly; no scalable-profile or multilayer playback acceptance '
        'is claimed. Generation and tests use no private media, external codec or network.'
    ), indent=2)+'\n')
    print('Layered SPS cases:', len(cases))


if __name__ == '__main__':
    main()
