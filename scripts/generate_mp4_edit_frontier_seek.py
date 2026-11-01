"""Authored non-native edit boundaries with independent scalar AAC controls."""
from pathlib import Path
from fractions import Fraction as F
import json,hashlib
from generate_mp4_aac_transitions import generate as transitions,SCALE
from generate_mp4_variable_rate_edits import edit_movie
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-edit-frontier-seek'
def generate():
    src=transitions();files={'reference-0.f32le':src['control-0.f32le'][:49*2*4],'reference-1.f32le':src['control-1.f32le'][:33*4]};cases=[]
    end0=F(1000010,10**9);end=end0+F(1,1000)
    # Continuous playback carries the final first-edit sample's full native
    # interval forward. The second edit cannot repeat that occupied interval.
    previous_end=F(49,48000);minimum_second=((previous_end-end0)*32000).__ceil__()
    queries=[]
    for ns in [0,1,999999,1000000,1000001,1000009,1000010,1000011,1020833,1031260,2000009,2000010,18446744073709551615999999999]:
        wanted=F(ns,10**9);parts=[]
        for control,rate,ch,first,last,start in [(0,48000,2,0,49,F(0)),(1,32000,1,minimum_second,32,end0)]:
            offset=min(last,max(first,((wanted-start)*rate).__ceil__()))
            if offset<last:parts.append(dict(control=control,rate=rate,channels=ch,first=offset,last=last,start_ns=int(start*10**9)))
        at=F(parts[0]['start_ns'],10**9)+F(parts[0]['first'],parts[0]['rate'])if parts else end
        queries.append(dict(target_ns=str(ns),parts=parts,anchor_ns=int(at*10**9)))
    for stem in ['changes','third','fragmented']:
        name=stem+'-frontier.mp4';files[name]=edit_movie(src[stem+'.mp4'],[(1000010,0),(1000000,SCALE*64//1000)],clock=10**9);cases.append(dict(file=name,queries=queries,duration_ns=int(end*10**9)))
    files['manifest.json']=(json.dumps(dict(provenance='Own LC/Main AAC pulse spectra, scalar IMDCT/overlap PCM and sixteen neutral VP9 frames. Non-native 1000010ns first edit followed by1ms edit; expected native windows derived with rational arithmetic. No private media,FFmpeg or network.',cases=cases,sha256={n:hashlib.sha256(b).hexdigest()for n,b in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
