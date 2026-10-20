"""Author SMPTE MIDI divisions versus equivalent independent PPQ tempos."""
import json,hashlib,struct
from pathlib import Path
from generate_smf_independent_tracks_regressions import smf
from generate_audio_reader_eof_regressions import generate as pcm
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/smf-smpte'
def vlq(n):
    out=[n&127];n>>=7
    while n:out.insert(0,(n&127)|128);n>>=7
    return bytes(out)
def meta_tempo(n):return b'\0\xff\x51\3'+n.to_bytes(3,'big')
def make(track,division):
    out=bytearray(smf(0,[track]));struct.pack_into('>H',out,12,division);return bytes(out)
def generate():
    files={'video.mkv':pcm()['video.mkv']};cases=[];invalid=[]
    for fps,tpf,ppq,tempo,ticks in [(24,10,120,500000,24),(25,40,500,500000,100),(29,100,3000,1001000,300),(30,10,150,500000,30)]:
        division=((256-fps)<<8)|tpf
        for mode in ['basic','tempo-ignored','fractional']:
            delta=1 if mode=='fractional' else ticks
            raw=b'\0\x90\x31\x40'+vlq(delta)+b'\x80\x31\0';control=meta_tempo(tempo)+raw
            if mode=='tempo-ignored':raw=meta_tempo(999999)+raw+meta_tempo(123456)
            rest=vlq(delta)+b'\x90\x32\x40'+vlq(delta)+b'\x80\x32\0\0\xff\x2f\0';raw+=rest;control+=rest
            label=f'{fps}-{mode}';files[label+'.mid']=make(raw,division);files[label+'-ppq.mid']=make(control,ppq);cases.append(dict(file=label+'.mid',control=label+'-ppq.mid',fps=fps,ticks_per_frame=tpf))
    for fps,tpf in [(23,10),(26,10),(28,10),(31,10),(24,0)]:
        label=f'invalid-{fps}-{tpf}.mid';files[label]=make(b'\0\xff\x2f\0',((256-fps)<<8)|tpf);invalid.append(label)
    files['manifest.json']=(json.dumps(dict(provenance='Twelve authored SMPTE24/25/29dropframe/30FPS basic,tempo-ignored andfractional cases compare independent PPQtempo ratios. Five illegal clock headers require refusal. Synthetic notes andnative VP9/PCM companion,no private media/encoder/network.',cases=cases,invalid=invalid,sha256={n:hashlib.sha256(v).hexdigest() for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
