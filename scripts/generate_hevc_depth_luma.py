"""Own interleaved Annex I prediction-block modes and two-pass ordinary syntax."""
import json
from generate_hevc_depth_dc import Encoder
from generate_hevc_wedge_tables import OUT,generate as wedges
def generate():
    counts={c['size']:c['count']for c in json.loads(wedges()['manifest.json'])['cases']}
    files={};cases=[]
    for name,swap,init in [('I',False,0),('P',False,1),('B',False,2),('P',True,2),('B',True,1)]:
        for qp in [0,26,51]:
            b=Encoder(init,qp)
            for value in [[154,141,155][init],154,[184,154,183][init]]:
                pre=max(1,min(126,(((value>>4)*5-45)*qp>>4)+(value&15)*8-16))
                b.contexts.append([63-pre if pre<=63 else pre-64,int(pre>63)])
            units=[]
            for log in range(2,6):
                for schedule in ['owco','cowo','wocw','oooo']:
                    enabled=schedule!='oooo';codes=[]
                    for i,mode in enumerate(schedule):
                        if enabled:
                            b.decision(3,mode=='o')
                            if mode!='o':b.decision(4,mode=='c')
                        if mode=='w':
                            index=counts[1<<log]-1 if i%2 else 0
                            for shift in reversed(range((counts[1<<log]-1).bit_length())):b.bypass((index>>shift)&1)
                            codes.append(dict(mode='wedge',index=index))
                        elif mode=='c':codes.append(dict(mode='contour'))
                        else:
                            mpm=i%2==0;value=2 if mpm else 31
                            b.decision(5,mpm);codes.append(dict(mode='ordinary',mpm=mpm,value=value))
                    for code in codes:
                        if code['mode']!='ordinary':continue
                        if code['mpm']:b.bypass(True);b.bypass(True)
                        else:
                            for shift in reversed(range(5)):b.bypass((code['value']>>shift)&1)
                    units.append(dict(log=log,enabled=enabled,codes=codes))
            file=f'depth-luma-{name}-swap{int(swap)}-qp{qp}.cabac';files[file]=b.finish()
            cases.append(dict(file=file,slice=name,swap=swap,qp=qp,units=units))
    files['depth-luma.json']=(json.dumps(dict(provenance='Owned mixed-PB CABAC components, not full video acceptance.',cases=cases),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
