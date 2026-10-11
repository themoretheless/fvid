#!/usr/bin/env python3
"""Original extracted-entry signalled reset on owned PCM/inter pictures, offline."""
import argparse
import json
import struct
import subprocess
from pathlib import Path
from generate_hevc_poc_reset_fixtures import Reader, ue, sps_width
from generate_hevc_colour_plane_fixtures import bits, packed, escape, child, packet_units
from generate_he_aac_packet_fixtures import DEST, boxes
from generate_hevc_vps_reset_updates import parts
from hevc_fixture_mp4 import box


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--irap',type=int,choices=[16,17,18,21],help='author BLA/CRA entry variants')
    parser.add_argument('--unaligned',action='store_true',help='use unaligned VPS for IRAP qualification')
    args=parser.parse_args()
    assert args.irap or not args.unaligned
    seed = DEST / 'hevc-pcm-mono-reference-rext8.mp4'
    root = Path(__file__).resolve().parents[1]
    meta = json.loads(subprocess.check_output(['cargo', 'run', '--quiet', '--offline',
        '--no-default-features', '--example', 'hevc_colour_plane_fixture_meta', '--', str(seed)], cwd=root))
    original = dict(boxes(seed.read_bytes()))
    stbl = child(child(child(child(original[b'moov'], b'trak'), b'mdia'), b'minf'), b'stbl')
    sizes = struct.unpack('>3I', child(stbl, b'stsz')[12:])
    at = struct.unpack_from('>I', child(stbl, b'stco'), 8)[0]
    packets = []
    for size in sizes:
        packets.append(seed.read_bytes()[at:at+size]); at += size
    config = child(next(boxes(child(stbl, b'stsd')[8:]))[1][78:], b'hvcC')
    at = 23; width = None
    for _ in range(config[22]):
        header = config[at:at+3]; at += 3
        for _ in range(int.from_bytes(header[1:], 'big')):
            size = int.from_bytes(config[at:at+2], 'big'); at += 2
            nal = config[at:at+size]; at += size
            if header[0] & 63 == 33: _, width = sps_width(nal)
    assert width is not None and meta['long_term_entries'] == 0
    alignment='unaligned' if args.unaligned else 'aligned'
    roots, _, _ = parts((DEST / f'hevc-poc-vps-{alignment}-msb-reset-synthetic.mp4').read_bytes())
    stem=f'hevc-irap-reset-{args.irap}-{alignment}' if args.irap else 'hevc-extracted-reset'
    labels=['cycle','msb','bad-full','bad-signalled','bad-cycle-range','bad-cycle-truncated'] if args.irap else ['msb','full','bad-no-reset']
    cases = []
    for label in labels:
        outputs = []
        for index, packet in enumerate(packets):
            units = packet_units(packet)
            assert len(units) == len(meta['packets'][index]) == 1
            nal = units[0][4:]; info = meta['packets'][index][0]
            value = bits(bytes(info['rbsp']))
            end = info['entropy_byte_offset'] * 8
            alignment = value[:end].rfind('1')
            r = Reader(value)
            assert r.read(1) == 1
            if info['idr']: r.read(1)
            r.ue(); r.read(meta['extra_bits']); r.ue()
            if meta['output_flag']: r.read(1)
            if index == 0:
                assert info['kind'] in (19, 20)
                # Convert the intra picture to TRAIL_R, retaining CABAC/PCM bytes.
                prefix = (value[:r.at] if args.irap else value[:1] + value[2:r.at]) + '0001' + '0'
                if meta['short_term_sets']: prefix += '0'
                prefix += ue(0) + ue(0)
                if meta['long_term_present']: prefix += ue(0)
                if meta['temporal_mvp_enabled']: prefix += '0'
                nal = bytes([(args.irap or 1)<<1, nal[1]]) + nal[2:]
            else:
                prefix = value[:r.at] + f'{index+1:04b}'
                r.read(width)
            prefix += value[r.at:alignment]
            extension = '00'
            if index == 0 and label != 'bad-no-reset':
                extension = '11' + '000000' + str(int(label == 'full')) + '0001'
            if args.irap and index == 0:
                extension = '00' if label == 'cycle' else ('10' if label == 'bad-full' else '11' if label == 'bad-signalled' else '01') + '000000'
                if label == 'bad-signalled': extension += '0' + '0001'
                extension += '00000000' if label == 'bad-cycle-truncated' else ue((1<<28)+1 if label == 'bad-cycle-range' else 1)
            elif not args.unaligned:
                extension += '0'  # cycle-present flag in aligned VPS
            extension = bits(packed(extension))
            prefix += ue(len(extension)//8) + extension
            rewritten = nal[:2] + escape(packed(prefix+'1') + bytes(info['rbsp'][end//8:]))
            outputs.append(len(rewritten).to_bytes(4,'big')+rewritten)
        def rewrite(tag,payload):
            if tag in (b'moov',b'trak',b'mdia',b'minf',b'stbl'):
                payload=b''.join(rewrite(t,p) for t,p in boxes(payload))
            elif tag==b'stsz': payload=bytes(8)+struct.pack('>I',3)+b''.join(struct.pack('>I',len(p)) for p in outputs)
            elif tag==b'stts': payload=bytes(4)+struct.pack('>III',1,3,1)
            elif tag==b'stsc': payload=bytes(4)+struct.pack('>IIII',1,1,3,1)
            elif tag==b'stss': payload=bytes(4)+struct.pack('>II',1,1)
            elif tag in (b'mvhd',b'mdhd'): payload=payload[:16]+struct.pack('>I',3)+payload[20:]
            elif tag==b'tkhd': payload=payload[:20]+struct.pack('>I',3)+payload[24:]
            return box(tag,payload)
        name=f'{stem}-{label}-synthetic.mp4'
        (DEST/name).write_bytes(box(b'ftyp',roots[b'ftyp'])+box(b'mdat',b''.join(outputs))+rewrite(b'moov',roots[b'moov']))
        cases.append(dict(file=name,expectation='invalid' if label.startswith('bad-') else 'acceptance',frames=3,first_poc=17 if label == 'cycle' else 1,period=0))
    (DEST/f'{stem}.json').write_text(json.dumps(dict(cases=cases,provenance='Original owned monochrome PCM/inter seed, rewritten TRAIL_R intra entry, empty RPS and signalled reset. No private parameters, external codec, FFmpeg or network.'),indent=2)+'\n')

if __name__=='__main__': main()
