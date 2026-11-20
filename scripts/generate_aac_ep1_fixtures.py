"""Own separate-ESC EP1 frames and short logical reference movies, offline.

Reference MP4/Matroska are epConfig=0 projections, not EP1 container acceptance.
EP1 data are the saved individual AUs, dependency graph, clock and exact bit sizes.
"""
import json
from pathlib import Path
from generate_aac_esc_extensions_fixtures import sources as core_sources
from generate_aac_eld_fixtures import mkv
from generate_he_aac_packet_fixtures import packed,video_fixture
ROOT=Path(__file__).resolve().parents[1];SOURCE=ROOT/'tests/fixtures/playback-errors';OUT=SOURCE/'aac-ep1'
def text(raw):return ''.join(f'{v:08b}'for v in raw)
def ep1(plain):
    bits=text(bytes.fromhex(plain));aot=int(bits[:5],2);at=25 if aot in [5,29]else 16
    assert bits[at:at+2]=='00';return packed(bits[:at]+'01'+bits[at+2:])
def sources():
    for c in core_sources():yield dict(c,asc=packed(c['setup']+'01'),ps=False)
    directory=SOURCE/'aac-esc-sbr';manifest=json.loads((directory/'manifest.json').read_bytes());info=(directory/'instances.bin').read_bytes()
    for key in ['cases','ps_cases']:
        for c in manifest[key]:
            frames=[]
            for row in c['frames']:
                parts=[dict(category=p['category'],bits=text(info[p['offset']:p['offset']+p['bytes']])[:p['bits']])for p in row['instances']]
                core=sum(len(p['bits'])for p in parts[:next((i for i,p in enumerate(parts)if p['category']==5),len(parts))]);frames.append(dict(wire=''.join(p['bits']for p in parts),esc=parts,alignment_bits=(-core)%8))
            base=dict(name='sbr-'+c['name'],aot=c['aot'],n=c['n'],rate=c['rate'],channels=c['channels'],configuration=c['configuration'],plain_asc=c['plain_asc'],asc=ep1(c['plain_asc']),frames=frames,ps=key=='ps_cases',signal=c.get('signal','explicit'))
            if key=='cases':base['golden']=(directory/c['golden']).read_bytes()
            else:base.update(core_reference=c['core_reference'],core_pcm=(directory/c['core_reference']).read_bytes(),payloads=c['payloads'])
            yield base

