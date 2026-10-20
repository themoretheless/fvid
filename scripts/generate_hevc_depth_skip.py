"""Own skip-intra CABAC components, offline."""
import json
from generate_hevc_depth_dc import Encoder
from generate_hevc_wedge_tables import OUT
def generate():
    files={};cases=[]
    for name,swap,init in [('I',False,0),('P',False,1),('B',False,2),('P',True,2),('B',True,1)]:
        for qp in [0,26,51]:
            b=Encoder(init,qp)
            for value in [185,137]:
                pre=max(1,min(126,(((value>>4)*5-45)*qp>>4)+(value&15)*8-16))
                b.contexts.append([63-pre if pre<=63 else pre-64,int(pre>63)])
            blocks=[]
            for repeat in range(8):
                for skipped,enabled in [(True,True),(False,False),(False,True)]:
                    for mode in [None,0,1,2,3]:
                        active=not skipped and enabled and mode is not None
                        if not skipped and enabled:b.decision(3,active)
                        if active:
                            b.decision(4,mode!=0)
                            if mode:b.bypass(mode>1)
                            if mode>1:b.bypass(mode==3)
                        blocks.append(dict(skipped=skipped,enabled=enabled,mode=mode if active else None))
            file=f'depth-skip-{name}-swap{int(swap)}-qp{qp}.cabac'
            files[file]=b.finish();cases.append(dict(file=file,slice=name,swap=swap,qp=qp,blocks=blocks))
    files['depth-skip.json']=(json.dumps(dict(provenance='Own skip-intra component streams; not full pictures.',cases=cases),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
