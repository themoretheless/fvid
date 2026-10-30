"""Authored VP9/UTF8/ASS Matroska subtitle spans, with independent block metadata."""
from pathlib import Path
import struct,json,hashlib
from generate_vp9_chroma import ebml
from generate_mp4_seek_io_regressions import generate
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'tests/fixtures/playback-errors/matroska-subtitle-spans'
ASS='''[Script Info]
ScriptType: v4.00+
PlayResX: 16
PlayResY: 16
[V4+ Styles]
Format: Name,Fontname,Fontsize,PrimaryColour,SecondaryColour,OutlineColour,BackColour,Bold,Italic,Underline,StrikeOut,ScaleX,ScaleY,Spacing,Angle,BorderStyle,Outline,Shadow,Alignment,MarginL,MarginR,MarginV,Encoding
Style: Default,Serif,12,&H00FFFFFF,&H00FFFFFF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,0,0,2,0,0,0,1
[Events]
Format: Layer,Start,End,Style,Name,MarginL,MarginR,MarginV,Effect,Text
'''.encode()
def uint(n):return n.to_bytes(max(1,(n.bit_length()+7)//8),'big')
def main():
    own=generate();video=[own[f'frame-{i}.packet'] for i in range(3)];texts=['first','middle\nline','last']
    payloads={'utf8':[b'first',b'<i>middle</i>\\Nline',b'last'],'ass':[f'{i},0,Default,,0,0,0,,{t}'.encode() for i,t in enumerate(['first','{\\i1}middle{\\i0}\\Nline','last'])]}
    def track(number,kind,codec,default,extra=b''):
        fields=ebml('d7',uint(number))+ebml('73c5',uint(number))+ebml('83',uint(kind))+ebml('86',codec.encode())+ebml('9c',b'\0')
        if default:fields+=ebml('23e383',uint(default))
        return ebml('ae',fields+extra)
    def clip(codec,scale,default,mode,invalid=False):
        header=ebml('1a45dfa3',ebml('4282',b'matroska')+ebml('4287',b'\4')+ebml('4285',b'\2'))
        info=ebml('1549a966',ebml('2ad7b1',uint(scale))+ebml('4489',struct.pack('>d',120_000_000/scale))+ebml('4d80',b'fvid-synthetic')+ebml('5741',b'fvid-synthetic'))
        v=track(1,1,'V_VP9',40_000_000,ebml('e0',ebml('b0',uint(16))+ebml('ba',uint(16))))
        s=b'' if codec is None else track(2,17,'S_TEXT/UTF8' if codec=='utf8' else 'S_TEXT/ASS',default,ebml('536e',b'Authored subtitles')+ebml('22b59c',b'eng')+(ebml('63a2',ASS) if codec=='ass' else b''))
        cluster=ebml('e7',uint(0))+b''.join(ebml('a3',b'\x81'+struct.pack('>h',t*1_000_000//scale)+b'\x80'+p) for t,p in zip([0,40,80],video))
        body=info+ebml('1654ae6b',v+s)+ebml('1f43b675',cluster)
        if codec is not None:
            sub=ebml('e7',uint(40_000_000//scale))
            for i,(t,p) in enumerate(zip([10,60,90],payloads[codec])):
                block=b'\x82'+struct.pack('>h',(t-40)*1_000_000//scale)+(b'\x80' if mode=='missing' else b'\0')+p
                if mode=='missing':sub+=ebml('a3',block)
                else:
                    duration=[20,0 if mode=='zero' else 10,10][i]*1_000_000//scale
                    if invalid and i==0:duration=(1<<64)-1
                    sub+=ebml('a0',ebml('a1',block)+ebml('9b',uint(duration)))
            body+=ebml('1f43b675',sub)
        return header+ebml('18538067',body)
    files={'video-only.mkv':clip(None,1_000_000,0,'missing')};cases=[]
    for codec in ['utf8','ass']:
        for i,p in enumerate(payloads[codec]):files[f'{codec}-{i}.packet']=p
        for scale in [1_000_000,2_000_000]:
            for default in [0,60_000_000]:
                for mode in ['explicit','zero','missing']:
                    n=f'{codec}-scale{scale}-default{default}-{mode}.mkv';files[n]=clip(codec,scale,default,mode)
                    durations=[None]*3 if mode=='missing' else [20_000_000,0 if mode=='zero' else 10_000_000,10_000_000]
                    pts=[10_000_000,60_000_000,90_000_000];cues=[]
                    for i,(start,d,text) in enumerate(zip(pts,durations,texts)):
                        end=start+d if d is not None else (start+default if default else (pts[i+1] if i+1<3 else start+2_000_000_000))
                        if end>start:cues.append(dict(text=text,start_ns=start,end_ns=end))
                    cases.append(dict(file=n,codec=codec,scale_ns=scale,default_ns=default,mode=mode,pts_ns=pts,packet_durations_ns=durations,cues=cues))
    files['duration-overflow.mkv']=clip('utf8',2_000_000,0,'explicit',True)
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in files.items():(OUT/n).write_bytes(b)
    (OUT/'manifest.json').write_text(json.dumps(dict(provenance='Pure authored three-color16x16 VP9 and invented UTF8/canonical mapped ASS text. Two clusters, negative relative block timestamps with positive absolute PTS, explicit BlockDuration including zero, DefaultDuration conflicts and SimpleBlock fallback controls. No private media/parameters, FFmpeg or network.',standards=['https://www.matroska.org/technical/subtitles.html','https://www.rfc-editor.org/rfc/rfc9559.html#section-5.1.3.5.3'],cases=cases,refused=['duration-overflow.mkv'],sha256={n:hashlib.sha256(b).hexdigest() for n,b in files.items()}),indent=2)+'\n')
if __name__=='__main__':main()
