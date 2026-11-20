#!/usr/bin/env python3
"""Owned ordinary-frame syntax and physical-region controls, offline only."""
import json,pathlib
from generate_aac_ld_mps_config_fixtures import TREES,BANDS,length
from generate_aac_ld_mps_entropy_fixtures import field,packed,pcm
ROOT=pathlib.Path(__file__).resolve().parents[1]
OUT=ROOT/'tests/fixtures/playback-errors/aac-ld-mps-assemble'
def main():
 configs=bytearray();packets=bytearray();cases=[];errors=[];error_packets=bytearray()
 for tree,(_,_,lfe,ttt)in enumerate(TREES):
  for resolution in range(1,8):
   bands=BANDS[resolution]
   for variant in range(6):
    slots=[8,15,16,31,32,8][variant];one=bool(variant%2)if tree!=7 else False
    lo,hi,split=[(0,1,bands//2),(2,3,bands//2),(4,5,bands//2),(0,4,0),(1,2,bands),(3,0,bands//2)][variant]
    ott=[([0,bands//2,bands,bands//2,0,bands][variant]if x else bands)for x in lfe]
    cutoffs=[0 if x else bands//2 if variant%3==2 else 0 for x in lfe]+([bands//3 if variant>=3 else 0]if ttt else[])
    rate=[6,3,0,3,6,3][variant];inversion=bool(variant%2)
    wire=field(rate,4)+field(slots-1,5)+field(resolution,3)+field(tree,4)+'00'
    if tree!=7:wire+=field(one,1)
    wire+='0'
    if tree!=7:wire+='000000'
    wire+='000'
    if tree!=7:wire+=field(inversion,1)
    wire+='0000'
    if tree!=7:wire+='0'
    for b,is_lfe in enumerate(lfe):
     if is_lfe:wire+=field(ott[b],5)
    if ttt:wire+='1'+field(lo,3)+field(hi,3)+field(split,5)
    wire+='0'*(-len(wire)%8)
    residual=any(cutoffs)or variant==4
    if residual:
     data=field(rate,4)+'00'+''.join('1'+field(v,5)for v in cutoffs);payload=packed(data);wire+='0000'+length(len(payload))+''.join(field(v,8)for v in payload)
    cfg=packed(wire);co=len(configs);configs.extend(cfg)
    regions=[]
    def region(id,kind,start,n):regions.append(dict(id=id,kind=kind,start=start,bands=n))
    for b,n in enumerate(ott):region(f'ott-cld:{b}','CLD',0,n)
    if one:region('ott-icc:all','ICC',0,bands)
    else:
     for b,is_lfe in enumerate(lfe):
      if not is_lfe:region(f'ott-icc:{b}','ICC',0,bands)
    if ttt:
     for high,start,stop,mode in [(False,0,split,lo),(True,split,bands,hi)]:
      if high and start==stop:continue
      for component in range(2):region(f'ttt-{"cpc"if mode<2 else"cld"}:0:{int(high)}:{component}','CPC'if mode<2 else'CLD',start,stop-start)
      if mode<2:region(f'ttt-icc:0:{int(high)}','ICC',start,stop-start)
    frames=[];previous={}
    for fi in range(3):
     variable=fi==1;positions=[0,slots-2]if variable else[(slots+1)//2-1,slots-1];bits=field(variable,1)+'1'+(''.join(field(v,(slots-1).bit_length())for v in positions)if variable else'')+field(fi==0,1);parameters=[];icc_positions=[]
     for ri,r in enumerate(regions):
      n=r['bands'];kind=r['kind'];mode=1 if fi==2 else 3;bits+=field(mode,2)*2
      if mode==3:
       levels,offset={'CLD':(31,15),'ICC':(8,0),'CPC':(51,20)}[kind]
       rows=[[(j*7+ri*3+ps*5+fi*11+variant)%levels-offset for j in range(n)]for ps in range(2)]
       if r['id'].startswith('ttt-icc'):rows=[[(j+ps+fi+variant)%5 for j in range(n)]for ps in range(2)]
       if r['id'].startswith('ttt-icc')and n:icc_positions.append(len(bits)+5)
       bits+='10001'+pcm([rows[ps][j]+offset for j in range(n)for ps in range(2)],levels)
       previous[r['id']]=rows[-1][:]
      else:rows=[previous[r['id']][:]for _ in range(2)]
      parameters.append(dict(id=r['id'],kind=kind,start=r['start'],values=rows))
     smoothing=[]
     for ps in range(2):
      mode=[0,2,3][(fi+ps)%3];time=(fi+ps)%4;mask=[(b+fi+ps)%2==0 for b in range(bands)]
      bits+=field(mode,2)+(field(time,2)if mode>=2 else'')+('00'+''.join(field(v,1)for v in mask)if mode==3 else'')
      smoothing.append(dict(time=256 if mode==0 else[64,128,256,512][time],mask=[False]*bands if mode==0 else[True]*bands if mode==2 else mask))
     bits+='0'*(-len(bits)%8)
     if residual:bits+='00000000'
     if tree==2 and resolution==7 and variant==0 and fi==0:
      at=icc_positions[0];bad=packed(bits[:at]+field(5,3)+bits[at+3:]);errors.append(dict(case=len(cases),offset=len(error_packets),bytes=len(bad),reason='singular LD-MPS TTT prediction coherence'));error_packets.extend(bad)
     packet=packed(bits);frames.append(dict(offset=len(packets),bytes=len(packet),bits=len(bits),slots=positions,parameters=parameters,smoothing=smoothing));packets.extend(packet)
    cases.append(dict(name=f'tree{tree}-res{resolution}-v{variant}',tree=tree,bands=bands,variant=variant,slots=slots,one_icc=one,ott_bands=ott,ttt=[lo,hi,split]if ttt else None,inversion=inversion,cutoffs=cutoffs,config_offset=co,config_bytes=len(cfg),frames=frames))
 OUT.mkdir(parents=True,exist_ok=True);(OUT/'errors.bin').write_bytes(error_packets);(OUT/'configs.bin').write_bytes(configs);(OUT/'packets.bin').write_bytes(packets);(OUT/'manifest.json').write_text(json.dumps(dict(scope='Frame-to-matrix assembly only; not spatial PCM/movie playback acceptance.',cases=cases,errors=errors),separators=(',',':'))+'\n');print(len(cases),len(configs),len(packets))
if __name__=='__main__':main()
