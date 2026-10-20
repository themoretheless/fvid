"""Author MIDI CC10 stereo balance with independent isolated mono voices."""
import json,hashlib
from pathlib import Path
from generate_smf_independent_tracks_regressions import smf
from generate_smf_smpte_regressions import vlq
from generate_audio_reader_eof_regressions import generate as pcm
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/midi-pan'
def generate():
    def track(events):
        out=bytearray();last=0
        for tick,message in sorted(events,key=lambda e:e[0]):out+=vlq(tick-last)+message;last=tick
        out+=vlq(288-last)+b'\xff\x2f\0';return smf(0,[bytes(out)])
    files={'video.mkv':pcm()['video.mkv']};cases=[]
    def add(label,voices,pans,extras=[]):
        events=[(tick,bytes([0xb0+ch,10,value])) for tick,ch,value in pans]+extras
        for ch,key in voices:events += [(0,bytes([0x90+ch,key,64])),(288,bytes([0x80+ch,key,0]))]
        file=label+'.mid';files[file]=track(events);controls=[]
        for ch,key in voices:
            own=[(tick,msg) for tick,msg in events if msg[0]&15==ch and not(msg[0]&0xf0==0xb0 and msg[1]==10)]
            control=f'{label}-channel-{ch}.mid';files[control]=track(own);controls.append(dict(channel=ch,file=control))
        cases.append(dict(file=file,controls=controls,pan_events=[dict(us=tick*500000//480,channel=ch,value=value) for tick,ch,value in pans]))
    for pan in [0,32,64,96,127]:add('pan-'+str(pan),[(0,69)],[(0,0,pan)])
    add('late-left',[(0,69)],[(96,0,0)])
    add('persistent-right',[(0,69)],[(96,0,127)])
    add('return-center',[(0,69)],[(96,0,0),(192,0,64)])
    add('reset-preserves-pan',[(0,69)],[(96,0,0)],[(192,b'\xb0\x79\0')])
    add('channel15',[(15,72)],[(0,15,127)])
    add('channel-isolation',[(0,69),(15,72)],[(0,0,0)])
    add('volume-expression',[(0,69)],[(0,0,32)],[(0,b'\xb0\7\x1f'),(0,b'\xb0\x0b\x40')])
    add('fractional-pan',[(0,69)],[(1,0,127)])
    add('pan-at-end',[(0,69)],[(288,0,0)])
    files['manifest.json']=(json.dumps(dict(provenance='Fourteen authored CC10cases withindependent isolated mono voice controls. Native center-preserving linear stereo balance:0left,64unchangeddualmono,127right. Intermediatevalues,timing,persistence,CC121preserves,channel15,isolation,gaincomposition andreleasewindow. No fullGMpan-law claim. NativeVP9/PCMcompanion,no private media/encoder/network.',cases=cases,sha256={n:hashlib.sha256(v).hexdigest() for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
