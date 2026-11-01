"""Own LC1024/Main960 transition with1ms finite edits on an exact media grid."""
from pathlib import Path
from fractions import Fraction as F
import json,hashlib
from generate_mp4_aac_transitions import generate as transitions,SCALE
from generate_mp4_variable_rate_edits import edit_movie
from generate_mp4_tfhd_defaults_regressions import atoms
from generate_vp9_chroma import atom,word
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-aac-native-grid-seek'
def generate():
    source=transitions();files={'reference-0.f32le':source['control-0.f32le'][:48*2*4],'reference-1.f32le':source['control-1.f32le'][:32*4]};cases=[]
    queries=[]
    for ns in [0,1,500001,999999,1000000,1000001,1500001,1999999,2000000,18446744073709551615999999999]:
        wanted=F(ns,10**9);parts=[]
        for control,rate,channels,count,start in [(0,48000,2,48,F(0)),(1,32000,1,32,F(1,1000))]:
            first=min(count,max(0,((wanted-start)*rate).__ceil__()))
            if first<count:parts.append(dict(control=control,rate=rate,channels=channels,first=first,last=count,start_ns=int(start*10**9)))
        at=F(parts[0]['start_ns'],10**9)+F(parts[0]['first'],parts[0]['rate'])if parts else F(2,1000)
        queries.append(dict(target_ns=str(ns),parts=parts,anchor_ns=at.numerator*10**9//at.denominator))
    for stem in ['changes','third','fragmented']:
        name=stem+'-two-edits.mp4';files[name]=edit_movie(source[stem+'.mp4'],[(1,0),(1,SCALE*64//1000)]);cases.append(dict(file=name,queries=queries))
    # Unused unknown headers must not select the timing path.
    for prepend in [False,True]:
        def catalog(data,audio=False):
            out=[]
            for k,b in atoms(data):
                if k==b'trak':b=catalog(b,b'soun'in b)
                elif k in [b'moov',b'mdia',b'minf',b'stbl']:b=catalog(b,audio)
                elif audio and k==b'stsd':
                    entries=list(atoms(b[8:]));opaque=atom(b'zzzz',entries[0][1][:28]);b=word(0,4)+(opaque+b[8:] if prepend else b[8:]+opaque)
                elif audio and prepend and k==b'stsc':
                    b=bytearray(b)
                    for at in range(16,len(b),12):b[at:at+4]=word(int.from_bytes(b[at:at+4],'big')+1)
                out.append(atom(k,b))
            return b''.join(out)
        name='unused-'+('first'if prepend else'last')+'.mp4';files[name]=catalog(files['changes-two-edits.mp4']);cases.append(dict(file=name,queries=queries,duration_ns=2000000))
    # A complete final access unit tests successful rounding to the true end.
    source_movie=dict(atoms(source['changes.mp4']));audio=next(b for k,b in atoms(source_movie[b'moov'])if k==b'trak'and b'soun'in b);mdia=dict(atoms(audio));tables=dict(atoms(dict(atoms(dict(atoms(mdia[b'mdia']))[b'minf']))[b'stbl']));durations=[]
    import struct
    for count,delta in struct.iter_unpack('>II',tables[b'stts'][8:]):durations.extend([delta]*count)
    last=sum(durations[:-1]);name='whole-last-aac.mp4';files[name]=edit_movie(source['changes.mp4'],[(1024,last)],clock=48000);files['reference-2.f32le']=source['control-0.f32le'][2048*2*4:3072*2*4];whole=[]
    for ns in [0,1,500001,21333332,21333333,21333334,18446744073709551615999999999]:
        first=min(1024,(ns*48000+10**9-1)//10**9);parts=[]if first==1024 else[dict(control=2,rate=48000,channels=2,first=first,last=1024,start_ns=0)];whole.append(dict(target_ns=str(ns),parts=parts,anchor_ns=first*10**9//48000))
    cases.append(dict(file=name,queries=whole,duration_ns=1024*10**9//48000))
    for c in cases:c.setdefault('duration_ns',2000000)
    files['manifest.json']=(json.dumps(dict(provenance='Own scalar AAC pulse spectra and independent IMDCT/overlap references. LC1024 stereo48k then Main960 mono32k;finite1ms edits,classic/reordered catalog/fragmented variants,16 neutral VP9 frames. Exact14112000Hz clock,no recovery. No private input,FFmpeg or network.',scale=SCALE,cases=cases,sha256={n:hashlib.sha256(b).hexdigest()for n,b in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
