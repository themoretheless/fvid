"""Author native MIDI RPN0bend sensitivity versus equivalent default-range bends."""
import json,hashlib
from pathlib import Path
from generate_smf_independent_tracks_regressions import smf
from generate_smf_smpte_regressions import vlq
from generate_audio_reader_eof_regressions import generate as pcm
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/midi-bend-range'
def cc(ch,n,v):return bytes([0xb0+ch,n,v])
def bend(ch,delta):
    value=8192+delta;return bytes([0xe0+ch,value&127,value>>7])
def select(ch,semi,cents=0):return [cc(ch,101,0),cc(ch,100,0),cc(ch,6,semi),cc(ch,38,cents)]
def track(events):
    out=bytearray();last=0
    for tick,message in sorted(events,key=lambda e:e[0]):out+=vlq(tick-last)+message;last=tick
    out+=vlq(288-last)+b'\xff\x2f\0';return smf(0,[bytes(out)])
def generate():
    files={'video.mkv':pcm()['video.mkv']};cases=[]
    def add(label,actual,control,ch=0,key=69,extra_voices=(),control_key=None):
        notes=[]
        for c,k in [(ch,key),*extra_voices]:notes += [(0,bytes([0x90+c,k,64])),(288,bytes([0x80+c,k,0]))]
        files[label+'.mid']=track(actual+notes);reference_notes=notes if control_key is None else [(0,bytes([0x90+ch,control_key,64])),(288,bytes([0x80+ch,control_key,0]))]
        files[label+'-default.mid']=track(control+reference_notes);cases.append(dict(file=label+'.mid',control=label+'-default.mid'))
    for semi in [0,1,2,3,4,12,24]:add('range-'+str(semi),[(0,m) for m in select(0,semi)]+[(0,bend(0,512))],[(0,bend(0,semi*256))])
    for semi,cents in [(0,50),(2,25),(12,75)]:add(f'cents-{semi}-{cents}',[(0,m) for m in select(0,semi,cents)]+[(0,bend(0,512))],[(0,bend(0,semi*256+cents*256//100))])
    add('active-retune',[(0,bend(0,512))]+[(96,m) for m in select(0,12)],[(0,bend(0,512)),(96,bend(0,3072))])
    add('lsb-first',[(0,m) for m in [cc(0,100,0),cc(0,101,0),cc(0,38,50),cc(0,6,2),bend(0,512)]],[(0,bend(0,640))])
    add('null-select',[(0,m) for m in select(0,12)+[cc(0,101,127),cc(0,100,127),cc(0,6,24),bend(0,512)]],[(0,bend(0,3072))])
    add('nrpn-takeover',[(0,m) for m in select(0,12)+[cc(0,99,0),cc(0,98,0),cc(0,6,24),bend(0,512)]],[(0,bend(0,3072))])
    add('rpn-reselect-after-nrpn',[(0,m) for m in select(0,12)+[cc(0,99,0),cc(0,98,0),cc(0,6,24),cc(0,100,0),cc(0,6,4),bend(0,512)]],[(0,bend(0,1024))])
    add('unknown-rpn',[(0,m) for m in [cc(0,101,0),cc(0,100,3),cc(0,6,24),bend(0,512)]],[(0,bend(0,512))])
    add('reset-selectors-preserve-range',[(0,m) for m in select(0,12)+[bend(0,512)]]+[(96,m) for m in [cc(0,121,0),cc(0,6,24),bend(0,512)]],[(0,bend(0,3072)),(96,cc(0,121,0)),(96,bend(0,3072))])
    add('channel-isolation',[(0,m) for m in select(15,12)+[bend(15,512),bend(0,512)]],[(0,bend(15,3072)),(0,bend(0,512))],15,72,[(0,69)])
    add('range127-small-bend',[(0,m) for m in select(0,127)]+[(0,bend(0,16))],[(0,bend(0,1016))])
    for semi,cents,delta in [(12,0,-512),(2,50,-512),(127,0,-16)]:
        add(f'negative-{semi}-{cents}',[(0,m) for m in select(0,semi,cents)]+[(0,bend(0,delta))],[(0,bend(0,int(delta*(semi+cents/100)/2)))])
    add('minus-ten-octaves',[(0,m) for m in select(0,120)]+[(0,bend(0,-8192))],[(0,bend(0,0))],key=127,control_key=7)
    add('unselected-data',[(0,cc(0,6,12)),(0,bend(0,512))],[(0,bend(0,512))])
    raw=[(0,bend(0,512))]+[(96,m) for m in select(0,12)]+[(192,bytes([0x90,69,64])),(288,bytes([0x80,69,0]))]
    control=[(0,bend(0,512)),(96,bend(0,3072)),(192,bytes([0x90,69,64])),(288,bytes([0x80,69,0]))]
    files['future-note.mid']=track(raw);files['future-note-default.mid']=track(control);cases.append(dict(file='future-note.mid',control='future-note-default.mid'))
    files['range-resource.mid']=track([(0,m) for m in select(0,127,127)+[bend(0,8191),bytes([0x90,127,64])]]+[(288,bytes([0x80,127,0]))])
    files['manifest.json']=(json.dumps(dict(provenance='Twenty-five authored RPN0PitchBendSensitivityinputs compare equivalent independently encoded default2semitone bends. Semitone/cents,active retune,LSB/MSBorder,null/NRPN/unknown selection,CC121selectorresetwithrangepreservation,channel15/isolation and127range small bend. NativeVP9/PCMcompanion,no private data/encoder/network.',cases=cases,sha256={n:hashlib.sha256(v).hexdigest() for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
