"""Author SMF extension chunks without counting them as musical tracks."""
import json,hashlib,struct
from pathlib import Path
from generate_smf_clock_range_regressions import midi
from generate_audio_reader_eof_regressions import generate as pcm
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/smf-unknown-chunks'
def chunk(tag,payload):return tag+struct.pack('>I',len(payload))+payload
def generate():
    files={'video.mkv':pcm()['video.mkv']};cases=[]
    one=midi(b'\0\x90\x45\x40\x60\x80\x45\0\0\xff\x2f\0',480)
    header=one[:14];note=one[14:];empty=chunk(b'MTrk',b'\0\xff\x2f\0')
    two=header[:8]+b'\0\1\0\2'+header[12:]+empty+note
    variants={'plain-single':one,'plain-two':two,'odd-extension-before':header+chunk(b'Xtra',b'abc')+note,'empty-extension-before':header+chunk(b'Xtra',b'')+note,'extensions-between':two[:14]+empty+chunk(b'Xtra',b'xyz')+chunk(b'JUNK',bytes(range(17)))+note,'many-extensions':header+chunk(b'Xtra',b'x')*32+note}
    for n,b in variants.items():files[n+'.mid']=b;cases.append(dict(name=n,file=n+'.mid',bad=False))
    files['truncated-extension.mid']=header+b'Xtra'+struct.pack('>I',99)+b'abc';cases.append(dict(name='truncated-extension',file='truncated-extension.mid',bad=True))
    files['manifest.json']=(json.dumps(dict(provenance='Authored100ms single note with bounded unknown SMF chunks before/between actual tracks, odd and zero lengths, truncated control. Separate authored native VP9/PCM video. No encoder/network/private media.',cases=cases,sha256={n:hashlib.sha256(v).hexdigest() for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
