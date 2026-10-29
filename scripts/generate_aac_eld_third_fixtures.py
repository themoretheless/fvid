"""Own factor-three ELD core movies, dense window/direct cosine oracle.
No LD-SBR factor-three acceptance, private media, process or network.
"""
import json
from generate_aac_eld_short_fixtures import generate as short,SOURCE
from generate_aac_eld_downscale_info_fixtures import prefix,extension
from generate_he_aac_packet_fixtures import packed,video_fixture
OUT=SOURCE/'aac-eld-third'
def generate():
    files=short((3,),(160,));m=json.loads(files['manifest.json']);bad=[]
    for corpus,names in [('aac-eld',['n512-c1','n512-c2']),('aac-eld-sbr',['n480-c1-x1-crc1','n480-c2-x2-crc1'])]:
        parent=SOURCE/corpus;manifest=json.loads((parent/'manifest.json').read_bytes());blob=(parent/'packets.bin').read_bytes()
        for stem in names:
            base=next(c for c in manifest['cases']if c['name']==stem);head,tail=prefix(base['asc']);asc=packed(head+extension(8000,True)+tail)
            rate=16000 if '-x2-'in stem else 8000;name='refuse-'+stem
            case=dict(base,name=name,asc=asc.hex(),container_rate=rate,container_frame_samples=base.get('container_frame_samples',base['n'])//3,video=name+'.mp4',requested_rate=rate,error='AAC output clock disagrees with configuration',reason='noninteger source frame'if corpus=='aac-eld'else 'LD-SBR has no integer CLDFB width at factor three')
            files[case['video']]=video_fixture([case],blob,case['channels'],filename=case['video'],write=False)['data'];bad.append(case)
    m['refused_modes']=bad;files['manifest.json']=(json.dumps(m,indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
