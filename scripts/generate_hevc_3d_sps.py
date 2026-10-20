"""Own complete SPS3D fields and short synthetic playback regressions, offline."""
import json,struct
from pathlib import Path
from generate_he_aac_packet_fixtures import DEST,boxes
from generate_hevc_extension_tail_fixtures import extended
from hevc_fixture_mp4 import box
OUT=DEST/'hevc-3d-sps'
def ue(v):
    bits=f'{v+1:b}';return '0'*(len(bits)-1)+bits
def payload(flags=None,sub=(0,0)):
    flags=[False]*14 if flags is None else flags
    f=''.join('1'if v else'0'for v in flags)
    return f[:2]+ue(sub[0])+f[2:6]+f[6:9]+ue(sub[1])+f[9:]
def generate():
    files={};syntax=[];videos=[]
    def add(name,flags,sub,cb):
        bits=payload(flags,sub);raw=bits+'0'*(-len(bits)%8)
        files[name+'.bin']=int(raw,2).to_bytes(len(raw)//8,'big')
        syntax.append(dict(name=name,bits=len(bits),flags=flags,sub=[v+3 for v in sub],cb=cb))
    add('inactive',[False]*14,(0,0),[3,6]);add('inactive-max',[False]*14,(3,3),[3,6])
    add('inactive-min4',[False]*14,(1,1),[4,6])
    for i in range(14):
        flags=[False]*14;flags[i]=True;add(f'flag{i}',flags,(0,0),[3,6])
    add('all-active',[True]*14,(3,3),[3,6])
    active=[False]*14;active[0]=True
    depth_only=[False]*14;depth_only[9]=True
    variants=[('inactive',payload(),False,None),('inactive-large',payload(sub=(2,2)),False,None),('inactive-future',payload()+'101001',True,None),('depth-only',payload(depth_only),False,None),('active-texture',payload(active),False,None),('inactive-base-vsp',payload([i==4 for i in range(14)]),False,None),('inactive-base-residual',payload([i==2 for i in range(14)]),False,None),('truncated','',False,'truncated HEVC 3D SPS extension'),('bad-size',payload(sub=(4,0)),False,'HEVC 3D sub-block size exceeds coding-block bounds')]
    for seedname in ['main-ipb','main10-ipb']:
        seed=(DEST.parent/'hevc'/f'{seedname}.mp4').read_bytes();roots=list(boxes(seed));assert roots[-1][0]==b'moov'
        for name,body,future,error in variants:
            def config(data):
                result=bytearray(data[:23]);at=23
                for _ in range(data[22]):
                    header=data[at:at+3];at+=3;result+=header
                    for _ in range(int.from_bytes(header[1:],'big')):
                        n=int.from_bytes(data[at:at+2],'big');at+=2;nal=data[at:at+n];at+=n
                        if header[0]&63==33:nal=extended(nal,0,body,extension_flags='0010'+('0001'if future else'0000'))
                        result+=struct.pack('>H',len(nal))+nal
                assert at==len(data);return bytes(result)
            def rewrite(tag,data):
                if tag in (b'moov',b'trak',b'mdia',b'minf',b'stbl'):data=b''.join(rewrite(t,p)for t,p in boxes(data))
                elif tag==b'stsd':data=data[:8]+b''.join(box(t,p[:78]+b''.join(box(k,config(v)if k==b'hvcC'else v)for k,v in boxes(p[78:])))for t,p in boxes(data[8:]))
                return box(tag,data)
            file=f'{seedname}-{name}.mp4';files[file]=b''.join(rewrite(t,p)for t,p in roots)
            videos.append(dict(file=file,seed=seedname,error=error))
    import generate_hevc_layered_sps as g
    target=g.fmt(chroma=0,depth=(8,8),size=(64,64));target['crop']=[0]*4
    files['depth-context.vps']=g.vps(target,depth_layer=True)[0]
    files['texture-context.sps']=g.sps(multilayer=False,independent=True)
    files['depth-active.sps']=g.sps(multilayer=True,sps_id=1,three_d=payload(depth_only))
    files['depth-unsupported-motion.sps']=g.sps(multilayer=True,sps_id=1,three_d=payload([i==8 for i in range(14)]))
    files['depth-unsupported-interview-motion.sps']=g.sps(multilayer=True,sps_id=1,three_d=payload([i==7 for i in range(14)]))
    files['depth-unsupported-disparity.sps']=g.sps(multilayer=True,sps_id=1,three_d=payload([i==6 for i in range(14)]))
    files['depth-inactive.sps']=g.sps(multilayer=True,sps_id=1,three_d=payload())
    files['depth-no-three-d.sps']=g.sps(multilayer=True,sps_id=1)
    files['scc-context.vps']=g.vps(g.fmt(chroma=3,separate=True,depth=(16,12)))[0]
    files['scc-three-d.sps']=g.sps(multilayer=True,sps_id=1,scc=True,three_d=payload())
    files['manifest.json']=(json.dumps(dict(provenance='Own complete Annex I SPS fields and unchanged authored synthetic I/P/B pictures. Syntax, depth-role admission and playback acceptance have separate tests.',syntax=syntax,videos=videos),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
