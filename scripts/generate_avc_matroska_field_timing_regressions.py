"""Matroska timing companions of independently authored PAFF clips."""
import json,hashlib,struct
from pathlib import Path
from generate_avc_field_component_regressions import generate as intra
from generate_avc_field_inter_component_regressions import generate as inter
from generate_audio_resample_window_fixture import ebml
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-matroska-field-timing'
def container(config,packets,order,mode,invisible=None):
    track=ebml('d7',b'\1')+ebml('73c5',b'\1')+ebml('83',b'\1')+ebml('86',b'V_MPEG4/ISO/AVC')+ebml('63a2',config)+ebml('e0',ebml('b0',b'\x20')+ebml('ba',b'\x20'))
    if mode=='default':track+=ebml('23e383',(10_000_000).to_bytes(4,'big'))
    info=ebml('1549a966',ebml('2ad7b1',(1_000_000).to_bytes(3,'big'))+ebml('4489',struct.pack('>d',len(order)*20.))+ebml('4d80',b'fvid synthetic')+ebml('5741',b'fvid synthetic'))
    blocks=ebml('e7',b'\0')
    for index,packet in enumerate(packets):
        picture=order[index//2];second=index%2;offset=7 if mode=='unequal' else 10
        group=ebml('a1',b'\x81'+struct.pack('>h',picture*20+second*offset)+bytes([8 if invisible==index or isinstance(invisible,set) and index in invisible else 0])+packet)
        if mode in ['explicit','unequal']:group+=ebml('9b',bytes([20-offset if second else offset]))
        if mode=='overflow':group+=ebml('9b',(10_000_000_000_000).to_bytes(6,'big'))
        references=[]
        if picture:
            references=[-20,20] if order==[0,2,1] and picture==1 else [-40] if order==[0,2,1] else [-20]
        if second:references.append(-offset)
        for reference in sorted(set(references)):group+=ebml('fb',reference.to_bytes(1,'big',signed=True))
        blocks+=ebml('a0',group)
    header=ebml('1a45dfa3',ebml('4282',b'matroska')+ebml('4287',b'\4')+ebml('4285',b'\4'))
    return header+ebml('18538067',info+ebml('1654ae6b',ebml('ae',track))+ebml('1f43b675',blocks))
def generate():
    result={};cases=[];invalid=[]
    for source,folder,names in [(intra,'avc-field-components',['y8-c8-top-pcm','y8-c14-bottom-pcm','y14-c8-top-dc-filtered','y14-c14-bottom-pcm']), (inter,'avc-field-inter-components',['y8-c14-top-b-bi','y14-c8-bottom-p-residual','y8-c14-bottom-p-weighted','y14-c8-top-p-y'])]:
        files=source();manifest=json.loads(files['manifest.json'])
        for name in names:
            case=next(c for c in manifest['cases'] if c['file']==name+'.mp4');order=case.get('decode_order',list(range(len(case['frames']))));packets=[files[n] for n in case['packets']]
            for mode in ['explicit','default','unequal','untimed']:
                filename=name+'-'+mode+'.mkv';result[filename]=container(files[case['configuration']],packets,order,mode)
                cases.append(dict(case,file=filename,source_folder=folder,control=case['file'],timing=mode))
                if name=='y8-c14-bottom-p-weighted':
                    filename=name+'-hidden-reference-'+mode+'.mkv';result[filename]=container(files[case['configuration']],packets,order,mode,invisible={0,1})
                    cases.append(dict(case,file=filename,source_folder=folder,control=case['file'],timing=mode,frames=case['frames'][1:],origin_ns=20_000_000))
    files=intra();manifest=json.loads(files['manifest.json']);case=next(c for c in manifest['cases'] if c['file']=='y8-c14-top-pcm.mp4')
    config=files[case['configuration']];packets=[files[n] for n in case['packets']]
    for name,data,error in [
        ('unpaired.mkv',container(config,packets[:1],[0],'explicit'),'unpaired AVC field at end of Matroska'),
        ('duration-overflow.mkv',container(config,packets[:2],[0],'overflow'),'AVC field-pair duration overflow'),
        ('visibility-mismatch.mkv',container(config,packets[:2],[0],'explicit',invisible=1),'AVC field pair visibility differs')]:
        result[name]=data;invalid.append({'file':name,'error':error})
    result['manifest.json']=(json.dumps({'provenance':'36 authored PAFF Matroska companions of independently qualified PCM/DC/P/B component fixtures. Field durations10/10 or7/13ms; default and missing durations. No private media/encoder/network.','cases':cases,'invalid':invalid,'sha256':{k:hashlib.sha256(v).hexdigest() for k,v in result.items()}},indent=2)+'\n').encode()
    return result
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
