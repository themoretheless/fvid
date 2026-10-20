"""Author MIDI sequence-number versus track-name metadata controls."""
import json,hashlib
from pathlib import Path
from generate_smf_independent_tracks_regressions import smf
from generate_audio_reader_eof_regressions import generate as pcm
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/smf-sequence-name'
def generate():
    note=b'\0\x90\x31\x40\x60\x80\x31\0\0\xff\x2f\0';name=b'\0\xff\3\x0fSynthetic score';seq=b'\0\xff\0\2\0\1'
    files={'video.mkv':pcm()['video.mkv'],'audio-control.mid':smf(0,[note])};cases=[]
    specs=[('plain',name,'Synthetic score'),('sequence-before',seq+name,'Synthetic score'),('sequence-after',name+seq,'Synthetic score'),('sequence-only',seq,''),('sequence-extension',b'\0\xff\0\3\0\1\xff'+name,'Synthetic score'),('no-name',b'','')]
    for label,metadata,wanted in specs:
        file=label+'.mid';files[file]=smf(0,[metadata+note]);cases.append(dict(file=file,name=wanted))
    files['manifest.json']=(json.dumps(dict(provenance='Six authored MIDI sequence-number FF00andtextual sequence/track-name FF03cases. Binary sequence IDs must not become public tracklabels. Independent metadata-free100msnote PCMcontrol andnative VP9companion. No private media/encoder/network.',cases=cases,sha256={n:hashlib.sha256(v).hexdigest() for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
