"""Authored Matroska subtitle CodecDelay clocks; offline, no external media."""
from pathlib import Path
import hashlib,json,struct
from generate_vp9_chroma import ebml
from generate_mp4_seek_io_regressions import generate
from generate_matroska_ssa_text import u,track,SSA,ASS
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/matroska-subtitle-delay'
def main():
    source=generate();files={};cases=[]
    for codec in ['UTF8','SSA','ASS']:
        for scale in [1_000_000,2_000_000]:
            for delay in [0,20_000_000,(1<<64)-1]:
                for mode in ['explicit','missing','default']:
                    name=f'{codec.lower()}-scale{scale}-delay{delay}{"" if mode=="explicit" else "-"+mode}.mkv'
                    pts=[0,40_000_000,80_000_000];durations=[30_000_000,20_000_000,20_000_000];texts=['First caption','Middle caption','Last caption']
                    packets=[t.encode() if codec=='UTF8' else f'{i},{"" if codec=="SSA" else "0"},Default,,0,0,0,,{t}'.encode() for i,t in enumerate(texts)]
                    header=ebml('1a45dfa3',ebml('4282',b'matroska')+ebml('4287',b'\4')+ebml('4285',b'\2'))
                    info=ebml('1549a966',ebml('2ad7b1',u(scale))+ebml('4489',struct.pack('>d',120_000_000/scale))+ebml('4d80',b'fvid-synthetic')+ebml('5741',b'fvid-synthetic'))
                    video=track(1,1,'V_VP9',ebml('23e383',u(40_000_000))+ebml('e0',ebml('b0',u(16))+ebml('ba',u(16))))
                    default=20_000_000 if mode=='default' else 0
                    extra=(ebml('23e383',u(default)) if default else b'')+(ebml('63a2',SSA if codec=='SSA' else ASS) if codec!='UTF8' else b'')+(ebml('56aa',u(delay)) if delay else b'')
                    sub=track(2,17,'S_TEXT/'+codec,extra);cluster=ebml('e7',b'\0')
                    for i in range(3):
                        timestamp=struct.pack('>h',pts[i]//scale)
                        cluster+=ebml('a3',b'\x81'+timestamp+b'\x80'+source[f'frame-{i}.packet'])
                        if mode=='explicit':cluster+=ebml('a0',ebml('a1',b'\x82'+timestamp+b'\0'+packets[i])+ebml('9b',u(durations[i]//scale)))
                        else:cluster+=ebml('a3',b'\x82'+timestamp+b'\x80'+packets[i])
                        files[f'{codec.lower()}-{i}.packet']=packets[i]
                    files[name]=header+ebml('18538067',info+ebml('1654ae6b',video+sub)+ebml('1f43b675',cluster))
                    cues=[]
                    for i,(t,p,d) in enumerate(zip(texts,pts,durations)):
                        if p<delay:continue
                        start=p-delay
                        end=start+d if mode=='explicit' else (start+default if default else (pts[i+1]-delay if i+1<3 else start+2_000_000_000))
                        cues.append(dict(text=t,start_ns=start,end_ns=end))
                    cases.append(dict(file=name,pts_ns=pts,packet_durations_ns=durations if mode=='explicit' else [None]*3,codec=codec.lower(),default_ns=default,delay_ns=delay,mode=mode,cues=cues))
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in files.items():(OUT/n).write_bytes(b)
    (OUT/'manifest.json').write_text(json.dumps(dict(provenance='Pure authored16x16 three-color VP9 with invented UTF8/SSA/ASS text. CodecDelay is nanoseconds independent of TimestampScale; omitted delay,20ms delay and maximum unsigned delay controls. No private media/parameters, FFmpeg or network.',reference='https://www.rfc-editor.org/rfc/rfc9559.html#section-11.2',cases=cases,sha256={n:hashlib.sha256(b).hexdigest() for n,b in files.items()}),indent=2)+'\n')
if __name__=='__main__':main()
