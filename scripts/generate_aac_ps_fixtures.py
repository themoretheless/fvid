#!/usr/bin/env python3
"""Original PS syntax packets and a three-packet HE-AACv2 synthetic video.

Only checked-in numeric protocol words are reused. All parameter choices,
payloads and AAC/SBR syntax are authored here. No private media, FFmpeg,
foreign codec, network or test-time generation. This is a PS parser oracle;
the video reproduces pending PS synthesis, not PCM conformance.
"""
import hashlib
import json
from generate_he_aac_packet_fixtures import DEST, asc, field, packed, packet, video_fixture
from generate_aac_sbr_dsp_fixtures import header as sbr_header
from generate_aac_sbr_data_fixtures import word as sbr_word
from generate_aac_sbr_extension_fixtures import crc
from generate_aac_sbr_frequency_oracles import tables

BOOKS = {b['name']: dict(b['rows']) for b in json.loads((DEST/'aac-ps-huffman-codewords.json').read_text())['books']}

def sized(text, escaped=False):
    count = (len(text)+7)//8
    if escaped:
        count = max(15, count)
    assert count <= 270
    # Reserved IDs (11) in any complete remaining octets; arbitrary fill bits.
    text += '1'*(count*8-len(text))
    return (field(count,4) if count<15 else '1111'+field(count-15,8)) + text

