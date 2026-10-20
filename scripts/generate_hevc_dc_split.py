"""Owned 64x64 DC-only CU forced into 16/32 prediction blocks, offline."""
import struct
from generate_hevc_depth_dc import Encoder
from generate_hevc_layered_sps import sps,ue,packed,escape
from generate_hevc_layered_pps import se
from generate_hevc_3d_sps import payload
from generate_hevc_wedge_tables import OUT
def generate():
    files={};flags=[False]*14;flags[10]=True
    for maximum in [2,3,4,5]:
        files[f'dc-split-tb{maximum}.sps']=sps(multilayer=True,three_d=payload(flags),coding_blocks=(3,6),transform_blocks=(2,maximum))
        for dlt in [False,True]:
            files[f"ordinary-dc-split-tb{maximum}{'-dlt'if dlt else ''}.y16"]=struct.pack('<4096H',*([136 if dlt else 129]*4096))
    b=Encoder(0,26)
    for value in [139,184,154,0]:
        pre=max(1,min(126,(((value>>4)*5-45)*26>>4)+(value&15)*8-16))
        b.contexts.append([63-pre if pre<=63 else pre-64,int(pre>63)])
    b.decision(3,False) # unsplit CU; no_dim inferred one at size 64
    b.decision(4,True);b.bypass(True);b.bypass(False) # ordinary DC MPM
    b.decision(5,True);b.decision(6,False) # DC-only, offset inferred +1
    bits='10'+ue(0)+ue(2)+'1'+'00000000'+'0'+se(0)+ue(0)+'1'
    bits+='0'*((-len(bits))%8)
    files['dc-split.nal']=((19<<9)|(3<<3)|1).to_bytes(2,'big')+escape(packed(bits)+b.finish())
    return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
