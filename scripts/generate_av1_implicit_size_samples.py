#!/usr/bin/env python3
"""Owned AV1 MP4/WebM samples with implicit final OBU size; offline wrapping only."""
import hashlib
import json
from generate_av1_configuration_samples import D, units, replace_record
from generate_av1_separate_frame_samples import mp4
from generate_av1_spatial_container_samples import webm
from av1_fixture_configuration import configuration
from generate_audio_resample_window_fixture import ebml

def webm_record(packet, record):
    track = ebml('d7',b'\1')+ebml('83',b'\1')+ebml('86',b'V_AV1')+ebml('63a2',record)
    track += ebml('23e383',(20000000).to_bytes(4,'big'))
    track += ebml('e0',ebml('b0',b'\x80')+ebml('ba',b'\x80'))
    info = ebml('1549a966',ebml('2ad7b1',(1000000).to_bytes(3,'big')))
    cluster = ebml('1f43b675',ebml('e7',b'\0')+ebml('a3',b'\x81\0\0\x80'+packet))
    return ebml('1a45dfa3',ebml('4282',b'webm'))+ebml('18538067',info+ebml('1654ae6b',ebml('ae',track))+cluster)


def implicit(raw, extension=False):
    at = 1 + bool(raw[0] & 4)
    while raw[at] & 128:
        at += 1
    at += 1
    header = bytes([raw[0] & ~2])
    ext = raw[1:2] if raw[0] & 4 else b''
    if extension and not ext:
        header = bytes([header[0] | 4]); ext = b'\0'
    return header + ext + raw[at:]

def main():
    seed = 'av1-separate-tiles-w128-h128-groups2'
    parts = list(units((D/(seed+'.obu')).read_bytes()))
    sequence = next(raw for kind,raw in parts if kind == 1)
    assert parts[-1][0] == 4
    prefix = b''.join(raw for _,raw in parts[:-1])
    variants = [('tile-group',prefix+implicit(parts[-1][1])),
        ('tile-group-extension',prefix+implicit(parts[-1][1],True)),
        ('padding',b''.join(raw for _,raw in parts)+b'\x78\0\0'),
        ('metadata',b''.join(raw for _,raw in parts)+b'\x28\x01\x04\xd2\x02\x37\x80')]
    manifest = []
    webm_cases = []
    for name,packet in variants:
        file = 'av1-implicit-size-'+name+'.mp4'
        data = mp4(packet,sequence,(128,128))
        (D/file).write_bytes(data)
        last_kind = 4 if name.startswith('tile-group') else (15 if name=='padding' else 5)
        manifest.append(dict(file=file,sha256=hashlib.sha256(data).hexdigest(),last_kind=last_kind))
        file = 'av1-implicit-size-'+name+'.webm'
        data = webm([packet],sequence,(128,128))
        (D/file).write_bytes(data)
        webm_cases.append(dict(file=file,sha256=hashlib.sha256(data).hexdigest(),last_kind=last_kind))
    for name,packet,error,config in [
        ('extension-truncated',b''.join(raw for _,raw in parts)+b'\x2c','truncated AV1 OBU extension',False),
        ('extension-reserved',b''.join(raw for _,raw in parts)+b'\x2c\x01','invalid AV1 OBU extension reserved bits',False),
        ('configuration',b''.join(raw for _,raw in parts),'AV1 low-overhead OBU requires a size field',True)]:
        file = 'av1-implicit-size-invalid-'+name+'.mp4'
        data = mp4(packet,sequence,(128,128))
        if config:
            data = replace_record(data,configuration(sequence)[:4]+implicit(sequence))
        (D/file).write_bytes(data)
        manifest.append(dict(file=file,sha256=hashlib.sha256(data).hexdigest(),error=error,configuration=config))
        file = 'av1-implicit-size-invalid-'+name+'.webm'
        record = configuration(sequence)
        if config: record = record[:4]+implicit(sequence)
        data = webm_record(packet,record)
        (D/file).write_bytes(data)
        webm_cases.append(dict(file=file,sha256=hashlib.sha256(data).hexdigest(),error=error,configuration=config))
    (D/'av1-implicit-size.json').write_text(json.dumps(dict(seed=seed,cases=manifest,webm_cases=webm_cases,
        provenance='Owned separate-tile stream; only final OBU size framing modified or owned metadata/padding appended. No private media or external codec.'),indent=2)+'\n')

if __name__ == '__main__': main()
