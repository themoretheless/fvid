"""Own8k/16kPCM transition on an exact48kHz media grid,not a recovered clock."""
from pathlib import Path
import json,hashlib,struct
from generate_mp4_recovered_silence_seek import generate as recovered
from generate_mp4_tfhd_defaults_regressions import atoms
from generate_vp9_chroma import atom,word
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-native-grid-silence-seek'
def generate():
    source=recovered();case=next(c for c in json.loads(source['manifest.json'])['cases']if c.get('pcm',False));data=source[case['file']]
    def rewrite(data,audio=False):
        out=[]
        for k,b in atoms(data):
            if k==b'trak':b=rewrite(b,b'soun'in b)
            elif k in [b'moov',b'mdia',b'minf',b'stbl',b'edts']:b=rewrite(b,audio)
            elif audio and k==b'mdhd':b=b[:12]+word(48000,762)+b[20:]
            elif audio and k==b'stts':b=word(0,2,1,384,2,189)
            elif audio and k==b'ctts':b=word(0,3,1,0,1,576,1,867)
            elif audio and k==b'elst':b=word(0,2,1,0,65536,1,1152,65536)
            out.append(atom(k,b))
        return b''.join(out)
    name='changing-pcm-native-grid.mp4';files={name:rewrite(data)};case.update(file=name,scale=48000,first_duration=384,descriptions=2)
    def unused(data,audio=False):
        out=[]
        for k,b in atoms(data):
            if k==b'trak':b=unused(b,b'soun'in b)
            elif k in [b'moov',b'mdia',b'minf',b'stbl']:b=unused(b,audio)
            elif audio and k==b'stsd':
                entries=list(atoms(b[8:]));b=word(0,3)+b[8:]+atom(b'zzzz',entries[0][1])
            out.append(atom(k,b))
        return b''.join(out)
    extra='changing-pcm-unused-entry.mp4';files[extra]=unused(files[name]);cases=[case,dict(case,file=extra,descriptions=3)]
    # A valid final packet exercises EOF rounding with no later source packet.
    def whole(data,audio=False):
        out=[]
        for k,b in atoms(data):
            if k==b'trak':
                is_audio=b'soun'in b;b=whole(b,is_audio)
            elif k in [b'moov',b'mdia',b'minf',b'stbl',b'edts']:b=whole(b,audio)
            elif k==b'mvhd':b=b[:12]+word(16000,960)+b[20:]
            elif k==b'tkhd':b=b[:20]+word(63 if audio else 960)+b[24:]
            elif audio and k==b'elst':b=word(0,1,63,1440,65536)
            out.append(atom(k,b))
        return b''.join(out)
    whole_name='whole-final-packet.mp4';files[whole_name]=whole(files[name]);queries=[]
    for ns in [0,1,500001,3937499,3937500,3937501,18446744073709551615999999999]:
        first=min(63,(ns*16000+10**9-1)//10**9);queries.append(dict(target_ns=str(ns),first=first,anchor_ns=first*10**9//16000))
    cases.append(dict(case,file=whole_name,whole=True,frames=63,duration_ns=3937500,queries=queries))
    files['manifest.json']=(json.dumps(dict(provenance='Own neutral VP9 and analytic0.125PCM with8k/16k setups. Exact packet durations384/189/189 on48000Hz grid;no quantization recovery. Finite1ms coded+1ms gap edits. No private input,FFmpeg or network.',cases=cases,sha256={n:hashlib.sha256(b).hexdigest()for n,b in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
