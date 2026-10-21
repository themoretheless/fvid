#!/usr/bin/env python3
"""Author checksum-valid FLAC frames with contradictory coded clocks."""
import hashlib,json,struct
from pathlib import Path
from generate_flac_coded_numbers_regressions import frame,stream
from generate_audio_reader_eof_regressions import generate as owned
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/flac-number-timeline'
def generate():
    files={'video.mkv':owned()['video.mkv']};bad=[];controls=[]
    def add(name,sizes,variable,numbers=None,strategies=None,broken=False):
        variables=[variable]*len(sizes) if strategies is None else strategies
        starts=[];total=0
        for size in sizes:starts.append(total);total+=size
        expected_numbers=starts if variable else list(range(len(sizes)))
        if numbers is None:numbers=expected_numbers
        levels=[4096,-4096,8192];packets=[];spec=[]
        for i,(n,size,v) in enumerate(zip(numbers,sizes,variables)):
            coded=frame(n,size,v,levels[i%3]);packet=f'{name}-{i}.frame';files[packet]=coded;packets.append(coded)
            spec.append(dict(file=packet,number=n,variable=v,frames=size,level=levels[i%3]))
        filename=name+'.flac';container=bytearray(stream(packets,sizes,8000))
        minimum=min(sizes[:-1] or sizes);maximum=max(sizes)
        if variable and minimum==maximum:minimum=max(16,minimum//2)
        struct.pack_into('>HH',container,8,minimum,maximum)
        raw_pcm=b''.join(struct.pack('<h',levels[i%3])*size for i,size in enumerate(sizes))
        container[26:42]=hashlib.md5(raw_pcm).digest()
        files[filename]=bytes(container)
        literal=name+'.f32le';files[literal]=b''.join(struct.pack('<f',levels[i%3]/32768.)*size for i,size in enumerate(sizes))
        target=bad if broken else controls;target.append(dict(file=filename,expected=literal,frames=total,packets=spec))
    for name,variable,numbers in [
        ('fixed-first',False,[1,2,3]),('fixed-gap',False,[0,2,3]),('fixed-duplicate',False,[0,1,1]),('fixed-backward',False,[0,1,0]),
        ('variable-first',True,[64,128,192]),('variable-gap',True,[0,128,192]),('variable-overlap',True,[0,32,128]),('variable-backward',True,[0,64,0]),('variable-wide-gap',True,[0,68719476735,128]),
    ]:add(name,[64]*3,variable,numbers,broken=True)
    add('mixed-fixed-variable',[64]*3,False,[0,64,2],[False,True,False],True)
    add('mixed-variable-fixed',[64]*3,True,[0,1,128],[True,False,True],True)
    add('fixed-control',[64,64,7],False)
    add('variable-control',[32,64,7],True)
    add('fixed-wide-control',[64]*130,False)
    add('variable-wide-control',[64]*4,True)
    files['manifest.json']=(json.dumps(dict(provenance='Eleven owned checksum-valid constant16bit FLAC streams violate frame/sample continuity or change blocking strategy. All subframes still decode to independent literal PCM,isolating missing file-clock validation. Four legal controls include changing variable blocks,short final blocks and multi-byte coded numbers. Owned VP9/PCM companion;pure Python,no encoder,network or private media.',reference='https://www.rfc-editor.org/rfc/rfc9639.html#section-9.1.5',cases=bad,controls=controls,sha256={n:hashlib.sha256(b).hexdigest() for n,b in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
