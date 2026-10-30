"""Own positive-CTS chapter clocks, with movie/track duration controls."""
from pathlib import Path
import struct,json,hashlib
from generate_mp4_seek_io_regressions import generate
from generate_vp9_chroma import atom,word
from generate_mp4_tfhd_defaults_regressions import atoms
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-chapter-composition'
def main():
    own=generate();files={};cases=[]
    for header in ['known','unknown','zero']:
        for movie in ['known','unknown','zero']:
            for clock,starts in [('movie',[200,220,240]),('ten-mhz',[2000000,2200000,2400000])]:
                def rewrite(data):
                    out=b''
                    for k,b in atoms(data):
                        if k in [b'moov',b'trak',b'mdia',b'minf',b'stbl']:
                            b=rewrite(b)
                            if k==b'stbl':b+=atom(b'ctts',word(0,1,3,10))
                            if k==b'moov':
                                chpl=word(1<<24,0)+bytes([3])
                                for start,title in zip(starts,[b'Opening',b'Middle',b'End']):chpl+=struct.pack('>Q',start)+bytes([len(title)])+title
                                b+=atom(b'udta',atom(b'chpl',chpl))
                        elif k==b'mvhd':b=b[:12]+word(1000,{'known':260,'unknown':0xffffffff,'zero':0}[movie])+b[20:]
                        elif k==b'tkhd':b=b[:20]+word({'known':260,'unknown':0xffffffff,'zero':0}[header])+b[24:]
                        out+=atom(k,b)
                    return out
                n=f'{header}-track-{movie}-movie-{clock}.mp4';files[n]=rewrite(own['three-colors.mp4']);cases.append(n)
    for i in range(3):files[f'frame-{i}.yuv16']=own[f'frame-{i}.yuv16']
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in files.items():(OUT/n).write_bytes(b)
    (OUT/'manifest.json').write_text(json.dumps({'provenance':'Own 3-frame colored VP9, decode PTS 0/1/2 plus authored ctts +10 ticks at 50Hz. Chapter moments 200/220/240ms in two supported clocks; movie and track duration known/unknown/zero. No private media, codec parameters, FFmpeg or network.','cases':cases,'sha256':{n:hashlib.sha256(b).hexdigest() for n,b in files.items()}},indent=2)+'\n')
if __name__=='__main__':main()
