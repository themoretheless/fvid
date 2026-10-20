"""Own CRA contour/DC-only entropy and independent synthetic texture oracle."""
import struct
from generate_hevc_depth_dc import Encoder
from generate_hevc_layered_sps import sps
from generate_hevc_3d_sps import payload
from generate_hevc_wedge_tables import OUT
from generate_he_aac_packet_fixtures import DEST

def generate(residual=None,mode="contour",offset=10,invalid_split=False):
    assert mode in ("contour","wedge") and residual in (None,0,64,-64)
    assert offset in (10,150) and (not invalid_split or residual==0)
    flags=[False]*14;flags[9]=flags[10]=True
    b=Encoder(0,26)
    for value in [139,154,154,0,154,154]:
        pre=max(1,min(126,(((value>>4)*5-45)*26>>4)+(value&15)*8-16))
        b.contexts.append([63-pre if pre<=63 else pre-64,int(pre>63)])
    b.decision(3,False) # full partition
    b.decision(4,False) # no_dim_flag
    b.decision(8,mode=="contour") # contour rather than wedge
    if mode=="wedge":
        for _ in range(9):b.bypass(False) # indexed 32x32 wedge zero
    b.decision(5,residual is None) # dc_only_flag
    if residual is None:b.decision(6,True) # only DC-only signals presence
    for negative in [True,False]:
        for _ in range(3):b.decision(7,True)
        eg=f'{offset-3+1:b}'
        for bit in '1'*(len(eg)-1)+'0'+eg[1:]:b.bypass(int(bit))
        b.bypass(negative)
    if residual is not None:
        # I-slice ctxIdx init values: split32, CBF depth0, lastX/Y
        # (log=5 offset10), greater1 ctx1, greater2 ctx0.
        for value in [153,141,111,111,92,138,138,111]:
            pre=max(1,min(126,(((value>>4)*5-45)*26>>4)+(value&15)*8-16))
            b.contexts.append([63-pre if pre<=63 else pre-64,int(pre>63)])
        b.decision(9,invalid_split) # split=1 violates the DIM constraint
        if invalid_split:
            for _ in range(4):
                b.decision(15,False);b.decision(16,False) # four uncoded 16x16 leaves
        else:b.decision(10,residual!=0)
        if residual:
            b.decision(11,False);b.decision(12,False) # last coefficient (0,0)
            b.decision(13,True);b.decision(14,True) # magnitude at least three
            b.bypass(residual<0)
            for bit in '11111111011011':b.bypass(int(bit)) # Rice0 remainder61 => abs64
    texture=(DEST/'hevc-pcm-reference-rext8.yuv').read_bytes()[:4096]
    threshold=sum(texture[i]for i in [0,31,31*64,31*64+31])//4
    mask=[texture[y*64+x]>threshold for y in range(32)for x in range(32)]
    if mode=="wedge":
        from generate_hevc_wedge_tables import generate as wedges
        mask=list(wedges()['32.bin'][:1024])
    assert any(mask) and not all(mask)
    # Independent DC-only DCT oracle: inverse quantization at QP26 gives
    # +/-3264, first inverse pass +/-1632, final uniform residual +26/-25 after signed rounding.
    adjustment=0 if not residual else (26 if residual>0 else -25)
    # I-94 clips to the 8-bit index domain, not the 32 populated entries.
    # Unpopulated DltIdxToVal entries are inferred zero (I.7.4.3.3.5).
    def dlt_value(segment):
        index=max(0,min(255,16+(offset if segment else -offset)))
        return index*8 if index<32 else 0
    return {'contour-picture.sps':sps(multilayer=True,three_d=payload(flags)),
        'contour-picture.cabac':b.finish(),
        'contour-picture.y16':struct.pack('<1024H',*(max(0,min(255,(128+offset if v else 128-offset)+adjustment)) for v in mask)),
        'contour-dlt.y16':struct.pack('<1024H',*(max(0,min(255,dlt_value(v)+adjustment)) for v in mask))}
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)

    for mode in ["contour","wedge"]:
        for residual,label in [(0,"zero"),(64,"positive"),(-64,"negative")]:
            for n,b in generate(residual,mode).items():
                if n.endswith('.sps'):continue
                suffix=n.removeprefix('contour-')
                (OUT/f'{mode}-residual-{label}-{suffix}').write_bytes(b)

        for residual,label in [(64,"positive"),(-64,"negative")]:
            for n,b in generate(residual,mode,150).items():
                if n.endswith('.sps'):continue
                (OUT/f'{mode}-residual-{label}-clip-{n.removeprefix("contour-")}').write_bytes(b)
        invalid=generate(0,mode,invalid_split=True)
        (OUT/f'{mode}-residual-split.cabac').write_bytes(invalid['contour-picture.cabac'])
