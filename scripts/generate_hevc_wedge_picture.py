"""Own parsed monochrome depth wedge/DC-only IDR picture, offline."""
import struct
from generate_hevc_depth_dc import Encoder
from generate_hevc_layered_sps import vps,sps,fmt,ue,packed,escape
from generate_hevc_layered_slices import header_pps
from generate_hevc_layered_pps import se
from generate_hevc_3d_sps import payload
from generate_hevc_wedge_tables import OUT,generate as wedges
def generate():
    files={};target=fmt(chroma=0,depth=(8,8),size=(32,32));target['crop']=[0]*4
    flags=[False]*14;flags[10]=True
    files['wedge-picture.vps']=vps(target,depth_layer=True)[0]
    files['wedge-picture.sps']=sps(multilayer=True,three_d=payload(flags))
    files['wedge-picture.pps']=header_pps(3,0,False,False)
    from generate_hevc_depth_tables import header,delta
    tables=header(8,1)+'100'+delta(8,list(range(0,256,8)))
    files['wedge-dlt.pps']=header_pps(3,0,False,False,depth_tables=tables)
    b=Encoder(0,26)
    for value in [139,154,154,0,154]:
        pre=max(1,min(126,(((value>>4)*5-45)*26>>4)+(value&15)*8-16))
        b.contexts.append([63-pre if pre<=63 else pre-64,int(pre>63)])
    b.decision(3,False);b.decision(4,False)
    for _ in range(9):b.bypass(False) # index zero in the 510-mask domain
    b.decision(5,True);b.decision(6,True)
    for negative in [True,False]:
        for _ in range(3):b.decision(7,True)
        for bit in [1,1,1,0,0,0,0]:b.bypass(bit) # EG0(10-3)
        b.bypass(negative)
    bits='10'+ue(0)+ue(2)+'1'+'00000000'+'0'+se(0)+ue(0)+'1'
    bits+='0'*((-len(bits))%8)
    files['wedge-picture.nal']=((19<<9)|(3<<3)|1).to_bytes(2,'big')+escape(packed(bits)+b.finish())
    mask=wedges()['32.bin'][:1024]
    files['wedge-picture.y16']=struct.pack('<1024H',*(138 if bit else 118 for bit in mask))
    files['wedge-dlt.y16']=struct.pack('<1024H',*(208 if bit else 48 for bit in mask))
    for present in [False,True]:
        dc=Encoder(0,26)
        for value in [139,154,154,0,154,184]:
            pre=max(1,min(126,(((value>>4)*5-45)*26>>4)+(value&15)*8-16))
            dc.contexts.append([63-pre if pre<=63 else pre-64,int(pre>63)])
        dc.decision(3,False);dc.decision(4,True) # no_dim
        dc.decision(8,True);dc.bypass(True);dc.bypass(False) # ordinary MPM index 1: DC
        dc.decision(5,True);dc.decision(6,present)
        if present:
            for _ in range(3):dc.decision(7,True)
            for bit in [1,1,0,1,1]:dc.bypass(bit) # EG0(9-3)
            dc.bypass(False)
        label='present'if present else'absent'
        files[f'ordinary-dc-{label}.nal']=((19<<9)|(3<<3)|1).to_bytes(2,'big')+escape(packed(bits)+dc.finish())
        files[f'ordinary-dc-{label}.y16']=struct.pack('<1024H',*([138 if present else 129]*1024))
        files[f'ordinary-dc-{label}-dlt.y16']=struct.pack('<1024H',*([208 if present else 136]*1024))
    return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
