#!/usr/bin/env python3
"""Original complete HE-AAC raw_data_blocks, silent LC core + authored SBR.
No private parameters/audio, encoder/decoder, FFmpeg, libav or network.
"""
from pathlib import Path
import json,hashlib,struct
from generate_aac_sbr_data_fixtures import field
from generate_aac_sbr_dsp_fixtures import payload
DEST=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
def packed(text):
    text+='0'*(-len(text)%8)
    return bytes(int(text[i:i+8],2) for i in range(0,len(text),8))
def frequency(rate):
    rates=[96000,88200,64000,48000,44100,32000,24000,22050,16000,12000,11025,8000,7350]
    return field(rates.index(rate),4) if rate in rates else '1111'+field(rate,24)
def asc(core,out,slots,signalling,ps=False):
    ga=field(slots==15,1)+'00'
    if signalling=='explicit':
        return packed(field(29 if ps else 5,5)+frequency(core)+'0001'+frequency(out)+field(2,5)+ga)
    return packed(field(2,5)+frequency(core)+'0001'+ga+field(0x2b7,11)+field(5,5)+'1'+frequency(out)+(field(0x548,11)+'1' if ps else ''))
def packet(raw):
    # SCE tag0, gain100, ONLY_LONG sine, max_sfb=0, no prediction/pulse/TNS/gain.
    core='000'+'0000'+field(100,8)+'0'+'00'+'0'+'000000'+'0'+'000'
    count=len(raw)
    fill='110'+(field(count,4) if count<15 else '1111'+field(count-14,8))
    text=core+fill+''.join(field(b,8) for b in raw)+'111'
    return packed(text)
def boxes(data):
    offset=0
    while offset<len(data):
        size,tag=struct.unpack_from('>I4s',data,offset)
        assert size>=8 and offset+size<=len(data)
        yield tag,data[offset+8:offset+size]
        offset+=size

def box(tag,body): return struct.pack('>I4s',len(body)+8,tag)+body

