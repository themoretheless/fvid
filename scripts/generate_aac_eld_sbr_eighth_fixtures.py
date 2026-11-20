"""Explicit own factor-eight LD-SBR movies. No process, network or decoder."""
import json,struct
from generate_aac_eld_sbr_downscaled_fixtures import generate as downscaled,SOURCE
from generate_aac_eld_short_fixtures import short_window
OUT=SOURCE/'aac-eld-sbr-eighth'
def generate():
    files=downscaled(factors=(8,),high_rates=(48000,96000))
    for n in [60,64]:files[f'window-{n}.f64le']=struct.pack('<'+'d'*(4*n),*short_window(n))
    from generate_aac_eld_cldfb_downscaled_fixtures import generate as primitive
    files.update({"cldfb/"+name:data for name,data in primitive((4,),(4,)).items()})
    from generate_aac_eld_sbr_downscaled_fixtures import core_data
    from generate_aac_eld_downscale_info_fixtures import prefix,extension
    from generate_he_aac_packet_fixtures import packed,video_fixture
    from generate_aac_eld_fixtures import mkv
    cases=[];blob=bytearray()
    for n,cfg in [(480,1),(512,2)]:
        data=core_data(n,cfg,8);base=json.loads(data['manifest.json'])['cases'][0]
        head,tail=prefix(base['asc']);asc=packed(head+extension(3000,True)+tail);name=f'core-n{n}-c{cfg}-ds8';frames=[];packets=[]
        for row in base['frames']:
            packet=data['packets.bin'][row['offset']:row['offset']+row['bytes']];frames.append(dict(offset=len(blob),bytes=len(packet)));blob.extend(packet);packets.append(packet)
        case=dict(base,name=name,asc=asc.hex(),frames=frames,video=name+'.mp4',matroska=name+'.mka',golden=name+'.f32le',out_n=n//8,source_rate=24000)
        files['core/'+case['golden']]=data[base['name']+'.f32le'];files['core/'+case['video']]=video_fixture([case],blob,case['channels'],filename=case['video'],write=False)['data'];files['core/'+case['matroska']]=mkv(asc,packets,3000,n//8,case['channels']);cases.append(case)
    files['core/packets.bin']=bytes(blob);files['core/manifest.json']=(json.dumps(dict(cases=cases,provenance='Own full-spectrum core packets, direct short cosine and independently dense-solved windows. No LD-SBR, private media, codec implementation, process or network.'),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():
        path=OUT/name;path.parent.mkdir(parents=True,exist_ok=True);path.write_bytes(data)
