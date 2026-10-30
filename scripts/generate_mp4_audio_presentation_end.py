"""Own PCM/ALAC+VP9 composition endpoint witnesses."""
from pathlib import Path
import json,struct,hashlib
from generate_mp4_audio_clock_regressions import generate
from generate_vp9_chroma import atom,word
from generate_mp4_tfhd_defaults_regressions import atoms
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-audio-presentation-end'
def main():
    own=generate();files={'full.f32le':own['expected.f32le'],'negative.f32le':own['negative-expected.f32le'],'full-tail.f32le':own['expected.f32le']+bytes(160*4)};cases=[]
    for codec in ['pcm','alac']:
        for clock in [1000,44100,1001]:
            base=own[f'{codec}-{1000 if clock==1001 else clock}.mp4']
            for shift in [-10,0,10]:
                for state in ['known','unknown']:
                    shift_ticks=(abs(shift)*clock//1000)*(1 if shift>=0 else -1)
                    end_ns=60_000_000+shift_ticks*1_000_000_000//clock
                    end=(end_ns+999_999)//1_000_000
                    def rewrite(data,audio=False):
                        out=b''
                        for k,b in atoms(data):
                            selected=audio
                            if k==b'trak':selected=dict(atoms(dict(atoms(b))[b'mdia']))[b'hdlr'][8:12]==b'soun'
                            if k in [b'moov',b'trak',b'mdia',b'minf',b'stbl']:
                                b=rewrite(b,selected)
                                if k==b'stbl' and audio:b+=atom(b'ctts',word(1<<24,1,3)+struct.pack('>i',shift_ticks))
                            elif k==b'mvhd':b=b[:12]+word(1000,max(60,end))+b[20:]
                            elif k==b'tkhd':b=b[:20]+word(end if audio else 60)+b[24:]
                            elif k==b'mdhd' and audio:b=b[:12]+word(clock,0xffffffff if state=='unknown' else struct.unpack_from('>I',b,16)[0])+b[20:]
                            out+=atom(k,b)
                        return out
                    n=f'{codec}-{clock}-shift{shift}-{state}.mp4';files[n]=rewrite(base);cases.append(dict(file=n,end_ns=end_ns,shift_ms=shift,shift_ticks=shift_ticks,clock=clock,oracle='negative.f32le' if shift<0 else 'full.f32le'))
    # Hybrid initial tables plus an explicit empty-duration audio tail. Its
    # stated extent must survive even though it adds no packet/picture.
    for codec in ['pcm','alac']:
        base=files[f'{codec}-1000-shift0-known.mp4']
        def gap(data,audio=False):
            out=b''
            for k,b in atoms(data):
                selected=audio
                if k==b'trak':selected=dict(atoms(dict(atoms(b))[b'mdia']))[b'hdlr'][8:12]==b'soun'
                if k in [b'moov',b'trak']:
                    b=gap(b,selected)
                    if k==b'moov':b+=atom(b'mvex',atom(b'trex',word(0,2,1,0,0,0)))
                elif k==b'mvhd':b=b[:16]+word(80)+b[20:]
                elif k==b'tkhd' and audio:b=b[:20]+word(80)+b[24:]
                out+=atom(k,b)
            return out
        tail=atom(b'moof',atom(b'mfhd',word(0,1))+atom(b'traf',atom(b'tfhd',word(0x30008,2,20))+atom(b'tfdt',word(0,60))))
        n=f'{codec}-empty-tail.mp4';files[n]=gap(base)+tail;cases.append(dict(file=n,end_ns=80_000_000,shift_ms=0,shift_ticks=0,clock=1000,oracle='full-tail.f32le',silent_tail_frames=160))
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in files.items():(OUT/n).write_bytes(b)
    (OUT/'manifest.json').write_text(json.dumps({'provenance':'Own 60ms VP9 with independently authored 8kHz mono PCM/raw ALAC. Signed composition offsets -10/0/+10ms, 1000/44100/1001 media clocks, known/unknown mdhd, explicit empty audio tails. No private media/parameters, external encoder, FFmpeg or network.','cases':cases,'sha256':{n:hashlib.sha256(b).hexdigest() for n,b in files.items()}},indent=2)+'\n')
if __name__=='__main__':main()
