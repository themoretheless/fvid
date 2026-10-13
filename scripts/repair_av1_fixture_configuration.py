#!/usr/bin/env python3
"""Repair owned AV1 fixture summary bytes only; dry run unless --apply.

Packet bytes, offsets and lengths stay unchanged. Deliberately malformed
av1-configuration-* fixtures are excluded. No private media is inspected.
"""
import argparse
import hashlib
import json
import re
from pathlib import Path
from av1_fixture_configuration import configuration

ROOT = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors'

def vint(data, at, identifier=False):
    first = data[at]
    width = next((n for n in range(1, 9) if first & (1 << (8-n))), None)
    if width is None or at + width > len(data):
        raise ValueError('invalid EBML integer')
    value = int.from_bytes(data[at:at+width], 'big')
    return (value if identifier else value & ((1 << (7*width))-1)), at+width

def records(data, start, end, webm):
    while start < end:
        if webm:
            kind, body = vint(data, start, True)
            size, body = vint(data, body)
            stop = body + size
            children = {0x18538067, 0x1654ae6b, 0xae}
            skip = 0
            target = kind == 0x63a2
        else:
            size = int.from_bytes(data[start:start+4], 'big')
            kind = bytes(data[start+4:start+8])
            body = start + 8
            if size == 1:
                size = int.from_bytes(data[body:body+8], 'big')
                body += 8
            stop = start + size if size else end
            children = {b'moov', b'trak', b'mdia', b'minf', b'stbl', b'stsd', b'av01'}
            skip = {b'stsd': 8, b'av01': 78}.get(kind, 0)
            target = kind == b'av1C'
        if not body <= stop <= end or stop <= start:
            raise ValueError('invalid container size')
        if webm and kind == 0xae:
            child = body
            codecs = []
            while child < stop:
                child_kind, child_body = vint(data, child, True)
                child_size, child_body = vint(data, child_body)
                child_stop = child_body + child_size
                if not child_body <= child_stop <= stop or child_stop <= child:
                    raise ValueError('invalid track entry size')
                if child_kind == 0x86:
                    codecs.append(bytes(data[child_body:child_stop]))
                child = child_stop
            if codecs != [b'V_AV1']:
                start = stop
                continue
        if target:
            yield body, stop
        elif kind in children:
            yield from records(data, body+skip, stop, webm)
        start = stop

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--apply', action='store_true')
    args = parser.parse_args()
    hashes = {}
    changed = skipped = 0
    for path in sorted([*ROOT.glob('av1*.webm'), *ROOT.glob('av1*.mp4')]):
        if path.name.startswith('av1-configuration-'):
            continue
        original = path.read_bytes()
        data = bytearray(original)
        for start, end in records(data, 0, len(data), path.suffix == '.webm'):
            record = bytes(data[start:end])
            if len(record) < 5 or record[0] != 0x81:
                raise ValueError(f'{path.name}: unexpected configuration structure')
            try:
                expected = configuration(record[4:])
            except ValueError as error:
                # Negative sequence fixtures must retain their intended failure.
                print(f'skip {path.name}: {error}')
                skipped += 1
                continue
            data[start+1:start+3] = expected[1:3]
        if data != original:
            old = hashlib.sha256(original).hexdigest()
            new = hashlib.sha256(data).hexdigest()
            if old in hashes and hashes[old] != new:
                raise ValueError('inconsistent duplicate fixture hash')
            hashes[old] = new
            changed += 1
            if args.apply:
                path.write_bytes(data)
    manifests = 0
    for path in sorted(ROOT.glob('*.json')):
        original = path.read_text()
        updated = re.sub(r"[0-9a-f]{64}", lambda match: hashes.get(match[0], match[0]), original)
        if updated != original:
            json.loads(updated)
            manifests += 1
            if args.apply:
                path.write_text(updated)
    print(json.dumps(dict(apply=args.apply, changed=changed, skipped=skipped,
        manifests=manifests, unique_hashes=len(hashes))))

if __name__ == '__main__':
    main()
