"""Own AAC/PCM edit lists whose final edits are occupied by a preceding native sample."""
from pathlib import Path
import json,hashlib,struct
from generate_mp4_aac_transitions import generate as transitions,SCALE
from generate_mp4_variable_rate_edits import edit_movie
from generate_mp4_native_grid_silence_seek import generate as pcm
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-edit-frontier-eof-seek'
def generate():
    src=transitions();files={'reference-0.f32le':src['control-0.f32le'][:49*2*4],'reference-3.f32le':struct.pack('<9f',*[0.125]*9)};cases=[]
    def queries(rate,ch,count,control,end):
        out=[]
        for ns in sorted(set([0,1,999999,1000000,1000001,1000009,1000010,1000011,1000019,end-1,end,end+1,18446744073709551615999999999])):
            first=min(count,(ns*rate+10**9-1)//10**9)
            parts=[]if first==count else[dict(control=control,rate=rate,channels=ch,first=first,last=count,start_ns=0)]
            out.append(dict(target_ns=str(ns),parts=parts,anchor_ns=first*10**9//rate if parts else end))
        return out
    def add(name,data,edits,rate,ch,count,control,scale,video=16,raw=None):
        end=sum(d for d,_ in edits);files[name]=edit_movie(data,edits,clock=10**9);c=dict(file=name,queries=queries(rate,ch,count,control,end),duration_ns=end,scale=scale,video_frames=video)
        if raw is not None:c.update(raw_tick=raw,raw_endpoint_tick=end*scale//10**9)
        cases.append(c)
    for stem in ['changes','third','fragmented']:
        add(stem+'-occupied-eof.mp4',src[stem+'.mp4'],[(1000010,0),(10,SCALE*64//1000)],48000,2,49,0,SCALE)
    add('occupied-silence-and-eof.mp4',src['changes.mp4'],[(1000010,0),(3,-1),(7,SCALE*64//1000)],48000,2,49,0,SCALE)
    add('raw-grid-occupied-eof.mp4',src['changes.mp4'],[(1000001,0),(99,SCALE*64//1000)],48000,2,49,0,SCALE,raw=14113)
    p=pcm();add('pcm-occupied-eof.mp4',p['changing-pcm-native-grid.mp4'],[(1000010,0),(99990,0)],8000,1,9,3,48000,video=3,raw=49)
    files['manifest.json']=(json.dumps(dict(provenance='Own LC/Main AAC pulse spectra, independent scalar IMDCT/overlap references;own8kHz PCM0.125 and neutral VP9 pictures. Initial edit includes a complete last native sample whose occupied interval covers final edit(s). Exact rational EOF and raw tick endpoints are independent controls. No private input,FFmpeg or network.',cases=cases,sha256={n:hashlib.sha256(b).hexdigest()for n,b in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
