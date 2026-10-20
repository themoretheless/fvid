"""Untimed authored AVC companions; no encoder/network/private media."""
import json,hashlib
from pathlib import Path
from generate_avc_mbaff_inter_component_regressions import generate as source,matroska
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-matroska-pipeline'
def generate():
    files,cases=source();result={};companions=[]
    for name in ['y8-c14-frame-cavlc-b-bi','y14-c8-field-cabac-b-bi']:
        case=next(c for c in cases if c['file']==name+'.mp4')
        display=[(files[f['packet']],None) for f in case['frames']]
        data=matroska(files[case['configuration']],display,case['decode_order'],case['mode'],timed=False)
        result[name+'-untimed.mkv']=data
        companions.append({'file':name+'-untimed.mkv','control':case['matroska']})
    result['manifest.json']=(json.dumps({'provenance':'Authored synthetic AVC companions with missing default/block durations; duration inferred from visible presentation order and Segment end.','cases':companions,'sha256':{k:hashlib.sha256(v).hexdigest() for k,v in result.items()}},indent=2)+'\n').encode()
    return result
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
