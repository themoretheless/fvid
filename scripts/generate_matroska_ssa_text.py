"""Generate authored Matroska SSA/ASS videos; no external tools or media."""
from pathlib import Path
import hashlib,json,struct
from generate_vp9_chroma import ebml
from generate_mp4_seek_io_regressions import generate
from generate_matroska_subtitle_spans import ASS
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/matroska-ssa-text'
SSA=b'''[Script Info]
ScriptType: v4.00
PlayResX: 16
PlayResY: 16
[V4 Styles]
Format: Name,Fontname,Fontsize,PrimaryColour,SecondaryColour,TertiaryColour,BackColour,Bold,Italic,BorderStyle,Outline,Shadow,Alignment,MarginL,MarginR,MarginV,AlphaLevel,Encoding
Style: Default,Serif,12,16777215,16777215,0,0,0,0,1,0,0,2,0,0,0,0,1
[Events]
Format: Marked,Start,End,Style,Name,MarginL,MarginR,MarginV,Effect,Text
'''
def u(n):return n.to_bytes(max(1,(n.bit_length()+7)//8),'big')
def track(n,kind,codec,extra=b''):
    return ebml('ae',ebml('d7',u(n))+ebml('73c5',u(n))+ebml('83',u(kind))+ebml('86',codec.encode())+ebml('9c',b'\0')+extra)
def main():
    source=generate();video=[source[f'frame-{i}.packet'] for i in range(3)]
    files={};cases=[]
    for codec in ['SSA','ASS']:
        for layer in ([''] if codec=='SSA' else ['0','12']):
            n=f'{codec.lower()}-layer{layer or "empty"}.mkv'
            texts=['First, with commas','{\\i1}Middle{\\i0}\\Nnext','Last Привет']
            payloads=[f'{i},{layer},Default,,0,0,0,,{text}'.encode() for i,text in enumerate(texts)]
            header=ebml('1a45dfa3',ebml('4282',b'matroska')+ebml('4287',b'\4')+ebml('4285',b'\2'))
            info=ebml('1549a966',ebml('2ad7b1',u(1_000_000))+ebml('4489',struct.pack('>d',120))+ebml('4d80',b'fvid-synthetic')+ebml('5741',b'fvid-synthetic'))
            vt=track(1,1,'V_VP9',ebml('23e383',u(40_000_000))+ebml('e0',ebml('b0',u(16))+ebml('ba',u(16))))
            st=track(2,17,'S_TEXT/'+codec,ebml('63a2',SSA if codec=='SSA' else ASS))
            cluster=ebml('e7',b'\0')
            for i,(v,p) in enumerate(zip(video,payloads)):
                cluster+=ebml('a3',b'\x81'+struct.pack('>h',i*40)+b'\x80'+v)
                cluster+=ebml('a0',ebml('a1',b'\x82'+struct.pack('>h',i*40)+b'\0'+p)+ebml('9b',u(20)))
                files[f'{n}-{i}.packet']=p
            files[n]=header+ebml('18538067',info+ebml('1654ae6b',vt+st)+ebml('1f43b675',cluster))
            cases.append(dict(file=n,codec=codec,text=['First, with commas','Middle\nnext','Last Привет']))
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in files.items():(OUT/n).write_bytes(b)
    (OUT/'manifest.json').write_text(json.dumps(dict(provenance='Authored16x16 three-color VP9, complete invented SSA/ASS headers and text; canonical SSA empty Layer, numeric ASS controls. No external encoder/network/private media.',reference='https://www.matroska.org/technical/subtitles.html',cases=cases,sha256={n:hashlib.sha256(b).hexdigest() for n,b in files.items()}),indent=2)+'\n')
if __name__=='__main__':main()
