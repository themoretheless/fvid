"""Authored base-layer routing/seek witnesses; no encoder/network/private media."""
from pathlib import Path
import json,hashlib
import generate_matroska_parameter_seek as base
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'tests/fixtures/playback-errors/hevc-base-dispatch-seek'
def generate():
    originals=base.generate()
    control=next(c for c in json.loads(originals['controls.json'])if c['codec']=='hevc'and c['mode']=='skip-pictures')
    sources=base.hevc.generate();c=next(c for c in json.loads(sources['manifest.json'])if c['name']=='32-33-34-prefix')
    private,packets,size,keys=base.inventory(sources[c['name']+'.mp4'])
    files={f['oracle']:originals[f['oracle']]for f in control['frames']};cases=[]
    # IDR first_slice=1, no_output_of_prior_pics=0, ue(63), rbsp stop bit.
    pps_bits='10'+'0000001000000'+'1'
    absent_pps=b'\x26\x01'+int(pps_bits,2).to_bytes(2,'big')
    for mode,nal in [('healthy',None),('header-only',b'\x26\x01'),('bad-escape',b'\x26\x01\x00\x00\x03\x04'),('absent-pps',absent_pps)]:
        changed=list(packets)
        if nal is not None:
            for i in range(control['new_start']):changed[i]=len(nal).to_bytes(4,'big')+nal
        for companion in [False,True]:
            name=f'{mode}-'+('companion'if companion else 'video')+'.mkv'
            files[name]=base.movie('V_MPEGH/ISO/HEVC',private,changed,size,keys,companion)
            cases.append(dict(control,file=name,mode=mode,companion=companion,first_nal_hex=nal.hex()if nal else None))
    files['bad-parameters.mkv']=originals['hevc-refuse-parameters.mkv']
    cases.append(dict(control,file='bad-parameters.mkv',mode='bad-parameters',companion=True))
    bad=list(packets);nal=b'\xa6\x01';bad[0]=len(nal).to_bytes(4,'big')+nal
    files['bad-header.mkv']=base.movie('V_MPEGH/ISO/HEVC',private,bad,size,keys,False)
    cases.append(dict(control,file='bad-header.mkv',mode='bad-header',companion=False))
    files['manifest.json']=(json.dumps(dict(provenance='Original tiny owned HEVC old/new CVS and independent full-depth pixel arrays from matroska-parameter-seek. Valid NAL headers with empty RBSP, invalid escape or absent PPS simulate damaged old pictures; received parameters and new CVS remain intact. Separate corrupt parameter/header refusal controls. No private media,encoder,FFmpeg or network.',cases=cases,sha256={n:hashlib.sha256(b).hexdigest()for n,b in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
