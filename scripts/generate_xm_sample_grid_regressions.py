"""Author held-note rows with fractional44.1kHz sample endpoints."""
import json,hashlib,struct
from pathlib import Path
from generate_audio_reader_eof_regressions import generate as pcm
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'tests/fixtures/playback-errors/xm-sample-grid'
def generate():
    original=(ROOT/'tests/fixtures/xm/two-notes.xm').read_bytes()
    at=60+struct.unpack_from('<I',original,60)[0]
    end=at+struct.unpack_from('<I',original,at)[0]+struct.unpack_from('<H',original,at+7)[0]
    rows=31;cells=bytes([58,1,64,0,0])+bytes(5)+bytes((rows-1)*2*5)
    pattern=struct.pack('<IBHH',9,0,rows,len(cells))+cells
    files={'video.mkv':pcm()['video.mkv']};cases=[]
    for bpm in [125,127,199,255]:
        data=bytearray(original[:at]+pattern+original[end:]);struct.pack_into('<H',data,78,bpm)
        instrument=at+len(pattern);sample=instrument+struct.unpack_from('<I',data,instrument)[0]
        data[sample+14] |= 1 # Enable the authored sample's existing full-length forward loop.
        name=f'held-{bpm}.xm';files[name]=bytes(data);cases.append(dict(file=name,bpm=bpm,rows=rows))
    files['manifest.json']=(json.dumps(dict(provenance='Authored31row held note using repository synthetic looping sample/instrument, four tempos including integer control. Separate authored native VP9/PCM companion. No encoder/private media/network.',cases=cases,sha256={n:hashlib.sha256(v).hexdigest() for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
