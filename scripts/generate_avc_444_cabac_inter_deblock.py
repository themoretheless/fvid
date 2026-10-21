"""Owned full 444 CABAC component CBF/QP/deblocking with independent scalar pixel goldens."""
import json,tempfile,struct
from pathlib import Path
from generate_avc_444_inter import configuration,mux
from generate_avc_444_cabac_inter import pcm,p_packet,echo
import generate_avc_444_inter_deblock as oracle
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-444-cabac-inter-deblock'
def generate():
    OUT.mkdir(exist_ok=True);cases=[]
    with tempfile.TemporaryDirectory(prefix='fvid-444-cabac-filter-oracle-')as tmp:
        original=oracle.OUT;oracle.OUT=Path(tmp)
        try:oracle.generate()
        finally:oracle.OUT=original
        for init_idc in [0,1,2]:
            for c in json.loads((Path(tmp)/'manifest.json').read_text()):
                old=c['name'];name='cabac-'+old+f'-i{init_idc}';yd,cd=c['yd'],c['cd'];depths=[yd,cd,cd]
                words=struct.unpack('<4608H',(Path(tmp)/f'{old}.y16').read_bytes())
                source=[words[p*512:(p+1)*512]for p in range(3)]
                active=[c['activity']=='all',c['activity']in ['all','cb'],c['activity']in ['all','cr']]
                cfg=configuration(yd,cd,False,offsets=c['offsets'],cabac=True)
                packets=[pcm(source,depths),p_packet((0,0),'res4',False,active=active,mode=c['mode'],init_idc=init_idc),echo(False,init_idc=init_idc)]
                for i,p in enumerate(packets):(OUT/f'{name}-{i}.packet').write_bytes(p)
                for ext,data in [('avcc',cfg),('mp4',mux(cfg,[(i,i==0,p)for i,p in enumerate(packets)],32,16,25)),('y16',struct.pack('<4608H',*words))]:(OUT/f'{name}.{ext}').write_bytes(data)
                cases.append(dict(c,name=name,init_idc=init_idc))
        (OUT/'witnesses.json').write_bytes((Path(tmp)/'witnesses.json').read_bytes())
    assert len(cases)==162
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n')
if __name__=='__main__':generate()
