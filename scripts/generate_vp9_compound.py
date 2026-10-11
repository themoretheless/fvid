#!/usr/bin/env python3
"""Owned VP9 distinct-reference compound acceptance videos; offline."""
import json
from generate_vp9_chroma import BooleanWriter, DEST, coloured, table
from generate_vp9_chroma_inter import literal, vector, oracle as motion_oracle
from generate_audio_resample_window_fixture import ebml

def profile_bits(depth,chroma):
    p=(0 if chroma=='420' else 1)+(2 if depth>8 else 0)
    return '10'+str(p&1)+str(p>>1)+('0' if p==3 else ''),p

def tail(header,compressed,tile):
    header+='0100'+'0'*10+'0'*8+'000'+'0'+'0'+f'{len(compressed):016b}'
    return literal(header)+compressed+tile

def neutral_intra(depth,chroma):
    bits,p=profile_bits(depth,chroma)
    bits+='0100'+'1'+'00'+f'{0x498342:024b}'
    if p:
        if depth>8:bits+=str(int(depth==12))
        bits+='0010'
        if p&1:bits+={'422':'100','444':'000','440':'010'}[chroma]
    bits+=f'{2:08b}'+f'{63:016b}{63:016b}'+'0'
    return tail(bits,bytes(32),bytes(4096))

def compound(depth,chroma,choice,select,group=0,moving=False,nearest=False):
    fixed,var=[(3,[1,2]),(2,[1,3]),(1,[2,3])][group]
    idx=int(not select)
    bias=[not bool(idx)]*3;bias[fixed-1]=bool(idx)
    slots=[0,0,0];slots[var[1]-1]=1
    bits,_=profile_bits(depth,chroma)
    bits+='0110'+'00'+'00000000'
    for slot,sign in zip(slots,bias):bits+=f'{slot:03b}'+str(int(sign))
    bits+='1'+'0'+'0'+'011'
    c=BooleanWriter();c.write(128,0)
    for _ in range(3+21+4):c.write(252,0)
    c.write(128,1);c.write(128,int(select or nearest))
    for _ in range((5+10 if select or nearest else 0)+5+36+48+3+44+18):c.write(252,0)
    b=BooleanWriter()
    b.write(table('DEFAULT_PARTITION_PROBS')[12][0],0)
    b.write(table('DEFAULT_SKIP_PROB')[0],1)
    b.write(table('DEFAULT_IS_INTER_PROB')[0],1)
    if select or nearest:b.write(table('DEFAULT_COMP_MODE_PROB')[1],int(not nearest))
    if nearest:
        refs=[0,0];refs[idx]=fixed;refs[1-idx]=var[choice]
        target=refs[1]
        b.write(table('DEFAULT_SINGLE_REF_PROB')[2][0],int(target!=1))
        if target!=1:b.write(table('DEFAULT_SINGLE_REF_PROB')[2][1],int(target==3))
    else:b.write(table('DEFAULT_COMP_REF_PROB')[2],choice)
    mode=table('DEFAULT_INTER_MODE_PROBS')[2]
    if nearest:b.write(mode[0],1);b.write(mode[1],0)
    elif moving:
        for prob in mode:b.write(prob,1)
        vector(b,[6,-10]);vector(b,[16,-16])
    else:b.write(mode[0],0)
    return tail(bits,c.finish(),b.finish())

def webm(packets):
    track=ebml('d7',b'\1')+ebml('83',b'\1')+ebml('86',b'V_VP9')+ebml('23e383',(333333333).to_bytes(4,'big'))
    track+=ebml('e0',ebml('b0',b'\x40')+ebml('ba',b'\x40'))
    cluster=ebml('e7',b'\0')
    for i,p in enumerate(packets):cluster+=ebml('a3',b'\x81'+(i*333).to_bytes(2,'big')+bytes([128 if i==0 else 0])+p)
    info=ebml('1549a966',ebml('2ad7b1',(1000000).to_bytes(3,'big')))
    return ebml('1a45dfa3',ebml('4282',b'webm'))+ebml('18538067',info+ebml('1654ae6b',ebml('ae',track))+ebml('1f43b675',cluster))
def main():
    cases=[]
    for depth in [8,10,12]:
        for chroma in ['420','422','444','440']:
            key,reference=coloured(depth,chroma)
            neutral=b''.join((1<<(depth-1)).to_bytes(2,'little') for _ in range(len(reference)//2))
            for group in range(3):
                for select in [False,True]:
                    for choice in [0,1]:
                        for moving in [False,True]:
                            idx=int(not select)
                            inputs=[None,None];inputs[idx]=reference;inputs[1-idx]=neutral if choice else reference
                            mvs=[[6,-10],[16,-16]] if moving else [[0,0],[0,0]]
                            predictions=[motion_oracle(p,depth,chroma,mv) for p,mv in zip(inputs,mvs)]
                            expected=bytearray()
                            for i in range(0,len(reference),2):
                                a=int.from_bytes(predictions[0][i:i+2],'little');b=int.from_bytes(predictions[1][i:i+2],'little')
                                expected+=((a+b+1)//2).to_bytes(2,'little')
                            stem=f'vp9-compound-{chroma}-{depth}-{group}-{int(select)}-{choice}-{int(moving)}'
                            packets=[key,neutral_intra(depth,chroma),compound(depth,chroma,choice,select,group,moving)]
                            continuation=None
                            if moving:
                                packets.append(compound(depth,chroma,choice,select,group,moving,nearest=True))
                                continuation=f'{stem}-nearest.yuv16'
                                (DEST/continuation).write_bytes(predictions[1])
                            (DEST/f'{stem}.webm').write_bytes(webm(packets))
                            (DEST/f'{stem}.yuv16').write_bytes(expected)
                            cases.append(dict(file=f'{stem}.webm',oracle=f'{stem}.yuv16',continuation_oracle=continuation,depth=depth,chroma=chroma,select=select,choice=choice,fixed=[3,2,1][group],fixed_index=idx,moving=moving))
    (DEST/'vp9-compound.json').write_text(json.dumps(dict(cases=cases,provenance='Original coloured keyframe A, hidden neutral intra B refreshing slot1, compound prediction from A/A or A/B. All six sign-bias patterns, both variable choices, forced and select modes, two independently coded motion lists, and select-single NEAREST continuation from the previous second list. Independent interpolation and rounded-average pixel oracles. No external codec, FFmpeg, private media or network.'),indent=2)+'\n')
if __name__=='__main__':main()