def video_fixture(cases,blob,channels=1,filename="he-aac-sbr-synthetic.mp4"):
    # Existing authored AVC video is unchanged; original moov becomes equal-size
    # free, preserving its sample offsets. The audio template supplies container
    # structure only: original esds/sample tables/edits are all replaced.
    video=(DEST/'avc-slice-lists-temporal.mp4').read_bytes()
    template=(DEST.parent/'audio/aac-native-edit.m4a').read_bytes()
    case=next((c for c in cases if c['slots']==16 and c['bands']==64), cases[0])
    frame_samples=case.get('container_frame_samples',2*64*case['slots'])
    container_rate=case.get('container_rate',48000)
    durations=case.get('durations',[frame_samples]*len(case['frames']))
    assert len(durations)==len(case['frames']) and all(0<d<=frame_samples for d in durations)
    duration=sum(durations)
    packets=[blob[f['offset']:f['offset']+f['bytes']] for f in case['frames']]
    config=bytes.fromhex(case['asc'])
    def descriptor(tag,data):
        assert len(data)<128
        return bytes([tag,len(data)])+data
    esds=bytes(4)+descriptor(3,b'\x00\x02\x00'+descriptor(4,b'\x40\x15'+bytes(11)+descriptor(5,config))+descriptor(6,b'\x02'))
    original_moov=next(p for t,p in boxes(video) if t==b'moov')
    movie_header=next(p for t,p in boxes(original_moov) if t==b'mvhd')
    assert movie_header[0]==0
    movie_rate=struct.unpack_from('>I',movie_header,12)[0]
    audio_duration=(duration*movie_rate+container_rate-1)//container_rate
    offset=len(video)+8
    def rewrite(tag,body):
        if tag==b'edts':return b''
        if tag==b'tkhd':
            body=bytearray(body);assert body[0]==0;struct.pack_into('>I',body,12,2);struct.pack_into('>I',body,20,audio_duration);body=bytes(body)
        elif tag==b'mdhd':
            body=bytearray(body);assert body[0]==0;struct.pack_into('>II',body,12,container_rate,duration);body=bytes(body)
        elif tag==b'stsd':
            entries=list(boxes(body[8:]));assert len(entries)==1 and entries[0][0]==b'mp4a'
            entry=bytearray(entries[0][1][:28]);struct.pack_into('>H',entry,16,channels);struct.pack_into('>I',entry,24,container_rate<<16)
            body=bytes(4)+struct.pack('>I',1)+box(b'mp4a',bytes(entry)+box(b'esds',esds))
        elif tag==b'stts':
            body=(bytes(4)+struct.pack('>III',1,len(packets),frame_samples) if all(d==frame_samples for d in durations) else
                  bytes(4)+struct.pack('>I',len(durations))+b''.join(struct.pack('>II',1,d) for d in durations))
        elif tag==b'stsc':body=bytes(4)+struct.pack('>IIII',1,1,len(packets),1)
        elif tag==b'stsz':body=bytes(4)+struct.pack('>II',0,len(packets))+b''.join(struct.pack('>I',len(p)) for p in packets)
        elif tag==b'stco':body=bytes(4)+struct.pack('>II',1,offset)
        elif tag in [b'trak',b'mdia',b'minf',b'stbl']:
            allowed={b'trak':{b'tkhd',b'mdia'},b'mdia':{b'mdhd',b'hdlr',b'minf'},b'minf':{b'smhd',b'dinf',b'stbl'},b'stbl':{b'stsd',b'stts',b'stsc',b'stsz',b'stco'}}[tag]
            body=b''.join(rewrite(t,p) for t,p in boxes(body) if t in allowed)
        return box(tag,body)
    template_moov=next(p for t,p in boxes(template) if t==b'moov')
    audio=next(p for t,p in boxes(template_moov) if t==b'trak')
    audio=rewrite(b'trak',audio)
    original_moov=next(p for t,p in boxes(video) if t==b'moov')
    original_children=list(boxes(original_moov))
    header=bytearray(next(p for t,p in original_children if t==b'mvhd'));header[-4:]=struct.pack('>I',3);struct.pack_into('>I',header,16,max(struct.unpack_from('>I',header,16)[0],audio_duration))
    movie=box(b'mvhd',header)+b''.join(box(t,p) for t,p in original_children if t==b'trak')+audio
    data=b''.join(box(b'free' if t==b'moov' else t,p) for t,p in boxes(video))+box(b'mdat',b''.join(packets))+box(b'moov',movie)
    (DEST/filename).write_bytes(data)
    return dict(file=filename,sha256=hashlib.sha256(data).hexdigest(),
        video_seed_sha256=hashlib.sha256(video).hexdigest(),asc=case['asc'],pcm_offset=case['pcm_offset'],samples=case['samples'])

def main():
    source=json.loads((DEST/'aac-sbr-dsp-oracles.json').read_text())
    binary=(DEST/'aac-sbr-dsp-syntax.bin').read_bytes();blob=bytearray();cases=[]
    for reference in source['cases']:
        for signalling in ['explicit','sync']:
            config=asc(24000,48000 if reference['bands']==64 else 24000,reference['slots'],signalling)
            frames=[]
            for f in reference['frames']:
                raw=binary[f['offset']:f['offset']+f['byte_length']]
                data=packet(raw);frames.append(dict(offset=len(blob),bytes=len(data)));blob.extend(data)
            cases.append(dict(slots=reference['slots'],bands=reference['bands'],signalling=signalling,
                asc=config.hex(),frames=frames,pcm_offset=reference['pcm_offset'],samples=reference['samples']))
    (DEST/'he-aac-sbr-packets.bin').write_bytes(blob)
    (DEST/'he-aac-sbr-packets.json').write_text(json.dumps(dict(kind='original complete LC+SBR raw_data_blocks',
        video=video_fixture(cases,blob),sha256=hashlib.sha256(blob).hexdigest(),sbr_source_sha256=hashlib.sha256(binary).hexdigest(),cases=cases),separators=(',',':'))+'\n')
    print(len(cases),'three-packet HE-AAC sequences;',len(blob),'bytes')
if __name__=='__main__':main()
