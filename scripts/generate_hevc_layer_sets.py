#!/usr/bin/env python3
"""Original VPS base-syntax layer-set/HRD metadata fixtures, offline.

These are metadata acceptance cases, not full Annex F video acceptance.
"""
import json
from generate_hevc_colour_plane_fixtures import packed,escape
from generate_hevc_poc_reset_fixtures import ue
from generate_he_aac_packet_fixtures import DEST

def vps(maximum=2,max_id=7,sets=(9,128),internal=True,available=True,hrds=(),fault=None):
    # Original Main PTL; the opaque Annex F extension is deliberately absent.
    bits='0000'+str(int(internal))+str(int(available))+f'{maximum:06b}'+'0001'+'1'*16
    bits+='00000001'+f'{1<<30:032b}'+'0'*48+f'{120:08b}'
    bits+=('1'+ue(2)+ue(1)+ue(0)) if internal else '0'+ue(0)*3
    bits+=f'{max_id:06b}'+ue(1024 if fault=='set-count' else len(sets))
    if fault=='set-count':return b'\x40\x01'+escape(packed(bits+'1'))
    for mask in sets:bits+=''.join(str((mask>>i)&1) for i in range(max_id+1))
    bits+=str(int(bool(hrds)))
    if hrds:
        bits+=f'{1001:032b}{30000:032b}'+'0'+ue(len(sets)+2 if fault=='hrd-count' else len(hrds))
        for i,(index,present,nal,vcl,scale) in enumerate(hrds):
            bits+=ue(index)
            if i:bits+=str(int(present))
            if i==0 or present:
                bits+=str(int(nal))+str(int(vcl))
                if nal or vcl:bits+='0'+f'{scale:04b}{scale+1:04b}'+'00000'*3
            bits+='1'+ue(i)+ue(0)
            if nal:bits+=ue(i+1)+ue(i+2)+'1'
            if vcl:bits+=ue(i+3)+ue(i+4)+'0'
    bits+='0'+'1' # no VPS extension; RBSP trailing one
    result=b'\x40\x01'+escape(packed(bits))
    return result[:-2] if fault=='truncate' else result

def main():
    cases=[]
    specs=[
      ('sparse',{},None),
      ('reserved-ids',dict(maximum=63,max_id=63,sets=(1|(1<<63),(1<<64)-1)),None),
      ('maximum-sets',dict(maximum=63,max_id=10,sets=tuple(range(2,1025))),None),
      ('inherited-hrd',dict(maximum=1,max_id=3,sets=(8,9),hrds=((0,True,True,True,0),(1,False,True,True,0),(2,True,False,True,6))),None),
      ('external-base',dict(maximum=1,max_id=3,sets=(8,),internal=False,hrds=((1,True,True,False,0),)),None),
      ('unavailable-base',dict(available=False),None),
      ('empty-set',dict(sets=(0,)),'cardinality'),
      ('duplicate-set',dict(sets=(1,)),'duplicate'),
      ('too-many-members',dict(maximum=1,sets=(7,)),'cardinality'),
      ('bad-set-count',dict(fault='set-count'),'count exceeds'),
      ('bad-hrd-count',dict(hrds=((0,True,True,False,0),),fault='hrd-count'),'HRD count'),
      ('duplicate-hrd',dict(hrds=((0,True,True,False,0),(0,False,True,False,0))),'duplicate VPS HRD'),
      ('bad-hrd-index',dict(hrds=((3,True,True,False,0),)),'HRD layer set index'),
      ('external-zero-hrd',dict(internal=False,hrds=((0,True,True,False,0),)),'HRD layer set index'),
      ('external-single-layer',dict(internal=False,maximum=0,sets=()),'requires multiple layers'),
      ('truncated',dict(hrds=((0,True,True,True,0),),fault='truncate'),'truncated'),
    ]
    for label,args,error in specs:
        name=f'hevc-vps-layer-sets-{label}.vps';(DEST/name).write_bytes(vps(**args))
        cases.append(dict(file=name,error=error,maximum=args.get('maximum',2),max_id=args.get('max_id',7),internal=args.get('internal',True),available=args.get('available',True),sets=[1,*args.get('sets',(9,128))]))
    (DEST/'hevc-vps-layer-sets.json').write_text(json.dumps(dict(cases=cases,provenance='Original VPS Annex A base-syntax metadata only. No opaque Annex F extension, coded pictures or multilayer playback acceptance is claimed. Normative layer sets and HRD inheritance authored offline, no private input, external codec or network.'),indent=2)+'\n')
if __name__=='__main__':main()
