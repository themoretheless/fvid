#!/usr/bin/env python3
"""Owned PPS/colour-LUT syntax and short synthetic MP4 metadata reproducers."""
import json
import struct
from fractions import Fraction
from math import floor
from pathlib import Path
import generate_hevc_layered_sps as g
from generate_he_aac_packet_fixtures import boxes
from hevc_fixture_mp4 import box
from generate_hevc_colour_plane_fixtures import escape, packed
D = Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
ue = g.ue


def se(v):
    return ue(2*v-1 if v > 0 else -2*v)


def cell(pattern, y, cb, cr, quant=0):
    m = [[1024, 0, 0], [0, 1024, 0], [0, 0, 1024], [0, 0, 0]]
    unit = 1 << quant
    if pattern in ('bias', 'partition'):
        m[3] = [(5 + 2*y + 3*cb - 5*cr)*unit, (-7 + cb - y)*unit, (11 + cr + y)*unit]
    if pattern == 'swap':
        m[:3] = [[0, 1024, 0], [1024, 0, 0], [0, 0, 1024]]
    if pattern == 'clip':
        m = [[2047, 0, 0], [0, -1024, 0], [0, 0, -2048], [2047, -2048, 2047]]
    if pattern == 'mixed':
        m[1][1] = m[2][2] = 256
    if pattern == 'bad-coefficient':
        m[0][0] = 2048
    return m


