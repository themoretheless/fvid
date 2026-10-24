"""Owned exact-clock edit witnesses; explicit independent per-case sample cuts."""
import json,struct
from fractions import Fraction as F
from pathlib import Path
from generate_mp4_audio_reconfiguration import generate as base_generate
from generate_mp4_fragment_description_regressions import atoms
from generate_vp9_chroma import atom,word
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-variable-rate-edits'

def edit_movie(data,edits,clock=1000,negative=False):
    end=sum(d for d,_ in edits)
    def rewrite(data,audio=False):
        out=b''
        for k,b in atoms(data):
            if k==b'trak':b=rewrite(b,b'soun' in b)
            elif k in [b'moov',b'mdia',b'minf',b'stbl']:b=rewrite(b,audio)
            elif k==b'mvhd':
                b=bytearray(b);old_clock,old_end=struct.unpack_from('>II',b,12);b[12:20]=word(clock,max(end,(old_end*clock+old_clock-1)//old_clock))
            elif k==b'tkhd':
                b=bytearray(b);old=struct.unpack_from('>I',b,20)[0];b[20:24]=word(end if audio else (old*clock+49)//50)
            elif k==b'edts' and audio:continue
            if k==b'stbl' and audio and negative:b+=atom(b'ctts',word(1<<24,1,6)+struct.pack('>i',-5000))
            if k==b'trak' and b'soun' in b:b+=atom(b'edts',atom(b'elst',word(0,len(edits))+b''.join(struct.pack('>IiI',d,m,65536)for d,m in edits)))
            out+=atom(k,b)
        return out
    return rewrite(data)

def generate():
    base=base_generate();files={};cases=[]
    def part(oracle,start,end,rate,channels,pts):return dict(oracle=oracle,first=start,last=end,rate=rate,channels=channels,pts=[pts.numerator,pts.denominator])
    def silence(frames,pts):return part(None,0,frames,48000,2,pts)
    def pcm(i,start,end,pts):
        rate,ch=[(48000,2),(32000,1),(44100,2)][i%3]
        return part(f'pcm-{i}.f32le',start,end,rate,ch,pts)
    def add(name,source,edits,parts,clock=1000,negative=False):
        files[name+'.mp4']=edit_movie(base[source],edits,clock,negative)
        cases.append(dict(name=name,parts=parts,duration=[sum(d for d,_ in edits),clock]))
    repeat=[silence(240,F(0)),pcm(0,720,960,F(5,1000)),pcm(1,0,640,F(10,1000)),pcm(2,0,441,F(30,1000)),silence(336,F(40,1000)),pcm(0,720,960,F(47,1000)),pcm(1,0,640,F(52,1000)),pcm(2,0,441,F(72,1000))]
    for suffix,source in [('', 'pcm.mp4'),('-fragmented','pcm-fragmented.mp4'),('-third','pcm-start-third.mp4')]:add('repeat'+suffix,source,[(5,-1),(35,15000),(7,-1),(35,15000)],repeat)
    add('disjoint','pcm.mp4',[(10,25000),(10,0),(10,-1)],[pcm(1,160,480,F(0)),pcm(0,0,480,F(10,1000)),silence(480,F(20,1000))])
    add('fractional','pcm.mp4',[(1,-1),(80,15000),(2,-1),(80,15000)],[silence(16,F(0)),pcm(0,720,960,F(1,3000)),pcm(1,0,640,F(16,3000)),pcm(2,0,74,F(76,3000)),silence(31,F(27,1000)+F(1,48000)),pcm(0,720,960,F(83,3000)),pcm(1,0,640,F(98,3000)),pcm(2,0,74,F(158,3000))],clock=3000)
    add('negative','pcm.mp4',[(100,0)],[pcm(0,240,960,F(0)),pcm(1,0,640,F(15,1000)),pcm(2,0,882,F(35,1000)),pcm(3,0,960,F(55,1000)),pcm(4,0,640,F(75,1000)),pcm(5,0,221,F(95,1000))],negative=True)
    def ec(control,packet,start,end,rate,ch,pts):return part(f'eac3-control-{control}.mp4#{packet}',start,end,rate,ch,pts)
    add('eac3-repeat','eac3.mp4',[(5,-1),(80,80000),(5,-1),(80,80000)],[silence(240,F(0)),ec(1,2,768,1536,48000,2,F(5,1000)),ec(2,0,0,1536,32000,1,F(21,1000)),ec(2,1,0,512,32000,1,F(69,1000)),silence(240,F(85,1000)),ec(1,2,768,1536,48000,2,F(90,1000)),ec(2,0,0,1536,32000,1,F(106,1000)),ec(2,1,0,512,32000,1,F(154,1000))])
    add('eac3-eof-repeat','eac3.mp4',[(96,240000),(96,240000)],[ec(1,i,0,1536,48000,2,F(n*96+i*32,1000))for n in range(2)for i in range(3)])
    files['past-end.mp4']=edit_movie(base['pcm.mp4'],[(140,0)])
    files['manifest.json']=(json.dumps(dict(cases=cases,provenance='Own PCM/EAC3/VP9 from companion generator. Explicit independent source-frame cuts and rational start times for repeats,disjoint windows,fractional boundaries,negative PTS and delayed EOF repeats. No external encoder,FFmpeg,network or private media.'),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
