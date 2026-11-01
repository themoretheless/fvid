#!/usr/bin/env python3
"""Offline original AAC-LD core/PNS diagnostic; never run by ordinary tests.

Whole-stream metrics are always retained. A separate diagnostic partition marks
PNS-dependent samples using syntax,never an error/delay/gain search. No waveform
conformance verdict is assigned;PNS energy/correlation need their own tests.
"""
import argparse,array,collections,hashlib,json,math,re,subprocess,sys
from pathlib import Path
from compare_aac_reference_pcm import integer_wave
ROOT=Path(__file__).resolve().parents[1]
PIN='865521883bbc19db177c71c00fcb57bd3619a8ae73a05a21532c3fc22cc13e92'
PAIRS=[('er_ad1020np_48_ep0','er_ad1020np_48'),('er_ad1120np_24_ep0','er_ad1120np_24')]
def sha(path):return hashlib.sha256(path.read_bytes()).hexdigest()
def stats(values):
    count=0;energy=0.;maximum=0.
    for value in values:
        if not math.isfinite(value):raise ValueError('nonfinite PCM difference')
        count+=1;energy+=value*value;maximum=max(maximum,abs(value))
    return dict(scalars=count,rmse=math.sqrt(energy/count),max_abs_error=maximum)if count else None

def main():
    p=argparse.ArgumentParser(description=__doc__)
    for name in['vectors-dir','inventory','native-probe','inspect-probe','output-dir','report']:p.add_argument('--'+name,type=Path,required=True)
    p.add_argument('--mps-input',type=Path);a=p.parse_args()
    out=a.output_dir.resolve()
    if out.is_relative_to(a.vectors_dir.resolve())or out.is_relative_to(ROOT/'tests/fixtures'):raise ValueError('research output must be separate from originals/fixtures')
    if a.report.resolve().is_relative_to(a.vectors_dir.resolve()):raise ValueError('report cannot overwrite originals')
    if sha(a.inventory)!=PIN:raise ValueError('ISO inventory pin mismatch')
    sums={name.lstrip('./'):digest for digest,name in(map(str.split,a.inventory.read_text().splitlines()))}
    inputs=[]
    def verified(path,folder):
        if hashlib.md5(path.read_bytes()).hexdigest()!=sums[folder+'/'+path.name]:raise ValueError('original input pin mismatch: '+path.name)
        row=dict(name=path.name,sha256=sha(path),iso_md5=sums[folder+'/'+path.name],bytes=path.stat().st_size);inputs.append((path,row));return row
    for name,wave in PAIRS:
        verified(a.vectors_dir/(name+'.mp4'),'compressedMp4');verified(a.vectors_dir/(wave+'.wav'),'referencesWav')
    if a.mps_input:verified(a.mps_input,'compressedMp4')
    out.mkdir(parents=True,exist_ok=True)
    def inspect(movie):
        records=out/(movie.stem+'.records');metadata=out/(movie.stem+'.syntax.jsonl')
        subprocess.run([str(a.native_probe.resolve()),'packets',str(movie),str(records)],capture_output=True,check=True)
        with metadata.open('w')as f:subprocess.run([str(a.inspect_probe.resolve()),str(records)],stdout=f,check=True)
        rows=[json.loads(s)for s in metadata.read_text().splitlines()]
        if not rows or any(c['frame']!=i for i,c in enumerate(rows)):raise ValueError('incomplete syntax diagnostic')
        return rows,dict(aus=len(rows),frame_samples=rows[0]['frame_samples'],pns_aus=sum(any(13 in g for g in c['books'])for c in rows),ltp_aus=sum(c['has_ltp']for c in rows),window_shapes=dict(collections.Counter(c['window_shape']for c in rows)),metadata_sha256=sha(metadata),raw_asc_au_records_sha256=sha(records))
    cases=[]
    for name,wave in PAIRS:
        movie=a.vectors_dir/(name+'.mp4');reference=a.vectors_dir/(wave+'.wav');pcm=out/(name+'.f32le')
        native=subprocess.run([str(a.native_probe.resolve()),'real',str(movie),str(pcm)],capture_output=True,text=True,check=True)
        c=subprocess.run([sys.executable,str(ROOT/'scripts/compare_aac_reference_pcm.py'),str(reference),str(pcm)],capture_output=True,text=True,check=True);whole=json.loads(c.stdout)
        rows,syntax=inspect(movie);n=syntax['frame_samples']
        if syntax['ltp_aus']or any(c['spatial']or c['frame_samples']!=n for c in rows):raise ValueError('one-AU PNS dependency partition requires ordinary fixed-size LD without LTP')
        channels,rate,depth,ref=integer_wave(reference.read_bytes());actual=array.array('f',pcm.read_bytes())
        if sys.byteorder!='little':actual.byteswap()
        if channels!=1 or len(actual)!=len(ref)or len(actual)!=len(rows)*n:raise ValueError('LD diagnostic geometry mismatch')
        match=re.search(r'rate=(\d+) channels=(\d+) scalars=(\d+)',native.stdout)
        if match is None or tuple(map(int,match.groups()))!=(rate,1,len(actual)):raise ValueError('native/reference output geometry mismatch')
        pns=[any(13 in g for g in c['books'])for c in rows];affected=[v or(i>0 and pns[i-1])for i,v in enumerate(pns)]
        def differences(selected):
            for i,flag in enumerate(affected):
                if flag==selected:
                    for k in range(i*n,(i+1)*n):yield actual[k]-ref[k]/2**(depth-1)
        row=dict(name=name,whole_stream=whole,syntax=syntax,deterministic_diagnostic=stats(differences(False)),pns_affected_diagnostic=stats(differences(True)),partition='Current or previous AU has PNS. Fixed2N LD overlap,no LTP/spatial filtering. Syntax-derived diagnostic only;whole stream retained.')
        cases.append(row);print(name,'whole RMS',whole['per_channel'][0]['rmse'],'deterministic',row['deterministic_diagnostic'],flush=True)
    mps=None
    if a.mps_input:
        _,mps=inspect(a.mps_input)
    for path,row in inputs:
        if sha(path)!=row['sha256']:raise ValueError('original changed during audit')
    report=dict(complete=True,conformance_verdict=None,delay_search=False,gain_fit=False,explicit_lag=0,inventory_sha256=PIN,native_probe_sha256=sha(a.native_probe),inspect_probe_sha256=sha(a.inspect_probe),validator_sha256=sha(Path(__file__)),inputs=[r for _,r in inputs],cases=cases,spatial_input_core_diagnostic=mps,scope='Original mono AAC-LD480/512 controls and optional LD5151 core syntax. PNS random waveform identity is not a normative requirement. Diagnostic partitions do not replace full waveform results or establish whole-profile/PNS statistical conformance.')
    a.report.parent.mkdir(parents=True,exist_ok=True);a.report.write_text(json.dumps(report,indent=2)+'\n')
if __name__=='__main__':main()