def colour(pattern='identity', depth=0, part=0, quant=0, delta=0, inp=(8,8), out=(8,8), threshold=(0,0), split=True, ref=0):
    ny, nc = 1 << (depth+part), 1 << depth
    # Reserved-depth cases need only the complete header prefix to refuse.
    if depth > 1 or depth+part > 3:
        return ue(0)+f'{ref:06b}{depth:02b}{part:02b}', None
    half = 1 << (inp[1]-1)
    thresholds = [half + v for v in threshold]
    wanted = []
    for y in range(ny):
        for cb in range(nc):
            for cr in range(nc):
                if depth and not split:
                    wanted.append(cell(pattern, y//2, 0, 0, quant) if cb == cr == 0 else cell('identity',0,0,0))
                else:
                    wanted.append(cell(pattern,y,cb,cr,quant))
    b = ue(0)+f'{ref:06b}{depth:02b}{part:02b}'
    b += ''.join(ue(v-8) for v in (*inp,*out)) + f'{quant:02b}{delta:02b}'
    if depth:
        b += ''.join(se(v) for v in threshold)
    low = max(0, 10+inp[0]-out[0]-quant-delta-1)

    def emit(y, cb, cr):
        index = (y*nc+cb)*nc+cr
        pred = wanted[index-nc*nc] if y else cell('identity',0,0,0)
        result = ''
        for j in range(4):
            residual = [wanted[index][j][c]-pred[j][c] for c in range(3)]
            result += str(int(any(residual)))
            if any(residual):
                for value in residual:
                    assert value % (1 << quant) == 0
                    mag = abs(value) >> quant
                    result += ue(mag >> low)
                    if low:
                        result += f'{mag & ((1<<low)-1):0{low}b}'
                    if mag:
                        result += str(int(value < 0))
        return result

    if depth:
        b += str(int(split))
    if depth and split:
        for k in range(2):
            for cb in range(2):
                for cr in range(2):
                    for i in range(1 << part):
                        b += emit((1 << part)*k+i,cb,cr)
    else:
        for i in range(1 << part):
            b += emit(i << depth,0,0)
    probes = set()
    max_y, max_c = (1 << inp[0])-1, (1 << inp[1])-1
    cy = {0, 1, max_y}
    for y in range(ny):
        cy.update([y*(1 << (inp[0]-depth-part)), ((y+1)*(1 << (inp[0]-depth-part)))-1])
    cc = [{0, max_c}, {0, max_c}]
    for c in range(2):
        cc[c].update(v for v in (thresholds[c]-1,thresholds[c]) if 0 <= v <= max_c)
    samples = []
    for y in sorted(cy):
        for cb in sorted(cc[0]):
            for cr in sorted(cc[1]):
                iy = y >> (inp[0]-depth-part)
                icb = int(cb >= thresholds[0]) if depth else 0
                icr = int(cr >= thresholds[1]) if depth else 0
                m = wanted[(iy*nc+icb)*nc+icr]
                values = []
                for c in range(3):
                    # Independent rational affine-map oracle, not the bit-reader.
                    weighted = sum(m[j][c]*v for j,v in enumerate((y,cb,cr)))
                    value = floor(Fraction(weighted, 1 << (10+inp[0]-out[0])) + Fraction(1,2)) + m[3][c]
                    values.append(max(0,min((1 << out[c != 0])-1,value)))
                samples.append([[y,cb,cr],values])
    return b, dict(dimensions=[ny,nc,nc], coefficients=wanted, input=list(inp), output=list(out), thresholds=thresholds, probes=samples)


def pps(layer=3, id=0, scaling='none', reference=0, locations=(), mapping=None, future=False):
    b = ue(id)+ue(0)+'0000000'+ue(0)+ue(0)+se(0)+'000'+se(0)+se(0)+'000000'+'00'
    b += str(int(scaling == 'explicit'))
    if scaling == 'explicit':
        b += '1'+ue(25)+'1'*15+'01'*19  # first 4x4 matrix is 21, not SPS's 17
    b += '0'+ue(0)+'1'+'1'+'0100'+('0001' if future else '0000')
    b += '1'+str(int(scaling == 'infer'))
    if scaling == 'infer':
        b += f'{reference:06b}'
    b += ue(len(locations))
    for loc in locations:
        b += f'{loc["id"]:06b}'
        for key in ('scaled','region'):
            b += str(int(key in loc))
            if key in loc:
                b += ''.join(se(v) for v in loc[key])
        b += str(int('phases' in loc))
        if 'phases' in loc:
            b += ''.join(ue(v) for v in loc['phases'])
    b += '0' if mapping is None else '1'+mapping
    if future:
        b += '010001001'
    b += '1'
    return ((34 << 9) | (layer << 3) | 1).to_bytes(2,'big') + escape(packed(b))


def movie(vps, base_sps, current_sps, base_pps, current_pps):
    roots = list(boxes((D.parent/'hevc/main-ipb.mp4').read_bytes()))
    assert roots[-1][0] == b'moov'
    def config(data):
        out=bytearray(data[:23]); at=23
        for _ in range(data[22]):
            head=data[at:at+3]; at+=3; units=[]
            for _ in range(int.from_bytes(head[1:],'big')):
                n=int.from_bytes(data[at:at+2],'big'); at+=2
                units.append(data[at:at+n]); at+=n
            units={32:[vps],33:[base_sps,current_sps],34:[base_pps,current_pps]}.get(head[0]&63,units)
            out+=head[:1]+struct.pack('>H',len(units))+b''.join(struct.pack('>H',len(u))+u for u in units)
        assert at==len(data)
        return bytes(out)
    def rewrite(tag,payload):
        if tag in (b'moov',b'trak',b'mdia',b'minf',b'stbl'):
            payload=b''.join(rewrite(t,p) for t,p in boxes(payload))
        elif tag==b'stsd':
            payload=payload[:8]+b''.join(box(t,p[:78]+b''.join(box(k,config(v) if k==b'hvcC' else v) for k,v in boxes(p[78:]))) for t,p in boxes(payload[8:]))
        return box(tag,payload)
    return b''.join(rewrite(t,p) for t,p in roots)


def main():
    cases=[]
    definitions=[('identity',{}),('explicit-scaling',dict(scaling='explicit')),
        ('infer-from-sps',dict(scaling='infer')),('infer-from-pps',dict(scaling='infer',base_scaling='explicit')),
        ('locations',dict(locations=[dict(id=0,scaled=[1,-2,3,4],region=[-1,2,3,-4])])),
        ('phase-extremes',dict(locations=[dict(id=0,phases=[31,0,63,0])])),
        ('default-phase',dict(locations=[dict(id=0)])),
        ('444-phase',dict(chroma=3,locations=[dict(id=0)])),
        ('ignored-phase',dict(base_context=True,locations=[dict(id=3,phases=[4096,2048,512,1024])])),
        ('future',dict(future=True)),
        ('bad-inferred-phase',dict(target_size=[256,2048],locations=[dict(id=0)],error='phase exceeds range')),
        ('bad-phase',dict(locations=[dict(id=0,phases=[32,0,8,8])],error='resampling phase')),
        ('bad-chroma-phase',dict(locations=[dict(id=0,phases=[0,0,64,8])],error='resampling phase')),
        ('bad-offset',dict(locations=[dict(id=0,scaled=[16384,0,0,0])],error='signed value')),
        ('bad-region',dict(locations=[dict(id=0,region=[64,0,64,0])],error='remove the picture region')),
        ('bad-location-count',dict(locations=[dict(id=0),dict(id=0)],error='unsigned value')),
        ('bad-scaling-self',dict(scaling='infer',reference=3,error='available reference')),
        ('bad-scaling-reserved',dict(scaling='infer',reference=63,error='available reference')),
        ('bad-scaling-disabled',dict(scaling='infer',current_scaling='disabled',error='contradicts active parameters')),
        ('bad-scaling-reference-disabled',dict(scaling='infer',base_sps_scaling='disabled',error='reference SPS disables')),
        ('bad-base-infer',dict(base_context=True,scaling='infer',error='contradicts active parameters'))]
    for pattern in ('identity','bias','swap','clip'):
        definitions.append(('cm-'+pattern,dict(cm=dict(pattern=pattern))))
    for depth,part,split in [(0,3,False),(1,0,True),(1,2,True),(1,2,False)]:
        definitions.append((f'cm-part-{depth}-{part}-{int(split)}',dict(cm=dict(pattern='partition',depth=depth,part=part,split=split))))
    for q in range(4):
        definitions.append((f'cm-quant-{q}',dict(cm=dict(pattern='bias',quant=q,delta=q))))
    definitions += [('cm-adapt',dict(cm=dict(pattern='partition',depth=1,threshold=(-128,127)))),
        ('cm-depth10',dict(inp=(10,10),target=(10,10),cm=dict(inp=(10,10),out=(10,10),pattern='bias'))),
        ('cm-depth16',dict(inp=(16,16),target=(16,16),cm=dict(inp=(16,16),out=(16,16),pattern='clip'))),
        ('cm-low-zero',dict(target=(16,16),cm=dict(out=(16,16),pattern='bias',quant=3,delta=3))),
        ('cm-mixed-output',dict(cm=dict(out=(10,8),pattern='mixed'))),
        ('cm-mixed-input',dict(inp=(10,8),target=(12,10),cm=dict(inp=(10,8),out=(12,10),pattern='bias'))),
        ('bad-cm-id',dict(id=8,cm={},error='ID below eight')),
        ('bad-cm-reference',dict(cm=dict(ref=3),error='direct reference')),
        ('bad-cm-input',dict(cm=dict(inp=(10,10),out=(10,10)),error='input depth contradicts')),
        ('bad-cm-output',dict(cm=dict(out=(12,10)),error='output depth contradicts')),
        ('bad-cm-coefficient',dict(cm=dict(pattern='bad-coefficient'),error='signed twelve bits')),
        ('bad-cm-threshold',dict(cm=dict(depth=1,threshold=(128,0)),error='signed value')),
        ('reserved-cm-depth',dict(cm=dict(depth=2),error='reserved HEVC',unsupported=True)),
        ('reserved-cm-y',dict(cm=dict(depth=1,part=3),error='reserved HEVC',unsupported=True))]
    for label,o in definitions:
        inp=o.get('inp',(8,8)); target=o.get('target',(10,10))
        base_fmt=dict(g.BASE,depth=list(inp))
        vps,_,_=g.vps(dict(g.fmt(o.get('chroma',1),depth=target),size=o.get('target_size',[256,192])),base_format=base_fmt)
        base_sps=g.sps(multilayer=False,independent=True,scaling=o.get('base_sps_scaling','explicit'),buffers=4,latency=1,coded_depth=inp)
        current_sps=g.sps(scaling=o.get('current_scaling','default'))
        base_pps=pps(layer=0,scaling=o.get('base_scaling','none'))
        mapping,oracle=colour(**o['cm']) if 'cm' in o else (None,None)
        current_pps=pps(layer=0 if o.get('base_context') else 3,id=o.get('id',0),
            scaling=o.get('scaling','none'),reference=o.get('reference',0),locations=o.get('locations',()),mapping=mapping,future=o.get('future',False))
        prefix='hevc-layered-pps-'+label
        for suffix,raw in [('vps',vps),('base.sps',base_sps),('current.sps',current_sps),('base.pps',base_pps),('pps',current_pps),('mp4',movie(vps,base_sps,current_sps,base_pps,current_pps))]:
            (D/(prefix+'.'+suffix)).write_bytes(raw)
        cases.append(dict(prefix=prefix,error=o.get('error'),unsupported=o.get('unsupported',False),base_context=o.get('base_context',False),
            scaling=o.get('scaling','none'),first_coefficient=21 if o.get('scaling')=='explicit' or o.get('base_scaling')=='explicit' else (17 if o.get('scaling')=='infer' or o.get('base_context') else 16),
            locations=o.get('locations',[]),oracle=oracle,future=o.get('future',False)))
    (D/'hevc-layered-pps.json').write_text(json.dumps(dict(cases=cases,provenance='Owned VPS/SPS/PPS bit writing and rational LUT oracle. Synthetic MP4s retain original 17-frame seed samples but replace configuration; tests parse metadata directly. No scalable profile or multilayer picture acceptance is claimed. No private data, external codec, FFmpeg or network.'),indent=2)+'\n')
    print('Layered PPS cases:',len(cases))


if __name__=='__main__':main()
