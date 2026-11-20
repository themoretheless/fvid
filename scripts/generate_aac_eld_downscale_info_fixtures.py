"""Own ELD extension-3 sample-rate adaptation metadata, offline.

The default full-rate reconstruction is retained; this is not an optimized
short-transform/downscaled-output implementation. Independent parent PCM.
"""
import json
from pathlib import Path
from generate_he_aac_packet_fixtures import field,frequency,packed,video_fixture
from generate_aac_eld_fixtures import mkv
ROOT=Path(__file__).resolve().parents[1];SOURCE=ROOT/'tests/fixtures/playback-errors';OUT=SOURCE/'aac-eld-downscale-info'
def prefix(config):
    bits=''.join(f'{x:08b}'for x in bytes.fromhex(config));at=11
    index=int(bits[at:at+4],2);at+=4+(24 if index==15 else 0)
    cfg=int(bits[at:at+4],2);at+=4+4
    present=bits[at]=='1';at+=1
    if present:
        at+=2
        groups={1:1,2:1,3:2,4:3,5:3,6:3,7:4,11:4,12:4,14:4}[cfg]
        for _ in range(groups):
            extra1,extra2=bits[at+14:at+16];at+=16+5*int(extra1)+6*int(extra2)
    assert bits[at:at+4]=='0000'
    return bits[:at],bits[at:]
def extension(rate,explicit=False,fill=0,length=None):
    value=(field(15,4)+field(rate,24)if explicit else frequency(rate))+field(fill,4)
    assert len(value)%8==0
    return field(3,4)+field(len(value)//8 if length is None else length,4)+value
def generate():
    files={};cases=[];bad=[];exports=[]
    for corpus in ['aac-eld','aac-eld-sbr','aac-eld-esc-sbr']:
        parent=SOURCE/corpus;m=json.loads((parent/'manifest.json').read_bytes());blob=(parent/'packets.bin').read_bytes()
        selected=[c for c in m['cases']if corpus=='aac-eld'or c['name']in[f'n{n}-c{cfg}-x{x}-crc{crc}'for n in [480,512]for cfg in [1,2,7]for x in [1,2]for crc in [0,1]]]
        for base in selected:
            head,suffix=prefix(base['asc']);packets=[blob[r['offset']:r['offset']+r['bytes']]for r in base['frames']]
            # All saved core controls are 24 kHz or 48 kHz, except explicit high
            # rate controls. Identity and an explicitly encoded hint cover both
            # wire representations without altering source transform geometry.
            core=24000 if corpus!='aac-eld'else base['rate']
            rates=[('identity',core,False),('explicit',core,True)]
            if core%2==0:rates.append(('half',core//2,True))
            for label,rate,explicit in rates:
                name=corpus+'-'+base['name']+'-'+label;asc=packed(head+extension(rate,explicit)+suffix);case=dict(base,name=name,asc=asc.hex(),hint=rate,core_rate=core,plain_asc=base['asc'],control=f'../{corpus}/{base["video"]}',golden=f'../{corpus}/{base["name"]}.f32le',video=name+'.mp4',matroska=name+'.mka',source=corpus)
                files[case['video']]=video_fixture([case],blob,case['channels'],filename=case['video'],write=False)['data']
                files[case['matroska']]=mkv(asc,packets,case['rate'],case['container_frame_samples'],case['channels']);cases.append(case)
            if base['name']in ['n512-c1','n480-c7-x2-crc1']:
                for label,bits,error in [('zero',extension(0,True),'zero AAC frequency'),('fill',extension(core,True,1),'nonzero AAC ELD downscale fill nibble'),('short',extension(core,True,length=1),'truncated or oversized bit field'),('reserved',field(3,4)+field(1,4)+'11010000','reserved AAC frequency index')]:
                    name=corpus+'-'+base['name']+'-'+label;asc=packed(head+bits+suffix);case=dict(base,name=name,asc=asc.hex(),video=name+'.mp4',error=error)
                    files[case['video']]=video_fixture([case],blob,case['channels'],filename=case['video'],write=False)['data'];bad.append(case)
    from generate_mp4_ep1_epochs_fixtures import eld_control
    from generate_mp4_ep1_grouped_ps_fixtures import replace_asc
    parent=json.loads((SOURCE/'aac-eld-esc-sbr'/'manifest.json').read_bytes())
    for n in [480,512]:
      for cfg in [1,7]:
       for dual in [False,True]:
        for crc in [False,True]:
            stem=f'n{n}-c{cfg}-x{2 if dual else 1}-crc{int(crc)}';base=next(c for c in parent['cases']if c['name']==stem)
            control,_,_,channels=eld_control(n,cfg,crc=crc,dual=dual,frame_count=12)
            head,_=prefix(base['plain_asc']);asc=packed(head+extension(12000,True)+'0000'+'01')
            name='ep1-'+stem;files[name+'-control.mp4']=control;files[name+'.mp4']=replace_asc(control,asc)
            exports.append(dict(name=name,video=name+'.mp4',control=name+'-control.mp4',golden='../aac-eld-esc-sbr/'+stem+'.f32le',asc=asc.hex(),rate=base['rate'],channels=channels,n=n,frames=12,hint=12000))
    files['manifest.json']=(json.dumps(dict(cases=cases,malformed=bad,exports=exports,provenance='Own ELD/LD-SBR ASC writer and own packet sources. Extension 3 is exposed as adaptation metadata while retaining the existing full-rate reconstruction and independent parent PCM. No optimized short transform, private media, foreign codec, subprocess or network.'),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