def ps(iid_mode, icc_mode, stage, slots=32, variable=False, count=None,
       phase=True, escaped=False, include_header=None,
       iid_value=2, icc_value=2, phase_value=1, temporal_delta=0):
    if count is None:
        count = [1,2,0][stage]
    if include_header is None:
        include_header = stage==0
    ext = phase or escaped
    bits = field(include_header,1)
    if include_header:
        bits += field(iid_mode is not None,1)+(field(iid_mode,3) if iid_mode is not None else '')
        bits += field(icc_mode is not None,1)+(field(icc_mode,3) if icc_mode is not None else '')+field(ext,1)
    envs = [1,2,3,4] if variable else [0,1,2,4]
    bits += field(variable,1)+field(envs.index(count),2)
    borders = [(e+1)*slots//count-1 for e in range(count)]
    if variable:
        # Include a last-envelope overhang into the next frame's interpolation.
        borders = [min(slots-1, (e+1)*slots//(count+1)) for e in range(count)]
        bits += ''.join(field(b,5) for b in borders)
    result = dict(header_present=include_header,iid_mode=iid_mode,icc_mode=icc_mode,
                  extension=ext,variable_borders=variable,borders=borders,iid=[],icc=[],phase=[])
    for label, mode in [('iid',iid_mode),('icc',icc_mode)]:
        if mode is None:
            continue
        bands = [10,20,34][mode%3]
        for e in range(count):
            temporal = stage>0 or e>0
            values = ([temporal_delta]*bands if temporal else [iid_value if label=='iid' else icc_value]+[0]*(bands-1))
            family = 'Icc' if label=='icc' else 'IidFine' if mode>=3 else 'IidCoarse'
            codebook = BOOKS[family+('Time' if temporal else 'Frequency')]
            bits += field(temporal,1)+''.join(codebook[v] for v in values)
            result[label].append(dict(temporal=temporal,values=values))
    if ext:
        inner = '00'+field(phase,1)
        phase_rows = dict(enabled=phase,ipd=[],opd=[])
        if phase:
            assert iid_mode is not None
            bands = [5,11,17][iid_mode%3]
            for e in range(count):
                for label in ['ipd','opd']:
                    temporal = stage>0 or e>0
                    values = [temporal_delta%8]*bands if temporal else [phase_value]+[0]*(bands-1)
                    codebook = BOOKS[label.title()+('Time' if temporal else 'Frequency')]
                    inner += field(temporal,1)+''.join(codebook[v] for v in values)
                    phase_rows[label].append(dict(temporal=temporal,values=values))
        inner += '0'  # reserved_ps
        result['phase'].append(phase_rows)
        bits += sized(inner,escaped)
    return bits, result

def sbr(ps_bits, stage):
    # The same authored noise-only SBR geometry as the prior HE-AAC fixture.
    high_bands = len(tables(10,27,0,False,0,0)[1])-1
    temporal = stage>0
    env = sbr_word(0,0)*high_bands if temporal else field(2,7)+sbr_word(1,0)*(high_bands-1)
    noise = sbr_word(8,0) if temporal else field(7,5)
    data = '0'+'00001'+field(temporal,1)*2+'00'+env+noise+'0'+'1'+sized('10'+ps_bits)
    body = field(stage==0,1)+(sbr_header(2,True) if stage==0 else '')+data
    protected = stage==1
    body += '0'*(-(4+10*protected+len(body))%8)
    return packed(field(14 if protected else 13,4)+(field(crc(body),10) if protected else '')+body)

def main():
    binary = bytearray()
    sequences = []
    def store(bits, expected):
        start = len(sequences)%8
        encoded = packed('1'*start+bits+'10100101')
        frame = dict(offset=len(binary),bytes=len(encoded),start=start,bits=len(bits),expected=expected)
        binary.extend(encoded)
        return frame
    for slots in [24,30,32]:
        for mode in range(6):
            frames = [store(*ps(mode,(mode+3)%6,stage,slots)) for stage in range(3)]
            sequences.append(dict(name=f'mode-{mode}-slots-{slots}',slots=slots,frames=frames))
    for variable,count in [(False,4),(True,1),(True,2),(True,3),(True,4)]:
        sequences.append(dict(name=f'borders-{variable}-{count}',slots=30,
                              frames=[store(*ps(5,2,0,30,variable,count))]))
    for iid,icc in [(None,None),(0,None),(None,5)]:
        sequences.append(dict(name=f'disabled-{iid}-{icc}',slots=32,
                              frames=[store(*ps(iid,icc,0,phase=False))]))
    sequences.append(dict(name='escape-phase-disabled',slots=32,
                          frames=[store(*ps(0,0,0,phase=False,escaped=True))]))
    # Mode fields survive a header disabling a tool, then re-enable with dt=1.
    sequences.append(dict(name='mode-retained-across-disable',slots=32,frames=[
        store(*ps(5,5,0,phase=False)),store(*ps(None,None,0,phase=False)),
        store(*ps(5,5,1,phase=False,include_header=True))]))
    (DEST/'aac-ps-syntax.bin').write_bytes(binary)
    # Independent original video fixture: explicit AOT29 plus implicit AOT2
    # carrying the identical stereo extension payloads.
    blob = bytearray();frames=[];outer=[]
    for stage in range(3):
        bits,_ = ps(1,1,stage)
        raw = sbr(bits,stage)
        outer.append(raw.hex())
        data = packet(raw)
        frames.append(dict(offset=len(blob),bytes=len(data)))
        blob.extend(data)
    base = dict(slots=16,bands=64,frames=frames,pcm_offset=0,samples=12288)
    explicit = dict(base,asc=asc(24000,48000,16,'explicit',ps=True).hex())
    implicit = dict(base,asc=asc(24000,48000,16,'explicit',ps=False).hex())
    videos = [video_fixture([case],blob,channels=2 if name=='explicit' else 1,filename=f'he-aac-ps-{name}-synthetic.mp4')
              for name,case in [('explicit',explicit),('implicit',implicit)]]
    for video in videos:
        video.pop('pcm_offset')
        video.pop('samples')
        video['pcm_acceptance'] = 'pending owned PS synthesis'
    (DEST/'he-aac-ps-packets.bin').write_bytes(blob)
    manifest = dict(kind='original PS syntax acceptance and pending HE-AACv2 synthesis reproduction',
                    sha256=hashlib.sha256(binary).hexdigest(),sequences=sequences,
                    videos=videos,packet_frames=frames,sbr_payloads=outer)
    (DEST/'aac-ps-syntax.json').write_text(json.dumps(manifest,separators=(',',':'))+'\n')
    print(len(sequences),'original PS sequences,',len(binary),'syntax bytes;',len(blob),'AAC packet bytes')

if __name__=='__main__':
    main()
