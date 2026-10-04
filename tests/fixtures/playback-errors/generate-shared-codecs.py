"""Name the pre-existing short synthetic AV1 ramp as a dispatch regression.

The source is the checked-in generated fixture, never a private user video.
No encoder, reference tool or network is used here or by ordinary tests.
"""
from pathlib import Path
root = Path(__file__).parent
(root / 'shared-av1-private.webm').write_bytes((root.parent / 'av1/ramp.webm').read_bytes())

# A valid tiny WebM envelope with a truncated VP9 packet: the decoder must report
# a packet error, rather than turn corruption into a capability refusal.
def vint(size):
    for width in range(1, 9):
        if size < (1 << (7 * width)) - 1:
            return ((1 << (7 * width)) | size).to_bytes(width, 'big')
    raise ValueError('element too large')
def atom(identifier, payload):
    return bytes.fromhex(identifier) + vint(len(payload)) + payload
def uint(identifier, value):
    return atom(identifier, value.to_bytes(max(1, (value.bit_length()+7)//8), 'big'))
header = atom('1A45DFA3', atom('4282', b'webm') + uint('4287', 4) + uint('4285', 2))
video = atom('E0', uint('B0', 16) + uint('BA', 16))
track = atom('AE', uint('D7', 1) + uint('73C5', 1) + uint('83', 1) + atom('86', b'V_VP9') + video)
cluster = atom('1F43B675', uint('E7', 0) + atom('A3', b'\x81\0\0\x80\x00'))
segment = atom('18538067', atom('1549A966', uint('2AD7B1', 1000000)) + atom('1654AE6B', track) + cluster)
(root / 'shared-vp9-truncated.webm').write_bytes(header + segment)

# Move the generated ramp's sequence OBU to configOBUs; the first frame has no
# in-band sequence header and therefore requires actual decoder initialization.
source = (root.parent / 'av1/ramp.webm').read_bytes()
def element_vint(data, pos, is_size):
    first = data[pos]
    width = 1
    while not first & (1 << (8-width)):
        width += 1
    value = int.from_bytes(data[pos:pos+width], 'big')
    if is_size:
        value &= (1 << (7*width))-1
    return value, pos+width
def fields(data, begin=0, end=None):
    end = len(data) if end is None else end
    while begin < end:
        identifier, pos = element_vint(data, begin, False)
        length, payload = element_vint(data, pos, True)
        finish = min(payload+length, end)
        yield identifier, payload, finish
        begin = finish
containers = {0x18538067, 0x1654AE6B, 0xAE, 0xE0}
def find_field(identifier, begin=0, end=None):
    for kind, payload, finish in fields(source, begin, end):
        if kind == identifier:
            return source[payload:finish]
        if kind in containers:
            found = find_field(identifier, payload, finish)
            if found is not None:
                return found
record = find_field(0x63A2)
width = int.from_bytes(find_field(0xB0), 'big')
height = int.from_bytes(find_field(0xBA), 'big')
assert record[0] == 0x81 and len(record) >= 4
obus = (root.parent / 'av1/ramp.obu').read_bytes()
parsed = []
pos = 0
while pos < len(obus):
    begin = pos
    header_byte = obus[pos]
    kind = (header_byte >> 3) & 15
    pos += 1 + bool(header_byte & 4)
    assert header_byte & 2
    size, shift = 0, 0
    while True:
        byte = obus[pos]
        pos += 1
        size |= (byte & 127) << shift
        if byte < 128:
            break
        shift += 7
    pos += size
    assert pos <= len(obus)
    parsed.append((kind, obus[begin:pos]))
sequence = next(data for kind, data in parsed if kind == 1)
frame = next(data for kind, data in parsed if kind == 6)
video = atom('E0', uint('B0', width) + uint('BA', height))
track = atom('AE', uint('D7', 1) + uint('73C5', 1) + uint('83', 1) + atom('86', b'V_AV1') + atom('63A2', record[:4]+sequence) + video)
cluster = atom('1F43B675', uint('E7', 0) + atom('A3', b'\x81\0\0\0'+frame))
segment = atom('18538067', atom('1549A966', uint('2AD7B1', 1000000)) + atom('1654AE6B', track) + cluster)
(root / 'shared-av1-private-sequence.webm').write_bytes(header + segment)

# The decoder's padded coded-plane stride differs from the odd visible size.
(root / 'shared-vp9-stride.webm').write_bytes((root.parent / 'vp9/odd10.webm').read_bytes())

# Hide the first synthetic reference picture in the container, retaining its
# coded bytes and its reference role. The first presented picture now has a
# later block timestamp and must still start at presentation time zero.
hidden = bytearray((root.parent / 'vp9/motion.webm').read_bytes())
def hide_first_block(begin=0,end=None):
    for kind,payload,finish in fields(hidden,begin,end):
        if kind==0xA3:
            _,position=element_vint(hidden,payload,True)
            flags=position+2
            assert hidden[flags]&0x06==0
            hidden[flags]|=0x08
            return True
        if kind in {0x18538067,0x1F43B675}:
            if hide_first_block(payload,finish):return True
    return False
assert hide_first_block()
(root / 'shared-vp9-hidden-leading.webm').write_bytes(hidden)

# Static HDR records in the synthetic AV1 CodecPrivate. Their arbitrary public
# numeric values exercise record transport, not HDR image-quality acceptance.
import struct
cll_payload=b'\x01'+struct.pack('>HH',1234,567)+b'\x80'
mdcv_payload=b'\x02'+struct.pack('>8HII',8500,39850,6550,2300,35400,14600,15635,16450,10000000,1)+b'\x80'
metadata_obus=bytes([0x2a,len(cll_payload)])+cll_payload+bytes([0x2a,len(mdcv_payload)])+mdcv_payload
hdr_private=record[:4]+sequence+metadata_obus
hdr_video=atom('E0',uint('B0',width)+uint('BA',height))
hdr_track=atom('AE',uint('D7',1)+uint('73C5',1)+uint('83',1)+atom('86',b'V_AV1')+atom('63A2',hdr_private)+uint('23E383',40000000)+hdr_video)
hdr_cluster=atom('1F43B675',uint('E7',0)+atom('A3',b'\x81\0\0\0'+frame))
hdr_segment=atom('18538067',atom('1549A966',uint('2AD7B1',1000000))+atom('1654AE6B',hdr_track)+hdr_cluster)
(root/'shared-av1-hdr-carry.webm').write_bytes(header+hdr_segment)
