#!/usr/bin/env python3
"""Owned monochrome CABAC/PCM inter-layer references; no external codec.

Reuse documented synthetic entropy and saved seed pixels. Header rewrites enable
one same-AU reference and an original nonidentity reference region. The sample
oracle uses the independent rational FIR implementation, not the native decoder.
Run separately from tests, which consume saved fixtures only.
"""
import json
import subprocess
from pathlib import Path
from generate_hevc_layered_reconstruction import configuration, layer, parameter_ids, slice_pps
from generate_hevc_layered_sps import vps
from generate_hevc_colour_plane_fixtures import bits, packed, escape, unescape, ue_end
from generate_hevc_poc_reset_fixtures import Reader, ue, reset_pps
from generate_hevc_layered_pps import se
from generate_hevc_inter_layer_resampling import derive, oracle
from hevc_fixture_mp4 import mux

ROOT = Path(__file__).resolve().parents[1]
D = ROOT / 'tests/fixtures/playback-errors'


def main():
    build = subprocess.check_output(['cargo', 'build', '--quiet', '--locked', '--offline',
        '--no-default-features', '--example', 'hevc_colour_plane_fixture_meta',
        '--message-format=json'], cwd=ROOT, text=True)
    helper = next(row['executable'] for row in map(json.loads, build.splitlines())
        if row.get('reason') == 'compiler-artifact' and row.get('executable')
        and row['target']['name'] == 'hevc_colour_plane_fixture_meta')
    seed = 'hevc-pcm-mono-reference-rext8'
    meta = json.loads(subprocess.check_output([helper, str(D / (seed+'.mp4'))], cwd=ROOT))
    assert meta['chroma_format'] == 0 and meta['extra_bits'] == 0
    params = configuration(bytes(meta['configuration']))
    fmt = dict(size=meta['dimensions'], depth=[meta['depth'], meta['chroma_depth']],
        chroma=0, separate=False, crop=[0]*4)
    raw_vps, _, _ = vps(fmt, base_format=fmt, output_highest=True)
    source_ptl = bits(unescape(params[33][0][2:]))[8:104]
    value = bits(unescape(raw_vps[2:]))
    raw_vps = raw_vps[:2]+escape(packed(value[:32]+source_ptl+value[128:]))
    raw_pps = parameter_ids(layer(params[34][0], 3), 0, [1, 0])
    # reset_pps supplies the header-extension and multilayer-extension flags.
    # Replace its reset/scaling/count/mapping tail with an authored location.
    body = bits(unescape(reset_pps(raw_pps, False)[2:]))
    body = body[:body.rfind('1')]
    assert body[-4:] == '0010'
    location = '00'+ue(1)+'000000'+'0'+'1'+''.join(se(2) for _ in range(4))+'0'+'0'
    region_pps = raw_pps[:2]+escape(packed(body[:-4]+location+'1'))
    # The identity control uses the same coded reference and extension syntax.
    identity_pps = raw_pps[:2]+escape(packed(body+'1'))
    groups = []
    for index in range(2):
        base, enhancement = [], []
        for header in meta['packets'][index]:
            assert header['first'] and not header['dependent']
            raw = bytes(header['rbsp'])
            value = bits(raw)
            end = header['entropy_byte_offset']*8
            align = value[:end].rfind('1')
            if index == 0:
                assert header['kind'] in (19,20)
                r = Reader(value, 2); r.ue()
                r.read(meta['extra_bits']); assert r.ue() == 2
                if meta['output_flag']: r.read(1)
                rewritten = value[:r.at]+'0'*meta['poc_bits']+'0'+value[r.at:align]
                rewritten = slice_pps(rewritten, 2)
            else:
                assert header['kind'] not in (19,20)
                r = Reader(value, 1); r.ue(); r.read(meta['extra_bits'])
                assert r.ue() in (0,1)
                if meta['output_flag']: r.read(1)
                r.read(meta['poc_bits']); start = r.at
                if r.read(1):
                    if meta['short_term_sets'] > 1: r.read((meta['short_term_sets']-1).bit_length())
                else: r.read(header['short_term_bit_length'])
                if meta['long_term_present']:
                    if meta['long_term_entries']: assert r.ue() == 0
                    assert r.ue() == 0
                if meta['temporal_mvp_enabled']: r.read(1)
                empty = '0'+('0' if meta['short_term_sets'] else '')+ue(0)+ue(0)
                if meta['long_term_present']:
                    empty += (ue(0) if meta['long_term_entries'] else '')+ue(0)
                if meta['temporal_mvp_enabled']: empty += str(int(header['temporal_mvp']))
                rewritten = slice_pps(value[:start]+empty+'1'+value[r.at:align], 1)
            enhancement.append(((header['kind']<<9)|(3<<3)|1).to_bytes(2,'big')+
                escape(packed(rewritten+ue(0)+'1')+raw[end//8:]))
            base.append(((header['kind']<<9)|1).to_bytes(2,'big')+escape(raw))
        groups.append(base+enhancement)
    width, height = meta['dimensions']
    pixels = (D/(seed+'.yuv')).read_bytes()
    assert pixels[:width*height] == pixels[width*height:2*width*height]
    for name, pps in [('region', region_pps), ('identity', identity_pps)]:
        controls = [raw_vps, params[33][0], params[34][0], pps]
        stream = b''.join(b'\0\0\0\1'+n for n in [*controls,*sum(groups,[])])
        prefix = 'hevc-layered-mono-inter-'+name
        (D/(prefix+'.mp4')).write_bytes(mux(stream, meta['depth'], width, height,
            chroma_format=0, chroma_depth=meta['chroma_depth'], access_units=groups, multilayer=True))
        if name == 'region':
            bad_groups = [g[:] for g in groups]
            last = bad_groups[-1][-1]
            source_header = meta['packets'][1][-1]
            entropy_size = len(source_header['rbsp'])-source_header['entropy_byte_offset']
            assert entropy_size > 1
            rbsp = unescape(last[2:])
            bad_groups[-1][-1] = last[:2]+escape(rbsp[:len(rbsp)-entropy_size+1])
            bad_stream = b''.join(b'\0\0\0\1'+n for n in [*controls,*sum(bad_groups,[])])
            (D/(prefix+'-truncated.mp4')).write_bytes(mux(bad_stream, meta['depth'], width, height,
                chroma_format=0, chroma_depth=meta['chroma_depth'], access_units=bad_groups, multilayer=True))

        case = dict(reference_size=[width,height], current_size=[width,height],
            reference_format=0, current_format=0, reference_depth=[8,8], current_depth=[8,8],
            scaled=[0]*4, region=[2]*4 if name=='region' else [0]*4, phases=None)
        expected = oracle(case, [list(pixels[width*height:2*width*height]),[],[]], derive(case))[0]
        if name == 'region': assert bytes(expected) != pixels[width*height:2*width*height]
        (D/(prefix+'.yuv')).write_bytes(pixels[:width*height]+bytes(expected))
        (D/(prefix+'.json')).write_text(json.dumps(dict(seed=seed, region=case['region'],
            width=width, height=height, frames=2, depth=8,
            provenance='Owned saved PCM/CABAC seed, authored Annex F/H headers and rational luma FIR oracle. No private source, FFmpeg or external codec.'), separators=(',',':'))+'\n')


if __name__ == '__main__':
    main()
