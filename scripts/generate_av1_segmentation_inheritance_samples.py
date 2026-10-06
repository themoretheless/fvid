#!/usr/bin/env python3
"""Owned AV1 inherited zero-map segmentation streams and explicit gap reproducers."""
import argparse, hashlib, json, subprocess
from pathlib import Path
from generate_av1_show_existing_samples import Bits, obu, sequence, webm

def key():
    b=Bits();b.u(0);b.u(0,2);b.u(0);b.u(1);b.u(1);b.u(1);b.u(0)
    b.u(255,8);b.u(0);b.u(1);b.u(0,8);b.u(0,6)
    return obu(6,b.bytes()+bytes([0x2e,0x03,0xff,0xff,0xfd]))

def inter(slot,primary,enabled,update,features,update_map=False,temporal=False,truncated=False):
    b=Bits();b.u(0);b.u(1,2);b.u(1);b.u(0);b.u(1);b.u(0);b.u(primary,3);b.u(1<<slot,8)
    for _ in range(7):b.u(slot,3)
    b.u(0);b.u(1);b.u(0);b.u(0,2);b.u(0);b.u(1);b.u(0,8);b.u(0,4)
    b.u(int(enabled))
    if enabled:
        b.u(int(update_map))
        if update_map:b.u(int(temporal))
        b.u(int(update))
        if truncated:return obu(6,b.bytes())
        if update:
            for segment in range(8):
                for feature in range(8):
                    value=features.get((segment,feature));b.u(int(value is not None))
                    if value is not None:b.u(value,[9,7,7,7,7,3,0,0][feature])
    # Any positive unused ALT_Q makes the frame-wide coded_lossless flag false.
    if enabled and any(feature==0 and value>0 for (_,feature),value in features.items()):
        b.u(0,6);b.u(0,6);b.u(0,3);b.u(0) # zero loop filter levels
        b.u(0) # largest transform mode (active segment zero remains lossless)
    b.u(0);b.u(0);b.u(0,7)
    return obu(6,b.bytes()+bytes([0x8c,0x70]))

def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--oracle',type=Path);args=parser.parse_args()
    root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors';records=[];refusals=[];map_acceptances=[]
    profiles={
        'empty':{},
        'unused-negative':{(1,0):-256,(2,1):-64,(3,2):63,(4,3):0,(5,4):17,(6,5):7,(7,6):0,(7,7):0},
        'unused-positive':{(1,0):255,(2,1):63,(3,2):-63,(4,3):0,(5,4):-17,(6,5):0,(7,6):0,(7,7):0},
        'active-zero':{(0,feature):0 for feature in range(5)},
    }
    for name,features in profiles.items():
        for slot in range(8):
            for primary in range(7):
                prefix=sequence(False,False,False)+key();data=prefix
                # publish data, inherit it, disable/reset, inherit reset data,
                # restore data, and explicitly clear all previously active features.
                for enabled,update,table in [(True,True,features),(True,False,features),(False,False,{}),(True,False,{}),(True,True,features),(True,True,{})]:
                    data+=inter(slot,primary,enabled,update,table)
                filename=f'av1-seg-inherit-{name}-slot{slot}-primary{primary}.obu';(root/filename).write_bytes(data)
                records.append(dict(file=filename,sha256=hashlib.sha256(data).hexdigest(),shown=6,profile=name,slot=slot,primary=primary))
    for temporal in [False,True]:
        data=sequence(False,False,False)+key()+inter(0,0,True,False,{},True,temporal)
        filename=f'av1-seg-inherit-map-update-temporal{int(temporal)}.obu';(root/filename).write_bytes(data)
        map_acceptances.append(dict(file=filename,sha256=hashlib.sha256(data).hexdigest(),acceptance=True,reference_shown=1))
    data=sequence(False,False,False)+key()+inter(0,0,True,True,{(0,0):1})
    filename='av1-seg-inherit-active-alt-q-gap.obu';(root/filename).write_bytes(data)
    active_acceptances=[dict(file=filename,sha256=hashlib.sha256(data).hexdigest(),acceptance=True,reference_shown=1)]
    data=sequence(False,False,False)+key()+inter(0,0,True,True,{},truncated=True)
    filename='av1-seg-inherit-invalid-truncated-data.obu';(root/filename).write_bytes(data)
    refusals.append(dict(file=filename,sha256=hashlib.sha256(data).hexdigest(),error='truncated or oversized bit field',acceptance=False))
    for r in records:
        if r['slot'] in [0,7] and r['primary'] in [0,6]:
            filename=r['file'].replace('.obu','.webm');data=webm((root/r['file']).read_bytes());(root/filename).write_bytes(data)
            r['webm']=filename;r['webm_sha256']=hashlib.sha256(data).hexdigest()
    if args.oracle:
        for r in records:subprocess.run([str(args.oracle),str(root/r['file']),str(r['shown'])],check=True)
        for r in refusals+active_acceptances+map_acceptances:
            if 'reference_shown' in r:subprocess.run([str(args.oracle),str(root/r['file']),str(r['reference_shown'])],check=True)
    (root/'av1-seg-inherit-generated.json').write_text(json.dumps(dict(fixtures=records,refusals=refusals,active_acceptances=active_acceptances,map_acceptances=map_acceptances),indent=2)+'\n')
if __name__=='__main__':main()
