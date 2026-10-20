"""Author predictor-history seek probes using valid short coded runs."""
import json,hashlib
from pathlib import Path
from generate_coded_window_clock_regressions import generate as base,wave
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/coded-seek-history'
def generate():
    files=base();manifest=json.loads(files.pop('manifest.json'));cases=manifest['cases']
    extra=[('g722',0x28f,16000,4,1,8000,3072),('creative',0x200,8000,4,1,4000,3072)]
    for bits in [2,3,4,5]:extra.append((f'g726-{bits}',0x45,8000,bits,3 if bits==3 else 5 if bits==5 else 1,8000*bits//8,6144*bits//8))
    for name,tag,rate,bits,block,byte_rate,size in extra:
        payload=bytes((i*29+17)%256 for i in range(size));files[name+'.wav']=wave(tag,1,rate,bits,block,byte_rate,payload);cases.append(dict(name=name,kind='wav',file=name+'.wav',frames=6144,channels=1,grain=2048))
    files['manifest.json']=(json.dumps(dict(provenance='Fully authored short predictive MACE/GSM/TrueSpeech/G722/G726/Creative runs. No speech, private media or codec parameter copies. Continuous native PCM and existing rewind are controls for restoring state across coded window seeks. Short native VP9/PCM companion. No encoder/network.',cases=cases,sha256={n:hashlib.sha256(v).hexdigest()for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
