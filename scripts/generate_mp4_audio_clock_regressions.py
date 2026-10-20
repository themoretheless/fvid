"""Authored MP4 audio media clocks independent of PCM sample frequency."""
import hashlib,json,struct
from pathlib import Path
from generate_mp4_enabled_track_regressions import boxes,canonical_movie,rewrite_track,sample_track
from generate_vp9_chroma import frame,mp4
from generate_audio_resample_window_fixture import atom,word
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-audio-clocks'
def shifted_audio(data,shift):
    output=b''
    for kind,body in boxes(data):
        if kind in [b'trak',b'mdia',b'minf']:body=shifted_audio(body,shift)
        elif kind==b'stbl':body+=atom(b'ctts',word(0x01000000,1,3,shift & 0xffffffff))
        output+=atom(kind,body)
    return output
def raw_alac(packet):
    values=struct.unpack('<160h',packet)
    bits='000'+'0000'+'0'*12+'1'+'00'+'1'+f'{len(values):032b}'+''.join(f'{v & 65535:016b}' for v in values)+'111'
    bits+='0'*(-len(bits)%8)
    return int(bits,2).to_bytes(len(bits)//8,'big')
def generate():
    files={};cases=[]
    base=dict(boxes(mp4([frame(8,'420',16,16)]*3,8,'420',(16,16),sync_samples=[1,2,3])))
    ftyp=atom(b'ftyp',base[b'ftyp']);movie=dict(boxes(base[b'moov']));packets=[];expected=bytearray()
    for index in range(3):
        values=[((index*160+i)*37)%32768-16384 for i in range(160)]
        packets.append(b''.join(struct.pack('<h',v) for v in values))
        expected.extend(b''.join(struct.pack('<f',v/32768.) for v in values))
    entry=bytearray(28);entry[6:8]=b'\0\1';entry[16:20]=struct.pack('>HH',1,16);entry[24:28]=word(8000<<16)
    cookie=bytearray(24);cookie[:4]=word(160);cookie[5:10]=bytes([16,40,10,14,1]);cookie[20:24]=word(8000)
    entries={b'sowt':entry,b'alac':entry+atom(b'alac',word(0)+cookie)}
    encoded={b'sowt':packets,b'alac':[raw_alac(packet) for packet in packets]}
    for codec,clock,negative in [(codec,clock,negative) for codec in [b'sowt',b'alac'] for clock,negative in [(8000,False),(1000,False),(44100,False),(48000,False),(90000,False),(1000,True)]]:
        video=atom(b'trak',rewrite_track(movie[b'trak'],1,3,len(ftyp)+8,False))
        audio=sample_track(2,3,b'soun',codec,entries[codec],encoded[codec],len(ftyp)+8+len(base[b'mdat']),clock,clock//50)
        if negative:audio=shifted_audio(audio,-clock//100)
        filename=('pcm' if codec==b'sowt' else 'alac')+f'-{clock}'+('-negative' if negative else '')+'.mp4'
        files[filename]=ftyp+atom(b'mdat',base[b'mdat']+b''.join(encoded[codec]))+atom(b'moov',atom(b'mvhd',canonical_movie(movie[b'mvhd'],3))+video+audio)
        cases.append({'file':filename,'codec':codec.decode(),'clock':clock,'rate':8000,'negative':negative})
    # Explicit seek regression videos reuse authored neutral frames and integer PCM.
    for codec in ['pcm', 'alac']:
        files[f'{codec}-seek-inside.mp4']=files[f'{codec}-1000.mp4']
    files['expected.f32le']=bytes(expected)
    files['negative-expected.f32le']=bytes(expected[80*4:])
    files['manifest.json']=(json.dumps({'provenance':'Authored60ms neutral16-square VP9 plus8kHz mono PCM/raw ALAC; five mdhd media clocks and signed-ctts negative-start companion. No encoder, private media or network.','cases':cases,'sha256':{k:hashlib.sha256(v).hexdigest() for k,v in files.items()}},indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
