"""Create unedited native dispatch cases from existing short synthetic fixtures.
Replace edts with equal-size free boxes, preserving every media/sample offset.
No private media, encoder, network or reference command is used.
"""
from pathlib import Path
root = Path(__file__).parent

def remove_edits(data):
    output = bytearray(data)
    def walk(start, end):
        at = start
        while at < end:
            if end-at < 8:
                raise ValueError('truncated box')
            size = int.from_bytes(output[at:at+4], 'big')
            kind = bytes(output[at+4:at+8])
            header = 8
            if size == 1:
                size = int.from_bytes(output[at+8:at+16], 'big')
                header = 16
            elif size == 0:
                size = end-at
            if size < header or at+size > end:
                raise ValueError('invalid box extent')
            if kind == b'edts':
                output[at+4:at+8] = b'free'
            elif kind in (b'moov', b'trak'):
                walk(at+header, at+size)
            at += size
    walk(0, len(output))
    return output

for name, source in [('shared-avc-baseline', 'short/avc-baseline.mp4'),
                     ('shared-avc-bframes', 'short/avc-bframes.mp4'),
                     ('shared-hevc-main', 'hevc/main-ipb.mp4'),
                     ('shared-hevc-main10', 'hevc/main10-ipb.mp4')]:
    (root / (name+'.mp4')).write_bytes(remove_edits((root.parent/source).read_bytes()))
