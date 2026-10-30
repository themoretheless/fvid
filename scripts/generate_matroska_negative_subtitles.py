"""Authored negative-PTS subtitle frames with positive, decodable VP9 video."""
from pathlib import Path
import struct,json,hashlib
from generate_vp9_chroma import ebml
from generate_mp4_seek_io_regressions import generate
from generate_matroska_ssa_text import u,track,SSA,ASS
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/matroska-negative-subtitles'
def main():
    source=generate();files={};cases=[]
    for codec in ['UTF8','SSA','ASS']:
        for negative in [False,True]:
            name=f'{codec.lower()}-{"negative" if negative else "positive"}.mkv'
            pts=[-30,-10,60] if negative else [0,30,60];durations=[10,30,20]
            texts=['before origin','crosses origin','visible caption']
            packets=[t.encode() if codec=='UTF8' else f'{i},{"" if codec=="SSA" else "0"},Default,,0,0,0,,{t}'.encode() for i,t in enumerate(texts)]
            header=ebml('1a45dfa3',ebml('4282',b'matroska')+ebml('4287',b'\4')+ebml('4285',b'\2'))
            info=ebml('1549a966',ebml('2ad7b1',u(1_000_000))+ebml('4489',struct.pack('>d',120))+ebml('4d80',b'fvid-synthetic')+ebml('5741',b'fvid-synthetic'))
            video=track(1,1,'V_VP9',ebml('23e383',u(40_000_000))+ebml('e0',ebml('b0',u(16))+ebml('ba',u(16))))
            sub=track(2,17,'S_TEXT/'+codec,ebml('63a2',SSA if codec=='SSA' else ASS) if codec!='UTF8' else b'')
            cluster=ebml('e7',b'\0')
            for i in range(3):
                cluster+=ebml('a3',b'\x81'+struct.pack('>h',i*40)+b'\x80'+source[f'frame-{i}.packet'])
                cluster+=ebml('a0',ebml('a1',b'\x82'+struct.pack('>h',pts[i])+b'\0'+packets[i])+ebml('9b',u(durations[i])))
                files[f'{codec.lower()}-{i}.packet']=packets[i]
            files[name]=header+ebml('18538067',info+ebml('1654ae6b',video+sub)+ebml('1f43b675',cluster))
            cases.append(dict(file=name,pts_ns=[p*1_000_000 for p in pts],duration_ns=[d*1_000_000 for d in durations],codec=codec.lower(),cues=[dict(text=t,start_ns=p*1_000_000,end_ns=(p+d)*1_000_000) for t,p,d in zip(texts,pts,durations) if p>=0]))
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in files.items():(OUT/n).write_bytes(b)
    (OUT/'manifest.json').write_text(json.dumps(dict(provenance='Pure authored16x16 three-color VP9 with invented UTF8/SSA/ASS. Negative absolute subtitle PTS are discouraged by RFC9559 but its11.2 mandates decoding without playing them. Positive control videos and healthy later cues distinguish suppressed content from refusal. No private media, FFmpeg or network.',reference='https://www.rfc-editor.org/rfc/rfc9559.html#section-11.2',cases=cases,sha256={n:hashlib.sha256(b).hexdigest() for n,b in files.items()}),indent=2)+'\n')
if __name__=='__main__':main()
