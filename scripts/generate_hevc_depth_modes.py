"""Own intra_mode_ext arithmetic component streams, offline."""
import json
from generate_hevc_depth_dc import Encoder
from generate_hevc_wedge_tables import generate as wedge_tables, OUT
def generate():
    counts={c['size']:c['count']for c in json.loads(wedge_tables()['manifest.json'])['cases']}
    files={};cases=[]
    for slice_name,swap,init in [('I',False,0),('P',False,1),('B',False,2),('P',True,2),('B',True,1)]:
        for qp in [0,26,51]:
            enc=Encoder(init,qp)
            for value in [[154,141,155][init],154]:
                pre=max(1,min(126,(((value>>4)*5-45)*qp>>4)+(value&15)*8-16))
                enc.contexts.append([63-pre if pre<=63 else pre-64,int(pre>63)])
            blocks=[]
            for log in range(2,7):
                for enabled in [[False,False],[True,False],[False,True],[True,True]]:
                    modes=['ordinary']if log==6 or not any(enabled)else ['ordinary']+(['wedge']if enabled[0]else[])+(['contour']if enabled[1]else[])
                    for mode in modes:
                        if log<6 and any(enabled):enc.decision(3,mode=='ordinary')
                        if mode!='ordinary' and all(enabled):enc.decision(4,mode=='contour')
                        index=0
                        if mode=='wedge':
                            index=counts[1<<log]-1
                            for shift in reversed(range(index.bit_length())):enc.bypass((index>>shift)&1)
                        blocks.append(dict(log=log,enabled=enabled,mode=mode,index=index))
            file=f'depth-modes-{slice_name}-swap{int(swap)}-qp{qp}.cabac'
            files[file]=enc.finish();cases.append(dict(file=file,slice=slice_name,swap=swap,qp=qp,blocks=blocks))
    files['depth-modes.json']=(json.dumps(dict(provenance='Own CABAC components, not full 3D picture acceptance.',cases=cases),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
