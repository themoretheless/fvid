#!/usr/bin/env python3
"""Own hybrid VP9 + AAC/PCM/ALAC fragment controls for exact open media edits."""
from pathlib import Path
import struct,json,hashlib
from generate_vp9_chroma import atom,word
from generate_mp4_tfhd_defaults_regressions import atoms
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'tests/fixtures/playback-errors/mp4-open-ended-edit'
def generate():
    files={};cases=[]
    for codec,source,oracle,rate,channels in [('aac','mp4-aac-transitions/control-0.mp4','mp4-aac-transitions/control-0.f32le',48000,2),('pcm','mp4-audio-clocks/pcm-8000.mp4','mp4-audio-clocks/expected.f32le',8000,1),('alac','mp4-audio-clocks/alac-8000.mp4','mp4-audio-clocks/expected.f32le',8000,1)]:
        base=ROOT/'tests/fixtures/playback-errors';raw=(base/source).read_bytes();top=dict(atoms(raw));movie=list(atoms(top[b'moov']))
        audio=next(b for k,b in movie if k==b'trak' and b'soun' in b)
        mdia=dict(atoms(audio))[b'mdia'];mh=dict(atoms(mdia))[b'mdhd'];scale=int.from_bytes(mh[12:16],'big');total=int.from_bytes(mh[16:20],'big')
        tables=dict(atoms(dict(atoms(dict(atoms(mdia))[b'minf']))[b'stbl']))
        stsz=tables[b'stsz'];assert int.from_bytes(stsz[4:8],'big')==0
        count=int.from_bytes(stsz[8:12],'big');assert count==3
        sizes=struct.unpack('>III',stsz[12:]);offsets=struct.unpack('>'+str((len(tables[b'stco'])-8)//4)+'I',tables[b'stco'][8:]);sc=[struct.unpack('>III',tables[b'stsc'][i:i+12]) for i in range(8,len(tables[b'stsc']),12)]
        packets=[];sample=0
        for chunk,offset in enumerate(offsets,1):
            units=next(e[1] for e in reversed(sc) if e[0]<=chunk)
            for _ in range(units):packets.append(raw[offset:offset+sizes[sample]]);offset+=sizes[sample];sample+=1
        timing=[]
        for i in range(8,len(tables[b'stts']),8):
            n,d=struct.unpack('>II',tables[b'stts'][i:i+8]);timing.extend([d]*n)
        assert len(packets)==len(timing)==3 and len(set(timing))==1
        packet_frames=timing[0]*rate//scale;assert packet_frames*scale==timing[0]*rate
        files[f'{codec}-oracle.f32le']=(base/oracle).read_bytes()
        brand=bytearray(top[b'ftyp']);brand[:4]=b'iso5';brand[8:12]=b'iso5';prefix=atom(b'ftyp',brand)+atom(b'mdat',top[b'mdat'])
        old_movie=dict(movie)[b'mvhd'];old_scale=int.from_bytes(old_movie[12:16],'big');old_duration=int.from_bytes(old_movie[16:20],'big')
        for version in [0,1]:
            for movie_scale in [rate,1001,25]:
                for trim in [0,1]:
                    for opened in ([False,True] if movie_scale==rate else [True]):
                        start=trim*timing[0];frames=(3-trim)*packet_frames;duration=0 if opened else frames
                        row=word(duration,start,0x10000) if version==0 else struct.pack('>QqI',duration,start,0x10000)
                        edit=atom(b'edts',atom(b'elst',word(version<<24,1)+row))
                        def rewrite(data):
                            out=b''
                            for k,b in atoms(data):
                                if k in [b'mdia',b'minf',b'stbl']:b=rewrite(b)
                                elif k in [b'stts',b'stsc',b'stco']:b=word(0,0)
                                elif k==b'stsz':b=word(0,0,0)
                                elif k==b'tkhd':b=b[:20]+word(duration)+b[24:]
                                out+=atom(k,b)
                            return out
                        moov=b''
                        for k,b in movie:
                            if k==b'mvhd':b=b[:12]+word(movie_scale,(old_duration*movie_scale+old_scale-1)//old_scale)+b[20:]
                            elif k==b'trak' and b'soun' in b:b=rewrite(b)+edit
                            elif k==b'trak':
                                b=b''.join(atom(t, d[:20]+word((int.from_bytes(d[20:24],'big')*movie_scale+old_scale-1)//old_scale)+d[24:] if t==b'tkhd' else d) for t,d in atoms(b))
                            moov+=atom(k,b)
                        result=prefix+atom(b'moov',moov+atom(b'mvex',atom(b'trex',word(0,2,1,0,0,0))))
                        dts=0
                        for i,(packet,d) in enumerate(zip(packets,timing)):
                            def fragment(offset):return atom(b'moof',atom(b'mfhd',word(0,i+1))+atom(b'traf',atom(b'tfhd',word(0x020018,2,d,len(packet)))+atom(b'tfdt',word(0,dts))+atom(b'trun',word(1,1,offset))))
                            result+=fragment(len(fragment(0))+8)+atom(b'mdat',packet);dts+=d
                        name=f'{codec}-v{version}-clock{movie_scale}-trim{trim}-'+('open.mp4' if opened else 'known.mp4');files[name]=result
                        cases.append(dict(file=name,codec=codec,rate=rate,channels=channels,frames=frames,start_frames=trim*packet_frames,media_ticks=(3-trim)*timing[0],open=opened,oracle=f'{codec}-oracle.f32le'))
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in files.items():(OUT/n).write_bytes(b)
    (OUT/'manifest-audio.json').write_text(json.dumps({'provenance':'Owned VP9 plus nonzero AAC pulse/analytic PCM or integer PCM fixtures; known/open single V0/V1 edits with exact media endpoints. No private inputs, FFmpeg or network.','cases':cases,'sha256':{n:hashlib.sha256(b).hexdigest() for n,b in files.items()}},indent=2)+'\n')
if __name__=='__main__':generate()
