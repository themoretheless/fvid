"""Author Block/SimpleBlock invisible-caption witnesses, no external media."""
from pathlib import Path
import json,hashlib,struct
from generate_vp9_chroma import ebml
from generate_mp4_seek_io_regressions import generate
from generate_matroska_ssa_text import u,track,SSA,ASS
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/matroska-invisible-subtitles'
def main():
    source=generate();files={};cases=[]
    for codec in ['UTF8','SSA','ASS']:
        for mode in ['explicit','missing']:
            for mask in [0,2,6,7]:
                name=f'{codec.lower()}-{mode}-mask{mask}.mkv';texts=['First caption','Middle caption','Last caption'];pts=[0,40_000_000,80_000_000];flags=[bool(mask&(1<<i)) for i in range(3)]
                packets=[t.encode() if codec=='UTF8' else f'{i},{"" if codec=="SSA" else "0"},Default,,0,0,0,,{t}'.encode() for i,t in enumerate(texts)]
                header=ebml('1a45dfa3',ebml('4282',b'matroska')+ebml('4287',b'\4')+ebml('4285',b'\2'))
                info=ebml('1549a966',ebml('2ad7b1',u(1_000_000))+ebml('4489',struct.pack('>d',120))+ebml('4d80',b'fvid-synthetic')+ebml('5741',b'fvid-synthetic'))
                video=track(1,1,'V_VP9',ebml('23e383',u(40_000_000))+ebml('e0',ebml('b0',u(16))+ebml('ba',u(16))))
                sub=track(2,17,'S_TEXT/'+codec,ebml('63a2',SSA if codec=='SSA' else ASS) if codec!='UTF8' else b'');cluster=ebml('e7',b'\0')
                for i in range(3):
                    timestamp=struct.pack('>h',i*40);cluster+=ebml('a3',b'\x81'+timestamp+b'\x80'+source[f'frame-{i}.packet'])
                    payload=b'\x82'+timestamp+bytes([(8 if flags[i] else 0)|(128 if mode=='missing' else 0)])+packets[i]
                    cluster+=ebml('a0',ebml('a1',payload)+ebml('9b',u(20))) if mode=='explicit' else ebml('a3',payload)
                    files[f'{codec.lower()}-{i}.packet']=packets[i]
                files[name]=header+ebml('18538067',info+ebml('1654ae6b',video+sub)+ebml('1f43b675',cluster));cues=[]
                for i,(t,p) in enumerate(zip(texts,pts)):
                    if flags[i]:continue
                    following=next((pts[j] for j in range(i+1,3) if not flags[j]),p+2_000_000_000)
                    cues.append(dict(text=t,start_ns=p,end_ns=p+20_000_000 if mode=='explicit' else following))
                cases.append(dict(file=name,codec=codec.lower(),pts_ns=pts,packet_durations_ns=[20_000_000]*3 if mode=='explicit' else [None]*3,invisible=flags,cues=cues))
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in files.items():(OUT/n).write_bytes(b)
    (OUT/'manifest.json').write_text(json.dumps(dict(provenance='Authored16x16 three-color VP9 with invented UTF8/SSA/ASS. Native Block/SimpleBlock Invisible flags; visible controls, hidden middle/tail/all events, explicit and compatibility fallback timing. No private media/parameters, FFmpeg or network.',reference='https://www.rfc-editor.org/rfc/rfc9559.html#section-10.1',cases=cases,sha256={n:hashlib.sha256(b).hexdigest() for n,b in files.items()}),indent=2)+'\n')
if __name__=='__main__':main()
