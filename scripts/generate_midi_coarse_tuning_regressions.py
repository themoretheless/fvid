"""Author RPN2coarse tuning against independent note numbers / pitch bends."""
import json,hashlib
from pathlib import Path
from generate_midi_bend_range_regressions import cc,bend,track,select as sensitivity
from generate_audio_reader_eof_regressions import generate as pcm
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/midi-coarse-tuning'
def select(ch,value):return [cc(ch,101,0),cc(ch,100,2),cc(ch,6,value)]
def events(messages,tick=0):return [(tick,m) for m in messages]
def generate():
    files={'video.mkv':pcm()['video.mkv']};cases=[]
    def add(name,actual,control,key=64,control_key=None,ch=0,onset=0,extra=()):
        notes=[(onset,bytes([0x90+ch,key,64])),(288,bytes([0x80+ch,key,0]))]
        k=key if control_key is None else control_key;reference=[(onset,bytes([0x90+ch,k,64])),(288,bytes([0x80+ch,k,0]))]
        for c,n in extra:
            pair=[(0,bytes([0x90+c,n,64])),(288,bytes([0x80+c,n,0]))];notes+=pair;reference+=pair
        files[name+'.mid']=track(actual+notes);files[name+'-reference.mid']=track(control+reference);cases.append(dict(file=name+'.mid',control=name+'-reference.mid'))
    for value in [0,52,63,64,65,76,127]:add('value-'+str(value),events(select(0,value)),[],control_key=value)
    for shift in [-12,12]:add('active-'+str(shift),events(select(0,64+shift),96),events(sensitivity(0,24))+[(96,bend(0,shift*4096//12))])
    add('future',events(select(0,76),96),[],control_key=76,onset=192)
    add('lsb-ignored',events(select(0,76)+[cc(0,38,127)]),[],control_key=76)
    add('lsb-only',events([cc(0,101,0),cc(0,100,2),cc(0,38,127)]),[])
    add('lsb-first-selector',events([cc(0,100,2),cc(0,101,0),cc(0,6,52)]),[],control_key=52)
    add('null-select',events(select(0,76)+[cc(0,101,127),cc(0,100,127),cc(0,6,40)]),[],control_key=76)
    add('nrpn-takeover',events(select(0,76)+[cc(0,99,0),cc(0,98,2),cc(0,6,40)]),[],control_key=76)
    add('rpn-reselect',events(select(0,76)+[cc(0,99,0),cc(0,98,2),cc(0,6,40),cc(0,100,2),cc(0,6,52)]),[],control_key=52)
    add('unselected-data',events([cc(0,6,76)]),[])
    add('unknown-rpn',events([cc(0,101,0),cc(0,100,3),cc(0,6,76)]),[])
    add('reset-preserves-tuning',events(select(0,76))+events([cc(0,121,0),cc(0,6,40)],96),[(96,cc(0,121,0))],control_key=76)
    add('bend-composition',events(select(0,76)+[bend(0,1024)]),[(0,bend(0,1024))],control_key=76)
    add('channel-isolation',events(select(15,76)),[],ch=15,control_key=76,extra=[(0,60)])
    add('return-center',events(select(0,76))+events(select(0,64),96),events(sensitivity(0,24))+[(0,bend(0,4096)),(96,bend(0,0))])
    files['manifest.json']=(json.dumps(dict(provenance='Twenty-two authored native MIDI RPN2controls versus independent transposed-note orRPN0bend performances. Initial/active/future notes,extremes,LSBignore,null/NRPN/reselection,CC121preservation,bendcomposition andchannel15isolation. Separate nativeVP9/PCMcompanion;no encoder/network/private media.',cases=cases,sha256={n:hashlib.sha256(v).hexdigest() for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
