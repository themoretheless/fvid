#!/usr/bin/env python3
"""Authored v3 alpha slices, three context banks, independent RCT and opacity."""
import hashlib,json
from generate_ffv1_v3_configuration import config,sliced,D,TABLE
from generate_ffv1_v3_quantizer_switches import Writer,contexts,movie
from generate_ffv1_rice_fixtures import Rice,coded_line
def main():
    cases=[]
    for depth in [8,10,12,16]:
        maximum=(1<<depth)-1;bias=1<<depth
        for layout in ['gray','420','444','rgb']:
            for coder in [0,1,2]:
                rgb=layout=='rgb';chroma=layout!='gray';shift=(1,1) if layout=='420' else (0,0);sub=tuple(1<<s for s in shift)
                table=[0]+[min(255,max(1,n+int(n<240))) for n in TABLE[1:]] if coder==2 else TABLE
                private,initial,_=config(coder=coder,depth=depth,rgb=rgb,chroma=chroma,alpha=True,grid=(2,1),sx=shift[0],sy=shift[1],custom=table if coder==2 else None)
                name=f'ffv1-v3-alpha-{layout}-{depth}-coder{coder}';(D/(name+'.config')).write_bytes(private)
                histories={};previous={};packets=[];frames=[];keys=[True,False,False,True];w,h,sw=8,4,4
                for frame,key in enumerate(keys):
                    qa=[1,0,1,0][frame];selected=[0,1,qa];color=[0]*(w*h*(3 if rgb else 1));alphafull=[0]*(w*h);packet=b''
                    if chroma and not rgb:
                        cw,ch=w//sub[0],h//sub[1];chromafull=[[0]*(cw*ch) for _ in range(2)]
                    for order,sx in enumerate([0,1] if frame%2==0 else [1,0]):
                        if key:
                            entropy=Rice() if coder==0 else [[row.copy() for row in initial[q]] for q in selected]
                            if coder==0:entropy.models=[[dict(count=1,total=4,drift=0,bias=0) for _ in initial[q]] for q in selected]
                            histories[sx]=entropy
                        else:
                            entropy=histories[sx]
                            for group in range(3):
                                if len(initial[selected[group]])>len(initial[previous[sx][group]]):
                                    if coder==0:entropy.models[group]=[dict(count=1,total=4,drift=0,bias=0) for _ in initial[selected[group]]]
                                    else:entropy[group]=[[0]*32 for _ in initial[selected[group]]]
                        previous[sx]=selected
                        if rgb:
                            planes=[[],[],[]];sizes=[(sw,h)]*3
                            for y in range(h):
                                for x in range(sw):
                                    red=(x*17+y*13+sx*7+frame*19)&maximum;green=(x*31+y*5+sx*11+frame*3)&maximum;blue=(maximum-x*11-y*41-sx*23-frame*7)&maximum
                                    color[(y*w+sx*sw+x)*3:(y*w+sx*sw+x)*3+3]=[red,green,blue]
                                    cb=blue-green;cr=red-green
                                    for p,v in zip(planes,[green+((cb+cr)>>2),cb+bias,cr+bias]):p.append(v)
                        else:
                            luma=[(i*53+frame*17+sx*7)&maximum for i in range(sw*h)];planes=[luma];sizes=[(sw,h)]
                            for y in range(h):color[y*w+sx*sw:y*w+sx*sw+sw]=luma[y*sw:(y+1)*sw]
                            if chroma:
                                pw,ph=sw//sub[0],h//sub[1]
                                for p in range(2):
                                    values=[(i*97+p*maximum//2+frame*31+sx*17)&maximum for i in range(pw*ph)];planes.append(values);sizes.append((pw,ph))
                                    for y in range(ph):chromafull[p][y*cw+sx*pw:y*cw+sx*pw+pw]=values[y*pw:(y+1)*pw]
                        zero=coder!=0 and all(not any(row) for row in entropy[2])
                        alpha=[x+y+1 if zero else [0,maximum,1,maximum-1,maximum//2,2+frame,maximum//3,0][(y*sw+x+sx+frame)%8] for y in range(h) for x in range(sw)]
                        for y in range(h):alphafull[y*w+sx*sw:y*w+sx*sw+sw]=alpha[y*sw:(y+1)*sw]
                        planes.append(alpha);sizes.append((sw,h));r=Writer(table)
                        if order==0:r.bit([128],0,key)
                        header=[128]*32
                        for v in [sx,0,0,0,*selected,3,1,1]:r.integer(header,v)
                        if coder==0:entropy.bits='';r.bit([129],0,False)
                        run=[0]*4;bits=depth+int(rgb)
                        sequence=[(p,y) for y in range(h) for p in range(4)] if rgb else [(p,y) for p in range(len(planes)) for y in range(sizes[p][1])]
                        for p,y in sequence:
                            group=2 if p==len(planes)-1 else int(p!=0);pw,_=sizes[p]
                            diffs,_=coded_line(planes[p],pw,y,bits,False);ctx=contexts(planes[p],pw,y,bits,selected[group]);diffs=[-v if c<0 else v for v,c in zip(diffs,ctx)]
                            if coder==0:run[p]=entropy.line(diffs,ctx,bits,group,run[p])
                            else:
                                for v,c in zip(diffs,ctx):r.integer(entropy[group][abs(c)],v,True)
                        packet+=sliced(r.finish()+(entropy.finish() if coder==0 else b''),True)
                    if not rgb:color += chromafull[0]+chromafull[1] if chroma else [1<<(depth-1)]*(2*w*h)
                    serialize=lambda v:bytes(v) if depth==8 else b''.join(n.to_bytes(2,'little') for n in v)
                    paths={}
                    for extension,data in [('packet',packet),('color',serialize(color)),('alpha',serialize(alphafull))]:
                        file=f'{name}-{frame}.{extension}';(D/file).write_bytes(data);paths[extension]=file;paths[extension+'_sha256']=hashlib.sha256(data).hexdigest()
                    frames.append(dict(paths,keyframe=key));packets.append(packet)
                file=name+'.mkv';data=movie(private,packets,keys,w,h);(D/file).write_bytes(data)
                cases.append(dict(file=file,configuration=name+'.config',size=[w,h],version=3,coder=coder,depth=depth,layout=layout,frames=frames,sha256=hashlib.sha256(data).hexdigest(),configuration_sha256=hashlib.sha256(private).hexdigest()))
    (D/'ffv1-v3-alpha.json').write_text(json.dumps(dict(cases=cases,provenance='Original RGBA/YUVA/gray-alpha sample arrays, independent Python range/Rice coding and Green-base RCT. Two slices, separate alpha context bank, nonkeyframe alpha quantizer growth/shrink, keyframe recovery. Codec API acceptance only; presentation/export filters are separate. No private media, external codec or network.'),indent=2)+'\n')
if __name__=='__main__':main()
