"""Author held-to-EOF notes and independently explicit note-off controls."""
import json,hashlib,struct
from pathlib import Path
from generate_smf_clock_range_regressions import midi
from generate_xm_sample_grid_regressions import generate as grid
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/synth-tail-release'
def generate():
    authored=grid();files={'video.mkv':authored['video.mkv']}
    for held in [True,False]:
        files['midi-'+('held' if held else 'released')+'.mid']=midi(b'\0\x90\x45\x40\x60'+(b'' if held else b'\x80\x45\0\0')+b'\xff\x2f\0',480)
    b=authored['held-125.xm'];at=60+struct.unpack_from('<I',b,60)[0];end=at+9+struct.unpack_from('<H',b,at+7)[0]
    for held in [True,False]:
        cells=bytes([58,1,64,0,0])+bytes(15)
        if not held:cells+=bytes([97,0,0,0,0])*2
        rows=2 if held else 3
        files['xm-'+('held' if held else 'released')+'.xm']=b[:at]+struct.pack('<IBHH',9,0,rows,len(cells))+cells+b[end:]
    cases=[dict(kind=k,held=k+'-held.'+ext,control=k+'-released.'+ext) for k,ext in [('midi','mid'),('xm','xm')]]
    files['manifest.json']=(json.dumps(dict(provenance='Authored held-to-EOF notes versus explicit note-off controls. MIDI same100ms score; XM explicit keyoff row at240ms supplies independent release PCM for held source tail. Repository synthetic looping samples only; separate authored native VP9/PCM video. No encoder/private media/network.',cases=cases,sha256={n:hashlib.sha256(v).hexdigest() for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