def alignment_movies(files):
    from copy import deepcopy
    from generate_aac_eld_esc_fixtures import configuration,protect
    blob=bytearray();cases=[]
    for base in core_sources():
        source=[]
        for frame in base['frames']:
            parts=deepcopy(frame['esc']);pad=(-len(frame['wire']))%8;parts[-1]['bits']+='0'*pad;source.append(dict(esc=parts,pad=pad))
        bad_index=next(i for i,row in enumerate(source)if i>=1 and row['pad'])
        patterns=list(dict.fromkeys(tuple(p['category']for p in row['esc'])for row in source));orders=[list(range(len(p)))[::-1]if i%2 else list(range(len(p)))for i,p in enumerate(patterns)]
        _,_,_,ep=configuration(dict(rate=24000,configuration=1,n=512,flags=0),patterns,orders);asc=packed(base['setup']+'11'+ep+'1');rows=[];packets=[]
        for i,original in enumerate(source):
            good,_=protect(original,patterns,orders);row=dict(good_offset=len(blob),good_bytes=len(good));blob.extend(good);bad=deepcopy(original)
            if i==bad_index:bad['esc'][-1]['bits']=bad['esc'][-1]['bits'][:-bad['pad']]
            wire,_=protect(bad,patterns,orders);row.update(offset=len(blob),bytes=len(wire),alignment_bits=original['pad']);blob.extend(wire);rows.append(row);packets.append(wire)
        name=base['name']+'-missing-ep3-alignment';c=dict(name=name,asc=asc.hex(),frames=rows,bad_index=bad_index,channels=base['channels'],n=base['n'],rate=base['rate'],container_rate=base['rate'],container_frame_samples=base['n']*base['rate']//24000,samples=len(rows)*base['n']*base['rate']//24000,pcm_offset=0,slots=base['n']//64,bands=64 if base['rate']==48000 else 32,video=name+'.mp4',matroska=name+'.mka')
        files[c['video']]=video_fixture([c],blob,base['channels'],filename=c['video'],write=False)['data'];files[c['matroska']]=mkv(asc,packets,base['rate'],c['container_frame_samples'],base['channels']);cases.append(c)
    files['ep3-alignment-packets.bin']=bytes(blob);return cases

def generate():
    files={};cases=[];ps_cases=[];blob=bytearray();plain=bytearray()
    for base in sources():
        name=base['name'];rows=[];packets=[]
        for index,frame in enumerate(base['frames']):
            counts={};units=[];parent=None;pts=-37+index*base['n'];duration=base['n']
            for ordinal,part in enumerate(frame['esc']):
                category=part['category'];occurrence=counts.get(category,0);counts[category]=occurrence+1;stream=category*1024+occurrence*7+19
                bits=part['bits']+('0'*(-len(frame['wire'])%8)if ordinal+1==len(frame['esc'])else'');data=bytearray(packed(bits))
                # Low bits outside the access unit deliberately differ from
                # the logical AAC alignment, which remains zero inside the AU.
                if len(bits)%8:data[-1]|=(1<<(8-len(bits)%8))-1
                scale=1 if ordinal==0 else 1+ordinal%3
                units.append(dict(stream_id=stream,depends_on=parent,offset=len(blob),bytes=len(data),bits=len(bits),pts=pts*scale,duration=duration*scale,timescale=24000*scale,category=category))
                blob.extend(data);parent=stream
            # Transmission order is deliberately unrelated to dependency order.
            units=units[::-1]if index%2 else units[1:]+units[:1]
            wire=packed(frame['wire']);row=dict(units=units,alignment_bits=frame.get('alignment_bits',(-len(frame['wire']))%8),pts=pts,duration=duration,timescale=24000,offset=len(plain),bytes=len(wire));plain.extend(wire);packets.append(wire);rows.append(row)
        c={k:v for k,v in base.items()if k not in ['frames','golden','asc','setup','core_pcm']};c.update(asc=base['asc'].hex(),frames=rows,container_rate=base['rate'],container_frame_samples=base['n']*base['rate']//24000,samples=len(rows)*base['n']*base['rate']//24000,pcm_offset=0,slots=base['n']//64,bands=64 if base['rate']==48000 else 32)
        files[name+'.asc']=base['asc']
        if 'golden'in base:files[name+'.f32le']=base['golden'];c['golden']=name+'.f32le'
        if base['ps']:files[base['core_reference']]=base['core_pcm']
        # These videos are the logical projection reference for codec PCM and
        # navigation. They do not pretend that a single track carries EP1 AUs.
        movie=dict(c,asc=base['plain_asc']);c['reference_video']=name+'-logical.mp4';c['reference_matroska']=name+'-logical.mka'
        files[c['reference_video']]=video_fixture([movie],plain,base['channels'],filename=c['reference_video'],write=False)['data'];files[c['reference_matroska']]=mkv(bytes.fromhex(base['plain_asc']),packets,base['rate'],c['container_frame_samples'],base['channels'])
        (ps_cases if base['ps']else cases).append(c)
    ep3_alignment=alignment_movies(files)
    files['access-units.bin']=bytes(blob);files['logical-packets.bin']=bytes(plain);files['manifest.json']=(json.dumps(dict(cases=cases,ps_cases=ps_cases,ep3_alignment=ep3_alignment,provenance='Own separate ESC AUs in stable category/occurrence streams, exact bit lengths and signed rational clocks; independent sparse-core/QMF/CLDFB references. Logical reference movies use epConfig=0; EP1 container routing remains separate. No private source data, foreign codec, process or network.'),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
