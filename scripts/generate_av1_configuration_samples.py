#!/usr/bin/env python3
"""Owned AV1 av1C/container regressions. No codec executable or network."""
import json
from pathlib import Path
from generate_av1_separate_frame_samples import mp4

D = Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'

def units(data):
    at = 0
    while at < len(data):
        start = at; head = data[at]; at += 1
        assert head & 2
        if head & 4: at += 1
        length = 0
        for shift in range(0,56,7):
            byte = data[at]; at += 1; length |= (byte & 127) << shift
            if not byte & 128: break
        at += length
        assert at <= len(data)
        yield (head >> 3) & 15, data[start:at]

def replace_record(data, record, no_sync=False):
    result = bytearray(); at = 0
    containers = {b'moov':0,b'trak':0,b'mdia':0,b'minf':0,b'stbl':0,b'stsd':8,b'av01':78}
    while at < len(data):
        size = int.from_bytes(data[at:at+4],'big'); kind = data[at+4:at+8]
        assert size >= 8 and at+size <= len(data)
        body = data[at+8:at+size]
        if kind == b'av1C': body = record
        elif kind == b'stss' and no_sync: body = bytes(8)
        elif kind in containers:
            skip = containers[kind]
            body = body[:skip]+replace_record(body[skip:],record,no_sync)
            if kind == b'av01':
                colour = b'nclx'+b'\0\2'*3+b'\0'
                body += (8+len(colour)).to_bytes(4,'big')+b'colr'+colour
        result += (8+len(body)).to_bytes(4,'big')+kind+body
        at += size
    return bytes(result)

def main():
    seed = 'av1-separate-tiles-w128-h128-groups2'
    data = (D/(seed+'.obu')).read_bytes()
    parts = list(units(data)); sequence = next(raw for kind,raw in parts if kind==1)
    packet = b''.join(raw for kind,raw in parts if kind!=1)
    # This owned seed is profile 0, level 0, 8-bit 4:2:0, position 0.
    prefix = bytes([0x81,0,0x0c,0])
    base = mp4(packet,sequence,(128,128))
    cases = [('valid',prefix+sequence,None),
        ('delay-one',prefix[:3]+b'\x10'+sequence,None),
        ('delay-sixteen',prefix[:3]+b'\x1f'+sequence,None),
        ('reserved-high',prefix[:3]+b'\x20'+sequence,'nonzero AV1 configuration reserved bits'),
        ('reserved-delay',prefix[:3]+b'\x01'+sequence,'nonzero AV1 configuration delay reserved bits'),
        ('marker',b'\x01'+prefix[1:]+sequence,'invalid AV1 configuration marker'),
        ('empty',b'','MP4 AV1 configuration is absent'),
        ('truncated',prefix[:3],'truncated AV1 codec configuration record'),
        ('version',b'\x82'+prefix[1:]+sequence,'unsupported AV1 codec configuration version'),
        ('repeated-sequence',prefix+sequence+sequence,'repeated AV1 configuration sequence header'),
        ('late-sequence',prefix+b'\x12\x00'+sequence,'AV1 configuration sequence header must be first')]
    manifest=[]
    for name,record,error in cases:
        file='av1-configuration-'+name+'.mp4'
        (D/file).write_bytes(replace_record(base,record,no_sync=True))
        manifest.append(dict(file=file,error=error))
    # In-band sequence headers remain legal when configOBUs is empty.
    (D/'av1-configuration-inband.mp4').write_bytes(replace_record(mp4(data,sequence,(128,128)),prefix))
    manifest.append(dict(file='av1-configuration-inband.mp4',error=None))
    (D/'av1-configuration-duplicate-inband.mp4').write_bytes(replace_record(mp4(data,sequence,(128,128)),prefix+sequence))
    manifest.append(dict(file='av1-configuration-duplicate-inband.mp4',error=None))
    (D/'av1-configuration.json').write_text(json.dumps(dict(seed=seed, width=128,height=128,cases=manifest,
        provenance='Owned separate-frame/tile-group bytes, authored av1C records and MP4 wrapping. No private source, FFmpeg, external codec or network.'),indent=2)+'\n')

if __name__ == '__main__': main()
