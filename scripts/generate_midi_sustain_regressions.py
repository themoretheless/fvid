"""Author sustain-pedal inputs and independently delayed-note-off controls."""
import json,hashlib
from pathlib import Path
from generate_smf_clock_range_regressions import midi
from generate_audio_reader_eof_regressions import generate as pcm
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/midi-sustain'
def vlq(value):
    out=[value&127];value>>=7
    while value:out.insert(0,(value&127)|128);value>>=7
    return bytes(out)
def pack(events,end=288):
    data=bytearray();last=0
    for tick,message in sorted(events,key=lambda e:e[0]):data+=vlq(tick-last)+bytes(message);last=tick
    assert last<=end
    data+=vlq(end-last)+b'\xff\x2f\0';return midi(bytes(data),480)
def generate():
    files={'video.mkv':pcm()['video.mkv']};cases=[]
    on=(0,[0x90,69,100]);off=(96,[0x80,69,0]);late=(192,[0x80,69,0]);pedal=(0,[0xb0,64,64]);up=(192,[0xb0,64,0])
    specs=[('threshold64',[pedal,on,off,up],[on,late]),('threshold127',[(0,[0xb0,64,127]),on,off,up],[on,late]),('threshold63',[(0,[0xb0,64,63]),on,off,up],[on,off]),('key-still-down',[pedal,on,(0,[0x90,76,100]),off,up,(240,[0x80,76,0])],[on,(0,[0x90,76,100]),late,(240,[0x80,76,0])]),('channel-isolation',[pedal,on,(0,[0x97,76,100]),off,(96,[0x87,76,0]),up],[on,(0,[0x97,76,100]),(96,[0x87,76,0]),late]),('all-notes-off',[pedal,on,(96,[0xb0,123,0]),up],[on,late]),('all-sound-off',[pedal,on,off,(144,[0xb0,120,0]),up],[on,(144,[0xb0,120,0])]),('reset-pedal',[pedal,on,off,(192,[0xb0,121,0])],[on,late,(192,[0xb0,121,0])]),('same-tick-off',[pedal,on,(0,[0x80,69,0]),(96,[0xb0,64,0])],[on,off]),('pedal-after-release',[on,off,(144,[0xb0,64,127]),up],[on,off]),('repeated-on',[pedal,on,off,(144,[0xb0,64,127]),up],[on,late]),('held-pedal-eof',[pedal,on,off],[on,(288,[0x80,69,0])])]
    def add(name,actual,control,end=288):
        files[name+'.mid']=pack(actual,end);files[name+'-explicit.mid']=pack(control,end);files[name+'-no-pedal.mid']=pack([(t,m) for t,m in actual if not(m[0]&0xf0==0xb0 and m[1]==64)],end)
        cases.append(dict(name=name,file=name+'.mid',control=name+'-explicit.mid',without=name+'-no-pedal.mid',end_tick=end))
    for name,actual,control in specs:add(name,actual,control)
    add('held-pedal-fractional-eof',[pedal,on,off],[on,(239,[0x80,69,0])],239)
    files['manifest.json']=(json.dumps(dict(provenance='Thirteen authored short MIDI scores compare sustain64with independently delayed note-offs,threshold63/64/127,key-down andchannel isolation,AllNotesOff,AllSoundOff,ResetControllers,same-tick release,non-resurrection,repeatedon,pedal-held EOF andfractional-tick EOF inside a decode window. No-pedal counterparts preserve original ignored-control reproduction separately. Short native VP9/PCM companion. No encoder/network/private media.',cases=cases,sha256={n:hashlib.sha256(v).hexdigest() for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
