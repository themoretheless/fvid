"""Owned converted E-AC-3 syncframes and matching short video controls."""
import json
from pathlib import Path
from generate_eac3_playback import build_blksw_frame
from generate_eac3_bsid import identify, movie
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/eac3-converted'
def generate():
    files={};cases=[]
    for blocks in [1,2,3,6]:
        for kind in ['long','short','mixed']:
            pattern=[(0,0) if kind=='long' else (1,1) if kind=='short' else (i%2,1-i%2) for i in range(blocks)]
            for bsid in [11,16]:
                for mixing in [False,True]:
                    name=f'b{blocks}-{kind}-id{bsid}-mix{int(mixing)}'
                    raw=identify(build_blksw_frame(pattern,converted=True,mixing=mixing),bsid)
                    control=identify(build_blksw_frame(pattern),bsid)
                    files[name+'.ec3']=raw*3;files[name+'-control.ec3']=control*3
                    # Container packet timestamps use actual syncframe sample count.
                    files[name+'.mkv']=movie([raw]*3,blocks)
                    files[name+'-control.mkv']=movie([control]*3,blocks)
                    cases.append(dict(name=name,file=name+'.ec3',control=name+'-control.ec3',video=name+'.mkv',control_video=name+'-control.mkv',blocks=blocks))
    for blocks in [1,2,3]:
        for kind in ['long','short','mixed']:
            pattern=[(0,0) if kind=='long' else (1,1) if kind=='short' else (i%2,1-i%2) for i in range(blocks)]
            for bsid in [11,16]:
                name=f'b{blocks}-{kind}-id{bsid}-continuation'
                raw=identify(build_blksw_frame(pattern,converted=True,blkid=False),bsid)
                control=identify(build_blksw_frame(pattern),bsid)
                files[name+'.ec3']=raw*3;files[name+'-control.ec3']=control*3
                files[name+'.mkv']=movie([raw]*3,blocks);files[name+'-control.mkv']=movie([control]*3,blocks)
                cases.append(dict(name=name,file=name+'.ec3',control=name+'-control.ec3',video=name+'.mkv',control_video=name+'-control.mkv',blocks=blocks))
    invalid=[]
    for code in [38,63]:
        name=f'reserved-size{code}.ec3';files[name]=build_blksw_frame([(0,0)]*6,converted=True,frmsizecod=code);invalid.append(name)
    files['manifest.json']=(json.dumps(dict(cases=cases,invalid=invalid),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
