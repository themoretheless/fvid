"""Author muted MIDI voices, exact unmute phase controls and CPU workloads."""
import json,hashlib
from pathlib import Path
from generate_midi_bend_range_regressions import cc,track
from generate_audio_reader_eof_regressions import generate as pcm
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/midi-muted-render'
def generate():
    files={'video.mkv':pcm()['video.mkv']};cases=[]
    def add(name,voices,controller=7,resume=None,off=288,partial=False,mute_tick=0):
        init=[]
        for ch in range(min(voices,16)):init += [(0,cc(ch,7,64+(ch%4)*16)),(0,cc(ch,11,96)),(0,cc(ch,10,ch*7))]
        muted=set(range(8))if partial else set(range(min(voices,16)))
        actual=list(init);control=list(init)
        for ch in muted:
            actual.append((mute_tick,cc(ch,controller,0)))
            if resume is not None:actual.append((resume,cc(ch,controller,64+(ch%4)*16 if controller==7 else 96)))
        for i in range(voices):
            ch=i%16;key=40+i;notes=[(0,bytes([0x90+ch,key,64])),(off,bytes([0x80+ch,key,0]))];actual+=notes
            if not partial or ch not in muted:control+=notes
        files[name+'.mid']=track(actual);files[name+'-continuous.mid']=track(control)
        mode='reference'if partial else 'fractional'if mute_tick else 'resume'if resume is not None and off==288 else 'silent'
        cases.append(dict(file=name+'.mid',control=name+'-continuous.mid',mode=mode,resume_packet=2 if mode in ['resume','fractional'] else None,voices=voices,edge_frame=46 if mode=='fractional' else None))
    for voices in [1,8,32]:add(f'volume-muted-{voices}',voices)
    for voices in [1,8,32]:add(f'volume-unmute-{voices}',voices,resume=192)
    add('expression-unmute-8',8,controller=11,resume=192)
    add('released-before-unmute-8',8,resume=192,off=96)
    add('partial-muted-32',32,partial=True)
    add('fractional-unmute-8',8,resume=193,mute_tick=1)
    files['manifest.json']=(json.dumps(dict(provenance='Ten authored MIDI mute/render CPUprofiles. Volume/expression zero,1/8/32voices,phase continuity after200msunmute,release-before-unmute,fractional mid-window mute/unmute andpartialchannel mute compare independent continuous/unmuted-voice controls. Existing behavior baseline;notnewmediafailure claims. NativeVP9/PCMcompanion,no encoder/network/private media.',cases=cases,sha256={n:hashlib.sha256(v).hexdigest() for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
