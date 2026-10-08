#!/usr/bin/env python3
"""Explicit offline 80-digit Decimal SBR dequantization protocol oracle."""
from decimal import Decimal as D, localcontext
from pathlib import Path
import json
cases=[]
with localcontext() as c:
 c.prec=80
 levels=[-8,-1,0,1,6,31,63,127]
 for coarse in [False,True]:
  a=D(1 if coarse else 2);pan=D(12 if coarse else 24)
  for level in levels:
   value=D(64)*D(2)**(D(level)/a)
   cases.append(dict(kind='envelope',level=level,coarse=coarse,value=str(value)))
   for balance in [0,2,12,24,48]:
    # Independent direct normative fractions, not the production split helper.
    numerator=D(64)*D(2)**(D(level)/a+1)
    left=numerator/(1+D(2)**((pan-D(balance))/a))
    right=numerator/(1+D(2)**((D(balance)-pan)/a))
    cases.append(dict(kind='coupled_envelope',level=level,balance=balance,coarse=coarse,left=str(left),right=str(right)))
 for level in range(31):
  value=D(2)**(6-level)
  cases.append(dict(kind='noise',level=level,value=str(value)))
  for balance in range(0,25,2):
   numerator=D(2)**(7-level)
   left=numerator/(1+D(2)**(balance-12));right=numerator/(1+D(2)**(12-balance))
   cases.append(dict(kind='coupled_noise',level=level,balance=balance,left=str(left),right=str(right)))
root=Path(__file__).resolve().parents[1]
(root/'tests/fixtures/playback-errors/aac-sbr-dequant-oracles.json').write_text(json.dumps(cases,separators=(',',':'))+'\n')
print('Independent Decimal cases:',len(cases))
