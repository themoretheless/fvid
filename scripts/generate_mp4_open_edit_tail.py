"""Authored VP9/audio fragments with a declared packet-free final interval."""
from pathlib import Path
import hashlib,json,struct
from generate_vp9_chroma import atom,word
from generate_mp4_tfhd_defaults_regressions import atoms
ROOT=Path(__file__).resolve().parents[1]
BASE=ROOT/'tests/fixtures/playback-errors/mp4-open-ended-edit'
OUT=ROOT/'tests/fixtures/playback-errors/mp4-open-edit-tail'
def main():
    manifest=json.loads((BASE/'manifest-audio.json').read_text());files={};cases=[]
    for codec,rate in [('aac',48000),('pcm',8000),('alac',8000)]:
        files[f'{codec}-oracle.f32le']=(BASE/f'{codec}-oracle.f32le').read_bytes()
        for version in [0,1]:
            for trim in [0,1]:
                c=next(c for c in manifest['cases'] if c['file']==f'{codec}-v{version}-clock{rate}-trim{trim}-open.mp4')
                packet_frames=c['frames']//(3-trim);step=c['media_ticks']//(3-trim);end=4*step
                video=next(b for k,b in atoms(dict(atoms((BASE/c['file']).read_bytes()))[b'moov']) if k==b'trak' and b'vide' in b)
                stbl=dict(atoms(dict(atoms(dict(atoms(video))[b'mdia']))[b'minf']))[b'stbl']
                video_frames=int.from_bytes(dict(atoms(stbl))[b'stsz'][8:12],'big')
                for opened in [False,True]:
                    source=BASE/(f'{codec}-v{version}-clock{rate}-trim{trim}-'+('open.mp4' if opened else 'known.mp4'))
                    raw=source.read_bytes()
                    def rewrite(data,audio=False):
                        out=b''
                        for k,b in atoms(data):
                            selected=audio
                            if k==b'trak':selected=b'soun' in b
                            if k in [b'moov',b'trak',b'edts']:b=rewrite(b,selected)
                            elif k==b'elst' and audio and not opened:
                                b=word(version<<24,1)+(word(c['frames']+packet_frames,trim*step,65536) if version==0 else struct.pack('>QqI',c['frames']+packet_frames,trim*step,65536))
                            elif k==b'tkhd' and audio and not opened:b=b[:20]+word(c['frames']+packet_frames)+b[24:]
                            elif k==b'mvhd':b=b[:16]+word(max(int.from_bytes(b[16:20],'big'),c['frames']+packet_frames))+b[20:]
                            out+=atom(k,b)
                        return out
                    stamp=word(0,3*step) if version==0 else word(1<<24)+struct.pack('>Q',3*step)
                    tail=atom(b'moof',atom(b'mfhd',word(0,4))+atom(b'traf',atom(b'tfhd',word(0x030008,2,step))+atom(b'tfdt',stamp)))
                    name=f'{codec}-v{version}-trim{trim}-'+('open.mp4' if opened else 'known.mp4')
                    files[name]=rewrite(raw)+tail
                cases.append(dict(video_frames=video_frames,file=f'{codec}-v{version}-trim{trim}-open.mp4',known=f'{codec}-v{version}-trim{trim}-known.mp4',codec=codec,rate=rate,channels=c['channels'],frames=c['frames']+packet_frames,start_frames=c['start_frames'],tail_frames=packet_frames,end_media_ticks=end,edit_media_ticks=end-trim*step))
    refusals=[]
    for codec in ['aac','alac']:
        source=files[f'{codec}-v0-trim0-open.mp4'];parts=list(atoms(source));last=max(i for i,(k,b) in enumerate(parts) if k==b'mdat')
        assert parts[last-1][0]==b'moof'
        def broken_packet_size(data):
            out=b''
            for k,b in atoms(data):
                if k==b'traf':b=broken_packet_size(b)
                elif k==b'tfhd':
                    assert int.from_bytes(b[4:8],'big')==2 and len(b)==16
                    b=b[:-4]+word(1)
                out+=atom(k,b)
            return out
        parts[last-1]=(b'moof',broken_packet_size(parts[last-1][1]));parts[last]=(b'mdat',b'\xff')
        name=f'{codec}-invalid-last.mp4';files[name]=b''.join(atom(k,b) for k,b in parts);refusals.append(name)
    outside=[]
    for codec in ['aac','pcm','alac']:
        c=next(c for c in cases if c['file']==f'{codec}-v0-trim0-open.mp4')
        total=c['frames']+c['tail_frames']
        def outside_edit(data,audio=False):
            out=b''
            for k,b in atoms(data):
                selected=audio
                if k==b'trak':selected=b'soun' in b
                if k in [b'moov',b'trak',b'edts']:b=outside_edit(b,selected)
                elif k==b'elst' and audio:b=word(0,1,total,0,65536)
                elif k==b'tkhd' and audio:b=b[:20]+word(total)+b[24:]
                elif k==b'mvhd':b=b[:16]+word(max(int.from_bytes(b[16:20],'big'),total))+b[20:]
                out+=atom(k,b)
            return out
        name=f'{codec}-outside-tail.mp4';files[name]=outside_edit(files[c['known']]);outside.append(name)
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in files.items():(OUT/n).write_bytes(b)
    (OUT/'manifest.json').write_text(json.dumps(dict(provenance='Only existing authored VP9 and nonzero AAC pulse/analytic or integer PCM/ALAC controls. Append explicit empty-duration tfhd with no trun or mdat; V0/V1 elst/tfdt, zero or one-packet trim. Finite-edit equivalents and PCM plus zero-tail oracles. No private media/codec parameters, FFmpeg or network.',cases=cases,refusals=refusals,outside_tail=outside,sha256={n:hashlib.sha256(b).hexdigest() for n,b in files.items()}),indent=2)+'\n')
if __name__=='__main__':main()
