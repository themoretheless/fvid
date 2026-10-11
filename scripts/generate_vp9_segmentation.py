#!/usr/bin/env python3
"""Owned VP9 segmentation maps, temporal reuse and forced-reference/skip streams."""
import json
from generate_vp9_chroma import BooleanWriter, DEST, coloured, table
from generate_vp9_compound import profile_bits, literal, neutral_intra, webm
TREE=[2,4,6,8,10,12,0,-1,-2,-3,-4,-5,-6,-7]
PROBS=[91,137,203,73,166,119,219]
def symbol(b,t,p,value):
    path=find(t,value)
    if path is None:raise ValueError(value)
    for node,bit in path:b.write(p[node//2],bit)
def find(t,value,node=0):
    for bit in [0,1]:
        child=t[node+bit]
        if child<=0 and -child==value:return [(node,bit)]
        if child>0:
            path=find(t,value,child)
            if path is not None:return [(node,bit)]+path
    return None

def segmentation(update,temporal,data,absolute=False,enabled=True,force_intra=False):
    if not enabled:return '0'
    bits='1'+str(int(update))
    if update:
        bits+=''.join('1'+f'{p:08b}' for p in PROBS)+str(int(temporal))
        if temporal:bits+=''.join('1'+f'{p:08b}' for p in [87,149,211])
    bits+=str(int(data))
    if data:
        bits+=str(int(absolute))
        for i in range(8):
            # Every segment has a visible forced reference and skip tool;
            # segment ID changes therefore alter actual reconstructed pixels.
            bits+='00'+'1'+f'{0 if force_intra else 1+i%2:02b}'+'1'
    return bits

def packet(depth,chroma,ids,update=True,temporal=False,data=True,reuse=None,enabled=True,split=True,shown=True,force_intra=False,side=64,error_resilient=False):
    bits,_=profile_bits(depth,chroma)
    bits+=('011' if shown else '010')+str(int(error_resilient))
    if not shown:bits+='0'  # inter, not intra-only
    if not error_resilient:bits+='00'
    bits+='00000000'
    # LAST is coloured slot0; GOLDEN is neutral slot1; ALT is slot0.
    bits+='0000'+'0010'+'0000'
    bits+=('1' if side==64 else '000'+f'{side-1:016b}{side-1:016b}')+'0'+'0'+'011'
    c=BooleanWriter();c.write(128,0)
    for _ in range(3+21+4):c.write(252,0)
    # uniform sign biases imply SINGLE_REFERENCE: no reference-mode bits.
    for _ in range(10+36+48+3+44+18):c.write(252,0)
    b=BooleanWriter();partition=table('DEFAULT_PARTITION_PROBS')
    if split:
        symbol(b, [0,2,-1,4,-2,-3], partition[12],3)
    above=[0]*8;left=[0]*8
    for j,seg in enumerate(ids):
        row=(j//2)*4 if split else 0;col=(j%2)*4 if split else 0
        # Previous split64 sets no child contexts until each child completes:
        # all unsplit32 children have partition context8.
        symbol(b,[0,2,-1,4,-2,-3],partition[8 if split or side==32 else 12],0)
        if enabled and update:
            if temporal:
                pred=reuse[j];b.write([87,149,211][above[col]+left[row]],pred)
                above[col:col+(4 if split else 8)]=[int(pred)]*(4 if split else 8)
                left[row:row+(4 if split else 8)]=[int(pred)]*(4 if split else 8)
                if not pred:symbol(b,TREE,PROBS,seg)
            else:symbol(b,TREE,PROBS,seg)
        if enabled and force_intra:
            b.write(table('DEFAULT_Y_MODE_PROBS')[3][0],0)
            b.write(table('DEFAULT_UV_MODE_PROBS')[0][0],0)
        if not enabled:
            # Ordinary skipped inter block, explicitly select LAST and ZEROMV.
            b.write(table('DEFAULT_SKIP_PROB')[0 if j==0 else 1 if j<3 else 2],1)
            b.write(table('DEFAULT_IS_INTER_PROB')[0],1)
            b.write(table('DEFAULT_SINGLE_REF_PROB')[2][0],0)
            b.write(table('DEFAULT_INTER_MODE_PROBS')[2][0],0)
    compressed=c.finish();tile=b.finish()
    bits+=('00' if error_resilient else '0100')+'0'*10+'0'*8+'000'+segmentation(update,temporal,data,enabled=enabled,force_intra=force_intra)+'0'+f'{len(compressed):016b}'
    return literal(bits)+compressed+tile

def oracle(reference,depth,chroma,ids,split=True):
    sub={'420':(1,1),'422':(1,0),'444':(0,0),'440':(0,1)}[chroma]
    out=bytearray();at=0
    for plane in range(3):
        sx,sy=sub if plane else (0,0);w,h=64>>sx,64>>sy
        for y in range(h):
            for x in range(w):
                i=(y//(32>>sy))*2+x//(32>>sx) if split else 0
                value=(1<<(depth-1)) if ids[i]%2 else int.from_bytes(reference[at:at+2],'little')
                out+=value.to_bytes(2,'little');at+=2
    return out

def main():
    cases=[]
    quant_cases=[]
    filter_cases=[]
    reset_cases=[]
    lossy_cases=[]
    for depth in [8,10,12]:
        for chroma in ['420','422','444','440']:
            key,ref=coloured(depth,chroma)
            # All eight segment leaves, mixed predictor contexts, map preservation
            # through a disabled frame, hidden updates and min-map block aggregation.
            stages=[dict(ids=[7,6,4,5]),dict(ids=[7,6,4,2],temporal=True,data=False,reuse=[True,True,True,False],shown=False),
                dict(ids=[0],enabled=False,data=False,split=False),dict(ids=[7,6,4,2],update=False,data=False),
                dict(ids=[2],update=False,data=False,split=False),dict(ids=[0,1,3,4],data=False),dict(ids=[0],split=False,force_intra=True)]
            packets=[key,neutral_intra(depth,chroma)];expected=[]
            for i,stage in enumerate(stages):
                packets.append(packet(depth,chroma,**stage))
                name=f'vp9-segmentation-{chroma}-{depth}-{i}.yuv16'
                (DEST/name).write_bytes(oracle(ref,depth,chroma,[1] if stage.get('force_intra') else stage['ids'],stage.get('split',True)))
                expected.append(dict(oracle=name,pts=(i+2)*333000000,shown=stage.get('shown',True)))
            name=f'vp9-segmentation-{chroma}-{depth}.webm';(DEST/name).write_bytes(webm(packets))
            cases.append(dict(file=name,depth=depth,chroma=chroma,frames=expected))
            for kind in ['error-resilient','resize']:
                reset=packet(depth,chroma,[0],split=False,update=False,data=True,
                    side=32 if kind=='resize' else 64,error_resilient=kind=='error-resilient')
                sequence=[key,neutral_intra(depth,chroma),packet(depth,chroma,[1],split=False),reset,
                    packet(depth,chroma,[0],split=False,update=False,data=False)]
                stem=f'vp9-segmentation-reset-{chroma}-{depth}-{kind}'
                (DEST/f'{stem}.webm').write_bytes(webm(sequence))
                expected=bytearray();at=0
                sub={'420':(1,1),'422':(1,0),'444':(0,0),'440':(0,1)}[chroma]
                for plane in range(3):
                    sx,sy=sub if plane else (0,0);w,h=64>>sx,64>>sy
                    if kind=='resize':
                        for y in range(h//2):
                            for x in range(w//2):expected+=ref[at+((2*y)*w+2*x)*2:at+((2*y)*w+2*x)*2+2]
                    else:expected+=ref[at:at+w*h*2]
                    at+=w*h*2
                (DEST/f'{stem}.yuv16').write_bytes(expected)
                (DEST/f'{stem}-return.yuv16').write_bytes(ref)
                reset_cases.append(dict(file=f'{stem}.webm',oracle=f'{stem}.yuv16',returned=f'{stem}-return.yuv16',depth=depth,chroma=chroma,kind=kind))
            for base,q,absolute in [(20,6,False),(20,6,True),(20,-100,False),(250,100,False),(20,-100,True),(20,255,True)]:
                seg='11'+''.join('1'+f'{p:08b}' for p in PROBS)+'0'+'1'+str(int(absolute))
                seg+='1'+f'{abs(q):08b}'+str(int(q<0))+'000'+'0000'*7
                index=max(0,min(255,q+(0 if absolute else base)))
                quant=table('DC_QLOOKUP')[(depth-8)//2][index]
                picture,expected=coloured(depth,chroma,segment=seg,dc_quant=quant,base_q=base)
                stem=f'vp9-segmentation-lossy-{chroma}-{depth}-{base}-{q}-{int(absolute)}'
                (DEST/f'{stem}.vp9').write_bytes(picture)
                (DEST/f'{stem}.yuv16').write_bytes(expected)
                lossy_cases.append(dict(file=f'{stem}.vp9',oracle=f'{stem}.yuv16',base=base,q=q,absolute=absolute,index=index))
            for absolute in [False,True]:
                for q in [-12,6,255]:
                    # Explicit ALT_Q only on segment zero; base_q remains zero.
                    seg='11'+''.join('1'+f'{p:08b}' for p in PROBS)+'0'+'1'+str(int(absolute))
                    seg+='1'+f'{abs(q):08b}'+str(int(q<0))+'000'+'0000'*7
                    quant=table('DC_QLOOKUP')[(depth-8)//2][max(0,q)]
                    picture,expected=coloured(depth,chroma,segment=seg,dc_quant=quant)
                    stem=f'vp9-segmentation-q-{chroma}-{depth}-{int(absolute)}-{q}'
                    (DEST/f'{stem}.vp9').write_bytes(picture)
                    (DEST/f'{stem}.yuv16').write_bytes(expected)
                    quant_cases.append(dict(file=f'{stem}.vp9',oracle=f'{stem}.yuv16',depth=depth,chroma=chroma,q=q,absolute=absolute))
            for absolute,base,delta in [(False,16,-16),(False,16,-63),(False,16,63),(True,16,0),(True,16,32),(False,0,32)]:
                seg='11'+''.join('1'+f'{p:08b}' for p in PROBS)+'0'+'1'+str(int(absolute))
                q={8:11,10:13,12:7}[depth]
                qbits='1'+f'{q:08b}'+'0'
                prefix='11'+''.join('1'+f'{p:08b}' for p in PROBS)+'0'+'1'
                seg+=qbits+'1'+f'{abs(delta):06b}'+str(int(delta<0))+'00'+'0000'*7
                quant=table('DC_QLOOKUP')[(depth-8)//2][q]
                control_seg=prefix+str(int(absolute))+qbits+'000'+'0000'*7
                level=max(0,min(63,delta+(0 if absolute else base))) if base else 0
                picture,unfiltered=coloured(depth,chroma,segment=seg,dc_quant=quant,level=base)
                control,_=coloured(depth,chroma,segment=control_seg,dc_quant=quant,level=level)
                stem=f'vp9-segmentation-lf-{chroma}-{depth}-{int(absolute)}-{base}-{delta}'
                (DEST/f'{stem}.vp9').write_bytes(picture)
                (DEST/f'{stem}-control.vp9').write_bytes(control)
                (DEST/f'{stem}.yuv16').write_bytes(unfiltered)
                filter_cases.append(dict(file=f'{stem}.vp9',control=f'{stem}-control.vp9',unfiltered=f'{stem}.yuv16',level=level))
    (DEST/'vp9-segmentation-lossy.json').write_text(json.dumps(dict(cases=lossy_cases),indent=2)+'\n')
    (DEST/'vp9-segmentation-reset.json').write_text(json.dumps(dict(cases=reset_cases),indent=2)+'\n')
    (DEST/'vp9-segmentation-filter.json').write_text(json.dumps(dict(cases=filter_cases),indent=2)+'\n')
    (DEST/'vp9-segmentation-quant.json').write_text(json.dumps(dict(cases=quant_cases),indent=2)+'\n')
    (DEST/'vp9-segmentation.json').write_text(json.dumps(dict(cases=cases,provenance='Original synthetic coloured and neutral references; owned segmentation syntax/entropy writer, independent reference-selection pixel oracles. No private media, external codecs or network.'),indent=2)+'\n')
if __name__=='__main__':main()
