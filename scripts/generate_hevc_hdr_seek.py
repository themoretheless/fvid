"""Owned persistent HDR SEI before independent HEVC sequences; offline."""
import json,struct
from pathlib import Path
from generate_hevc_inband_layered_dispatch import inventory,configuration,ROOT
from generate_hevc_colour_plane_fixtures import escape
from hevc_fixture_mp4 import mux
from generate_matroska_parameter_seek import movie
OUT=ROOT/'hevc-hdr-seek'
SEED='hevc-pcm-reference-rext8'
HDR=[dict(primaries=[[8500,39850],[6550,2300],[35400,14600]],white=[15635,16450],maximum=10000000,minimum=1,cll=1000,fall=400),
     dict(primaries=[[13250,34500],[7500,3000],[34000,16000]],white=[15635,16450],maximum=15000000,minimum=10,cll=1400,fall=500),
     dict(primaries=[[8500,39850],[6550,2300],[35400,14600]],white=[15635,16450],maximum=8000000,minimum=2,cll=750,fall=250)]
def sei(messages):
    return b'\x4e\x01'+escape(b''.join(bytes([kind,len(payload)])+payload for kind,payload in messages)+b'\x80')
def packet(units):return b''.join(len(n).to_bytes(4,'big')+n for n in units)
def generate(hdr_values=HDR):
    cfg,pictures,size=inventory(ROOT/f'{SEED}.mp4');params=configuration(cfg)
    full=[n for k in [32,33,34]for n in params[k]];files={};cases=[];messages=[]
    for i,hdr in enumerate(hdr_values):
        master=struct.pack('>8HII',*[v for p in hdr['primaries']+[hdr['white']]for v in p],hdr['maximum'],hdr['minimum'])
        light=struct.pack('>HH',hdr['cll'],hdr['fall'])
        pair=[(137,master),(144,light)];messages.append(pair)
        for label,values in [('full',pair),('master',pair[:1]),('light',pair[1:])]:
            files[f'{i}-{label}.packet']=packet([sei(values)])
    files['bad-parameters.packet']=packet([sei(messages[2]),b'\x44\x01\xc0'])
    initial=sei(messages[0])
    files['configured.hvcc']=cfg[:22]+bytes([cfg[22]+1])+cfg[23:]+bytes([39])+struct.pack('>HH',1,len(initial))+initial
    for layout in ['inline','prefix','split']:
        groups=[];indices=[]
        for cycle in range(3):
            nals=[sei(messages[cycle])] if layout!='split' else [sei([m])for m in messages[cycle]]
            if layout!='inline':groups.extend([[n]for n in nals])
            start=len(groups);indices.append(list(range(start,start+len(pictures))))
            groups.extend([list(p)for p in pictures])
            if layout=='inline':groups[start]=nals+groups[start]
        stream=b''.join(b'\0\0\0\1'+n for n in full+[n for g in groups for n in g])
        for inband in [False,True]:
            name=layout+('-hev1' if inband else '-hvc1')+'.mp4'
            files[name]=mux(stream,8,*size,access_units=groups,inband_parameters=inband)
            cases.append(dict(file=name,kind='mp4',layout=layout,indices=indices))
        name=layout+'.mkv';keys={i[0]for i in indices}
        files[name]=movie('V_MPEGH/ISO/HEVC',cfg,[packet(g)for g in groups],size,keys,False)
        cases.append(dict(file=name,kind='mkv',layout=layout,indices=indices))
    files['manifest.json']=(json.dumps(dict(seed=SEED,frames=len(pictures),hdr=hdr_values,cases=cases),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for n,data in generate().items():(OUT/n).write_bytes(data)
