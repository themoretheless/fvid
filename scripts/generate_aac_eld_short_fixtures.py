"""Own short ELD reconstruction: dense spline solve and direct cosine oracle.

No codec, process, network, SciPy or production downscale implementation is used.
The normative source numbers are shared with the existing independent ELD oracle.
"""
import json,struct
from functools import lru_cache
from pathlib import Path
from generate_aac_eld_fixtures import generate as core,specifications,window,mkv
from generate_aac_eld_downscale_info_fixtures import prefix,extension
from generate_he_aac_packet_fixtures import packed,video_fixture
ROOT=Path(__file__).resolve().parents[1];SOURCE=ROOT/'tests/fixtures/playback-errors';OUT=SOURCE/'aac-eld-short'
def dense_spline(y):
    n=len(y);a=[[0.]*(n+1)for _ in range(n)]
    a[0][0:3]=[1.,-2.,1.];a[-1][-4:-1]=[1.,-2.,1.]
    for i in range(1,n-1):a[i][i-1:i+2]=[1.,4.,1.];a[i][-1]=3*(y[i+1]-2*y[i]+y[i-1])
    for k in range(n):
        pivot=max(range(k,n),key=lambda i:abs(a[i][k]));a[k],a[pivot]=a[pivot],a[k]
        scale=a[k][k];assert abs(scale)>1e-12
        a[k][k:]=[v/scale for v in a[k][k:]]
        for i in range(k+1,n):
            v=a[i][k]
            if v:a[i][k:]=[x-v*z for x,z in zip(a[i][k:],a[k][k:])]
    c=[0.]*n
    for i in range(n-1,-1,-1):c[i]=a[i][-1]-sum(v*x for v,x in zip(a[i][i+1:n],c[i+1:]))
    return c
@lru_cache(None)
def short_window(n):
    result=[];reference=window(512);count=n//4
    for start in range(0,len(reference),128):
        y=reference[start:start+128];c=dense_spline(y)
        for i in range(count):
            x=(i+.5)*128/count-.5;j=int(x);t=x-j
            # Independent interval Hermite form from knot derivatives.
            left=y[j+1]-y[j]-(2*c[j]+c[j+1])/3
            right=y[j+1]-y[j]+(c[j]+2*c[j+1])/3
            result.append((2*t**3-3*t*t+1)*y[j]+(t**3-2*t*t+t)*left+(-2*t**3+3*t*t)*y[j+1]+(t**3-t*t)*right)
    assert len(result)==4*n
    return result
def generate(factors=(2,4), windows=(120,128,240,256)):
    files={};cases=[];exports=[];blob=bytearray();parent=json.loads((SOURCE/'aac-eld'/'manifest.json').read_bytes());parent_blob=(SOURCE/'aac-eld'/'packets.bin').read_bytes()
    for spec in specifications():
      n,rate=spec[:2]
      for factor in factors:
        if rate%factor or n%factor:continue
        sn=n//factor;data=core(specs=[spec],output_samples=sn,synthesis_window=short_window);base=json.loads(data['manifest.json'])['cases'][0]
        original=next(c for c in parent['cases']if c['name']==base['name']);raw=data['packets.bin']
        assert [raw[r['offset']:r['offset']+r['bytes']]for r in base['frames']]==[parent_blob[r['offset']:r['offset']+r['bytes']]for r in original['frames']]
        head,tail=prefix(base['asc']);asc=packed(head+extension(rate//factor,True)+tail);name=base['name']+f'-ds{factor}';offset=len(blob);blob.extend(raw)
        for row in base['frames']:row['offset']+=offset
        case=dict(base,name=name,asc=asc.hex(),factor=factor,source_rate=rate,out_n=sn,video=name+'.mp4',matroska=name+'.mka',golden=name+'.f32le',control=f'../aac-eld/{original["video"]}')
        files[case['golden']]=data[base['name']+'.f32le'];files[case['video']]=video_fixture([case],blob,case['channels'],filename=case['video'],write=False)['data']
        packets=[blob[r['offset']:r['offset']+r['bytes']]for r in case['frames']];files[case['matroska']]=mkv(asc,packets,case['rate'],sn,case['channels']);cases.append(case)
    # The same source spectra also pass through real EP3 FEC and EP1 grouped
    # transport. Their independent short PCM is the plain programme's oracle.
    from generate_mp4_ep1_grouped_ps_fixtures import replace_asc
    ep_parent=json.loads((SOURCE/'aac-eld-esc'/'manifest.json').read_bytes())
    ep_blob=(SOURCE/'aac-eld-esc'/'packets.bin').read_bytes()
    originals=list(cases)
    for base in originals:
        original=next(c for c in parent['cases']if c['video']==base['control'].split('/')[-1])
        protected=next(c for c in ep_parent['cases']if c['name']==original['name'])
        head,tail=prefix(protected['asc']);asc=packed(head+extension(base['rate'],True)+tail)
        rows=[]
        for row in protected['frames']:
            payload=ep_blob[row['offset']:row['offset']+row['bytes']]
            rows.append(dict(offset=len(blob),bytes=len(payload)));blob.extend(payload)
        name='ep3-'+base['name'];case=dict(base,name=name,asc=asc.hex(),frames=rows,video=name+'.mp4',matroska=name+'.mka',protected=True)
        files[case['video']]=video_fixture([case],blob,case['channels'],filename=case['video'],write=False)['data']
        payloads=[blob[r['offset']:r['offset']+r['bytes']]for r in rows];files[case['matroska']]=mkv(asc,payloads,case['rate'],case['out_n'],case['channels']);cases.append(case)
        head,tail=prefix(original['asc']);asc=packed(head+extension(base['rate'],True)+tail[:4]+'01'+tail[6:])
        name='ep1-'+base['name'];case=dict(base,name=name,asc=asc.hex(),video=name+'.mp4',control=base['video'])
        files[case['video']]=replace_asc(files[base['video']],asc);exports.append(case)
    for n in windows:files[f'window-{n}.f64le']=struct.pack('<'+'d'*(4*n),*short_window(n))
    files['packets.bin']=bytes(blob);files['manifest.json']=(json.dumps(dict(cases=cases,exports=exports,provenance='Own full-spectrum ELD entropy packets, independently solved dense not-a-knot window segments, Hermite evaluation and direct short cosine/overlap oracle. Nominal source transform normalization. Existing source AU bytes asserted unchanged. No private media, foreign codec, process or network.'),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
