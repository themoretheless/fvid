"""Own ALAC split/silence/repeat edit windows with exact lossless PCM oracles."""
from pathlib import Path
from fractions import Fraction as F
import copy,json,hashlib
from generate_mp4_unedited_alac_subtick_seek import generate as source_alac
from generate_mp4_variable_rate_edits import edit_movie
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-alac-edit-subtick-seek'
def generate():
    source=source_alac();files={n:b for n,b in source.items()if n.startswith('reference-')};files['reference-8.f32le']=bytes(48*2*4);files['reference-9.f32le']=source['reference-0.f32le'][80*4:88*4];cases=[]
    def add(name,data,edits,parts,scale,video):
        end=sum(d for d,_ in edits);queries=[]
        for ns in sorted(set([0,1,500001,999999,1000000,1000001,1500001,1999999,2000000,2000001,2500001,2999999,3000000,3000001,3500001,3999999,4000000,end-1,end,end+1,18446744073709551615999999999])):
            remaining=[]
            for control,rate,ch,count,start in parts:
                first=min(count,max(0,((F(ns-start,10**9))*rate).__ceil__()))
                if first<count:remaining.append(dict(control=control,rate=rate,channels=ch,first=first,last=count,start_ns=start))
            anchor=remaining[0]['start_ns']+remaining[0]['first']*10**9//remaining[0]['rate']if remaining else end
            queries.append(dict(target_ns=str(ns),parts=remaining,anchor_ns=anchor))
        files[name]=edit_movie(data,edits,clock=10**9);cases.append(dict(file=name,queries=queries,duration_ns=end,scale=scale,video_frames=video))
    for stem in ['changing-raw','changing-rice','changing-raw-third','changing-rice-third']:
        data=source[stem+'.mp4']
        add(stem+'-split.mp4',data,[(1000000,0),(1000000,20000),(1000000,40000)],[(2,48000,2,48,0),(3,32000,1,32,1000000),(4,44100,2,45,2000000)],1000000,6)
        add(stem+'-silence.mp4',data,[(1000000,-1),(1000000,0),(1000000,20000)],[(8,48000,2,48,0),(2,48000,2,48,1000000),(3,32000,1,32,2000000)],1000000,6)
        add(stem+'-repeat.mp4',data,[(1000000,0),(1000000,20000)]*2,[(2,48000,2,48,0),(3,32000,1,32,1000000),(2,48000,2,48,2000000),(3,32000,1,32,3000000)],1000000,6)
    for stem,scale in [('alac-8000',8000),('alac-1000',1000),('alac-fill-prefix',8000)]:
        add(stem+'-split.mp4',source[stem+'.mp4'],[(1000000,0),(1000000,scale//100)],[(0,8000,1,8,0),(9,8000,1,8,1000000)],scale,3)
    files['active-short-body.mp4']=edit_movie(source['header-valid-body-short.mp4'],[(1000000,320)],clock=10**9)
    files['manifest.json']=(json.dumps(dict(provenance='Only authored ALAC raw/Rice/fill/8k ramps and48k stereo16/32k mono16/44.1k stereo24 packet references from companion factories;neutral VP9. Exact rational split,leading silence and repeated1ms edit windows. One separately declared active short-body refusal. No private data,FFmpeg or network.',cases=cases,refusal_file='active-short-body.mp4',sha256={n:hashlib.sha256(b).hexdigest()for n,b in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
