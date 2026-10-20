"""Own parsed depth-layer IDR pictures, including VPS/SPS/PPS and slice syntax."""
from generate_hevc_layered_sps import vps,sps,fmt,ue,packed,escape
from generate_hevc_layered_slices import header_pps
from generate_hevc_layered_pps import se
from generate_hevc_3d_sps import payload
from generate_hevc_single_cu import generate as entropy
from generate_hevc_wedge_tables import OUT
def generate():
    files={};coded=entropy()
    flags=[False]*14;flags[-1]=True
    for size in [64,128]:
        target=fmt(chroma=0,depth=(8,8),size=(size,size));target['crop']=[0]*4
        files[f'skip-{size}.vps']=vps(target,depth_layer=True)[0]
        files['skip-texture.sps']=sps(multilayer=False,independent=True)
        files['skip-depth.sps']=sps(multilayer=True,three_d=payload(flags),coding_blocks=(3,6))
        files['skip-depth.pps']=header_pps(3,0,False,False)
        for mode in range(4):
            bits='10'+ue(0)+ue(2)+'1'+'00000000'+'0'+se(0)+ue(0)+'1'
            bits+='0'*((-len(bits))%8)
            raw=packed(bits)+coded[f"single-cu-{mode}{'-128'if size==128 else ''}.cabac"]
            files[f'skip-{size}-mode{mode}.nal']=((19<<9)|(3<<3)|1).to_bytes(2,'big')+escape(raw)
    return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
