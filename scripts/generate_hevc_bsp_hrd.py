#!/usr/bin/env python3
"""Original BSP HRD continuations of owned general-VUI metadata."""
import json
from pathlib import Path
D=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
def ue(n):
 s=f'{n+1:b}';return '0'*(len(s)-1)+s
def packed(s):return int(s+'0'*((-len(s))%8),2).to_bytes((len(s)+7)//8,'big')
c=next(c for c in json.loads((D/'hevc-layered-vui.json').read_text()) if c['file']=='hevc-layered-vui-bsp.bin')
prefix=''.join(f'{v:08b}' for v in (D/c['file']).read_bytes())[:c['end']]
cases=[]
for name in ('basic','inherited','maximum','no-cpb-lists','zero','bad-schemes','bad-partitions','duplicate','omitted','reversed','bad-cpb','bad-hrd','bad-temporal','bad-order'):
 count=0 if name=='zero' else 2 if name=='inherited' else 3 if name=='bad-hrd' else 1
 cpbs=32 if name=='maximum' else 2
 schemes=16 if name=='maximum' else 17 if name=='bad-schemes' else 1
 schedule_count=32 if name=='maximum' else 2
 error={'bad-schemes':'out of range','bad-partitions':'out of range','duplicate':'multiple partitions','omitted':'omits a layer','reversed':'follows its dependent','bad-cpb':'CPB schedule index','bad-hrd':'HRD index','bad-temporal':'out of range','bad-order':'bitrate or buffer ordering'}.get(name)
 bits=prefix+ue(count)
 for h in range(count):
  if h:bits+='0' # Inherit the preceding common parameters.
  bits+=ue(3 if name=='bad-temporal' else 2)
  if h==0:bits+=('00' if name=='no-cpb-lists' else '100'+'0'*8+'0'*15)
  for tid in range(3):
   bits+='1'+ue(0)+ue(cpbs-1)
   if name!='no-cpb-lists':
    for i in range(cpbs):bits+=ue(10+h*40+i*10-1)+ue(128-h*32-i-1)+'1'
 partitions=[9,32896]
 if name=='duplicate':partitions=[9,32905]
 if name=='omitted':partitions=[9,128]
 if name=='reversed':partitions=[32896,9]
 if count:
  bits+=ue(schemes)
  for _ in range(schemes):
   bits+=ue(4 if name=='bad-partitions' else 1)
   for mask in partitions:bits+=''.join(str(int(bool(mask&(1<<id)))) for id in (0,3,7,15))
  for scheme in range(schemes+1):
   parts=4 if scheme==0 else 2
   for tid in range(3):
    bits+=ue(schedule_count-1)
    for schedule in range(schedule_count):
     for _ in range(parts):
      index=3 if name=='bad-hrd' else 1 if name=='inherited' and schedule else 0
      cpb=2 if name=='bad-cpb' else 0 if name in ('inherited','bad-order') else schedule
      if count>1:bits+=f'{index:0{(count-1).bit_length()}b}'
      bits+=ue(cpb)
 file='hevc-bsp-'+name+'.bin';(D/file).write_bytes(packed(bits))
 cases.append(dict(file=file,error=error,end=len(bits),hrds=count,cpbs=cpbs,schemes=schemes+1,schedules=schedule_count,no_lists=name=='no-cpb-lists',inherited=name=='inherited'))
(D/'hevc-bsp.json').write_text(json.dumps(cases,indent=2)+'\n')
