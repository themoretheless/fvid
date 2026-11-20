"""Own factor-three high-band/rational-clock controls, explicit and offline."""
import json
from generate_aac_eld_fixtures import generate as core,mkv
from generate_aac_eld_short_fixtures import short_window,SOURCE
from generate_aac_eld_downscale_info_fixtures import prefix,extension
from generate_he_aac_packet_fixtures import packed,video_fixture
from generate_mp4_ep1_grouped_ps_fixtures import replace_asc
OUT=SOURCE/'aac-eld-third-rates'
RATES=(96000,88200,48000,44100,24000,22050,12000,11025,7350,37566,12345)
def generate():
    files={};cases=[];exports=[];blob=bytearray()
    for rate in RATES:
     for cfg in [1,2]:
        spec=(480,rate,cfg,'high',0,0,True);full=core(True,[spec]);data=core(True,[spec],output_samples=160,synthesis_window=short_window)
        base=json.loads(data['manifest.json'])['cases'][0];original=json.loads(full['manifest.json'])['cases'][0]
        assert [r['bits']for r in base['frames']]==[r['bits']for r in original['frames']]
        head,tail=prefix(base['asc']);asc=packed(head+extension(rate//3,True)+tail);name=base['name']+'-ds3';offset=len(blob);blob.extend(data['packets.bin'])
        for row in base['frames']:row['offset']+=offset
        case=dict(base,name=name,asc=asc.hex(),factor=3,source_rate=rate,out_n=160,video=name+'.mp4',matroska=name+'.mka',golden=name+'.f32le')
        files[case['golden']]=data[base['name']+'.f32le'];files[case['video']]=video_fixture([case],blob,cfg,filename=case['video'],write=False)['data'];packets=[blob[r['offset']:r['offset']+r['bytes']]for r in case['frames']];files[case['matroska']]=mkv(asc,packets,rate//3,160,cfg);cases.append(case)
        ep=packed(head+extension(rate//3,True)+tail[:4]+'01'+tail[6:]);name='ep1-'+name;export=dict(case,name=name,asc=ep.hex(),video=name+'.mp4',control=case['video']);files[export['video']]=replace_asc(files[case['video']],ep);exports.append(export)
    files['packets.bin']=bytes(blob);files['manifest.json']=(json.dumps(dict(cases=cases,exports=exports,provenance='Own full-480-spectrum first/final-SFB controls. Full and short source bitstrings asserted identical; direct cosine plus independently dense-solved short windows. Standard and explicit clocks test fractional nanoseconds. No private media, foreign decoder, process or network.'),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
