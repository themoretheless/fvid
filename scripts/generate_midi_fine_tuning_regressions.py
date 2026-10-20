"""Author RPN1fine tuning versus independent one-semitone pitch bends."""
import json,hashlib
from pathlib import Path
from generate_midi_bend_range_regressions import cc,bend,track,select as sensitivity
from generate_midi_coarse_tuning_regressions import select as coarse,events
from generate_audio_reader_eof_regressions import generate as pcm
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/midi-fine-tuning'
def select(ch,value):return [cc(ch,101,0),cc(ch,100,1),cc(ch,6,value>>7),cc(ch,38,value&127)]
def reference(ch,value):return sensitivity(ch,1)+[bend(ch,value-8192)]
def generate():
    files={'video.mkv':pcm()['video.mkv']};cases=[]
    def add(name,actual,control,ch=0,key=64,control_key=None,onset=0,extra=()):
        notes=[(onset,bytes([0x90+ch,key,64])),(288,bytes([0x80+ch,key,0]))];k=key if control_key is None else control_key;other=[(onset,bytes([0x90+ch,k,64])),(288,bytes([0x80+ch,k,0]))]
        for c,n in extra:
            pair=[(0,bytes([0x90+c,n,64])),(288,bytes([0x80+c,n,0]))];notes+=pair;other+=pair
        files[name+'.mid']=track(actual+notes);files[name+'-bend.mid']=track(control+other);cases.append(dict(file=name+'.mid',control=name+'-bend.mid'))
    for value in [0,1,127,128,8191,8192,8193,8320,16382,16383]:add('value-'+str(value),events(select(0,value)),events(reference(0,value)))
    add('lsb-only',events([cc(0,101,0),cc(0,100,1),cc(0,38,1)]),events(reference(0,8193)))
    add('msb-only',events([cc(0,101,0),cc(0,100,1),cc(0,6,1)]),events(reference(0,128)))
    add('lsb-first-data',events([cc(0,100,1),cc(0,101,0),cc(0,38,1),cc(0,6,65)]),events(reference(0,8321)))
    add('active',events(select(0,12288),96),events(reference(0,8192))+[(96,bend(0,4096))])
    add('future',events(select(0,12288),96),events(reference(0,12288)),onset=192)
    for name,extra in [('null',[cc(0,101,127),cc(0,100,127),cc(0,6,0),cc(0,38,0)]),('nrpn',[cc(0,99,0),cc(0,98,1),cc(0,6,0),cc(0,38,0)])]:add(name,events(select(0,12288)+extra),events(reference(0,12288)))
    add('rpn-reselect',events(select(0,12288)+[cc(0,99,0),cc(0,98,1),cc(0,6,0),cc(0,100,1),cc(0,6,63),cc(0,38,127)]),events(reference(0,8191)))
    add('unknown-rpn',events([cc(0,101,0),cc(0,100,3),cc(0,6,0),cc(0,38,0)]),[])
    add('unselected-data',events([cc(0,6,0),cc(0,38,0)]),[])
    add('coarse-composition',events(coarse(0,76)+select(0,12288)),events(reference(0,12288)),control_key=76)
    add('bend-composition',events(select(0,12288)+[bend(0,1024)]),events(reference(0,14336)))
    add('range-composition',events(select(0,12288)+sensitivity(0,4)+[bend(0,512)]),events(reference(0,14336)))
    add('channel-isolation',events(select(15,12288)),events(reference(15,12288)),ch=15,extra=[(0,60)])
    add('reset-preserves-fine',events(select(0,12288)+[bend(0,1024)])+events([cc(0,121,0),cc(0,6,0),cc(0,38,0)],96),events(reference(0,14336))+events([cc(0,121,0),bend(0,4096)],96))
    add('return-center',events(select(0,12288))+events(select(0,8192),96),events(reference(0,12288))+[(96,bend(0,0))])
    add('sustained-key',events([cc(0,64,127)])+[(96,bytes([0x80,64,0]))]+events(select(0,12288),192),events([cc(0,64,127)]+reference(0,8192))+[(96,bytes([0x80,64,0])),(192,bend(0,4096))])
    files['manifest.json']=(json.dumps(dict(provenance='Twenty-seven authored RPN1fourteen-bit tuning controls against independent RPN0one-semitone bends;extremes/lowbits,byte-order/retention,active/future/sustainednotes,null/NRPN/coarse/bend/range composition,CC121,channel isolation. Separate nativeVP9/PCM,no encoder/network/private media.',cases=cases,sha256={n:hashlib.sha256(v).hexdigest() for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
