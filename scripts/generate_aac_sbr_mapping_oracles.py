#!/usr/bin/env python3
"""Offline original SBR mapping cases from explicit protocol grids/attack table.
No decoder processes; exact original integer scalefactors and boolean flags.
"""
from pathlib import Path
import json
root=Path(__file__).resolve().parents[1]/"tests/fixtures/playback-errors"
high=[10,11,14,18,21,28];low=[10,11,18,28];noise=[10,18,28]
cases=[]
for kind in ["fixvar","varfix"]:
    attack_table=[None,3,2,1] if kind=="fixvar" else [None,None,1,2]
    # Noise row corresponding to each envelope, for each transmitted pointer.
    noise_rows=[[0,0,1],[0,0,1],[0,0,1],[0,1,1]] if kind=="fixvar" else [[0,1,1],[0,0,1],[0,1,1],[0,0,1]]
    for pointer,attack in enumerate(attack_table):
        for pattern in range(32):
            harmonics=[bool(pattern&(1<<i)) for i in range(5)]
            expected=[]
            for envelope in range(3):
                active={ (high[i]+high[i+1])//2 for i in range(5) if harmonics[i] and (attack is None or envelope>=attack) }
                frequency=high if envelope==1 else low
                row=[]
                for band in range(10,28):
                    index=next(i for i,(a,b) in enumerate(zip(frequency,frequency[1:])) if a<=band<b)
                    presence=bool(active.intersection(range(frequency[index],frequency[index+1])))
                    q_index=0 if band<18 else 1
                    q_value=1+q_index+2*noise_rows[pointer][envelope]
                    row.append([(envelope+1)*100+index,q_value,presence,band in active])
                expected.append(row)
            cases.append({"class":kind,"pointer":pointer,"harmonics":harmonics,"attack":attack,"suppress":[envelope==attack for envelope in range(3)],"expected":expected})
assert len(cases)==256
(root/"aac-sbr-mapping-oracles.json").write_text(json.dumps({"source":"GOST R53556.4-2013 6.18.7.2 table176; ISO Cor.1 pp9/10", "high":high,"low":low,"noise":noise,"cases":cases},separators=(",",":"))+"\n")
