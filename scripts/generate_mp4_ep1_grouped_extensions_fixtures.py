"""Own grouped ER extension and PCE/tagged-core controls, offline only."""
import json
from generate_mp4_ep1_fixtures import SOURCE,make_movie
from generate_mp4_ep1_grouped_sbr_fixtures import grouped
from generate_he_aac_packet_fixtures import packed,field
from generate_aac_esc_extensions_fixtures import sources,extensions
from generate_aac_er_pce_fixtures import build_er_pce_asc
OUT=SOURCE/'mp4-ep1-grouped-extensions'
def make_control(base,rows,asc,seed):
    blob=bytearray();frames=[]
    assert all([p['category']for p in r]==[p['category']for p in rows[0]]for r in rows)
    for row in rows:
        units=[]
        for i,p in enumerate(row):
            bits=p['bits'];data=bytearray(packed(bits));scale=1+i%3
            if len(bits)%8:data[-1]|=(1<<(8-len(bits)%8))-1
            units.append(dict(stream_id=i+1,depends_on=i if i else None,offset=len(blob),bytes=len(data),bits=len(bits),timescale=24000*scale,duration=base['n']*scale));blob.extend(data)
        frames.append(dict(units=units))
    return make_movie(dict(base,asc=asc.hex(),frames=frames),seed,blob,allow_empty=True,frame_count=3,file_dependencies=True)
def generate():
    files={};cases=[];source={c['name']:c for c in sources()if c['aot']!=39}
    def emit(base,rows,asc,seed,gold,core,mutations,pce_tags=None):
        name=base['name'];control=make_control(base,rows,asc,seed);files[name+'-control.mp4']=control;files[name+'.f32le']=gold
        variants={'whole':[list(range(len(rows[0])))], 'extensions':([[i]for i in range(core)]+[list(range(core,len(rows[0])))])if core<len(rows[0])else[list(range(core))], 'zigzag':[list(range(i,min(i+2,len(rows[0]))))for i in range(0,len(rows[0]),2)]}
        for mode,parts in variants.items():
            video=name+'-'+mode+'.mp4';files[video]=grouped(control,0,'custom',partitions=parts)[0];bad=[]
            for label,index,bits,error in mutations:
                broken=name+'-'+mode+'-'+label+'.mp4';files[broken]=grouped(control,0,'custom',partitions=parts,edits={(1,index):bits})[0];bad.append(dict(video=broken,error=error))
            cases.append(dict(name=name+'-'+mode,video=video,control=name+'-control.mp4',golden=name+'.f32le',rate=base['rate'],channels=base['channels'],n=base['n'],frames=3,aot=base['aot'],partitions=parts,streams=len(parts),kind=base['kind'],pce_tags=pce_tags,malformed=bad))
    parent=SOURCE/'aac-esc-extensions';m=json.loads((parent/'manifest.json').read_bytes());info=(parent/'instances.bin').read_bytes()
    for base in m['cases']:
        if base['aot']==39:continue
        stem=base['name'][:-(len(base['kind'])+1)];original=source[stem]
        rows=[[dict(category=p['category'],bits=''.join(f'{v:08b}'for v in info[p['offset']:p['offset']+p['bytes']])[:p['bits']])for p in row['instances']]for row in base['frames'][:3]]
        core=base['frames'][0]['core_classes'];bad=[]
        if base['kind']=='fill-data':
            bits=rows[1][core+1]['bits'];bad=[('nibble',core+1,'0001'+bits[4:],'invalid AAC fill-data nibble'),('byte',core+1,bits[:4]+field(0x5a,8)+bits[12:],'invalid AAC fill-data byte')]
        if base['kind']=='anc-short':
            bits=rows[1][core+1]['bits'];bad=[('length',core+1,field(8,8)+bits[8:],'AAC ancillary data exceeds fill payload')]
        gold=(parent/base['golden']).read_bytes()[:3*base['n']*base['channels']*4]
        emit(base,rows,packed(original['setup']+'01'),(parent/base['video']).read_bytes(),gold,core,bad)
    # Retag existing own ER core samples into matching explicit PCE rosters.
    parent=SOURCE/'aac-ep1';m=json.loads((parent/'manifest.json').read_bytes());blob=(parent/'access-units.bin').read_bytes()
    for original in m['cases'][:6]:
        cfg=original['configuration'];kinds={1:[0],2:[1],6:[0,1,1,3]}[cfg]
        tags={1:[3],2:[5],6:[3,5,7,9]}[cfg];front=[(False,3),(True,5)]if cfg==6 else[(cfg==2,tags[0])];back=[(True,7)]if cfg==6 else[];lfe=[9]if cfg==6 else[]
        flags=original['flags'];asc=build_er_pce_asc(original['aot'],original['n']in[480,960],24000,front,[],back,lfe,resilience=tuple(bool(flags&(1<<i))for i in [2,1,0]),pce_profile=3 if original['aot']==19 else 1,ep_config=1)
        for chain in [False,True]:
            rows=[];tag_indices=[];core=0
            for frame in original['frames'][:3]:
                units=[];previous=None
                while len(units)<len(frame['units']):
                    u=next(u for u in frame['units']if u['depends_on']==previous);units.append(u);previous=u['stream_id']
                row=[dict(category=u['category'],bits=''.join(f'{v:08b}'for v in blob[u['offset']:u['offset']+u['bytes']])[:u['bits']])for u in units]
                if frame['alignment_bits']:row[-1]['bits']=row[-1]['bits'][:-frame['alignment_bits']]
                cursor=0;tag_indices=[]
                for kind,tag in zip(kinds,tags):
                    assert row[cursor]['category']==(0 if kind==1 else 1)
                    tag_indices.append(cursor);row[cursor]['bits']=field(tag,4)+row[cursor]['bits'][4:]
                    if kind==1:cursor+=1
                    for _ in range(2 if kind==1 else 1):
                        while row[cursor]['category']!=4:cursor+=1
                        cursor+=1
                assert cursor==len(row);core=len(row)
                if chain:row+=extensions(original,len(rows),'chain')
                row[-1]['bits']+='0'*(-sum(len(p['bits'])for p in row)%8);rows.append(row)
            base=dict(original,name=original['name']+('-pce-chain'if chain else'-pce-core'),kind='pce-chain'if chain else'pce-core')
            index=tag_indices[-1];bits=rows[1][index]['bits'];bad=[('tag',index,str(1-int(bits[0]))+bits[1:],'AAC element is absent from PCE')]
            gold=(parent/original['golden']).read_bytes()[:3*original['n']*original['channels']*4]
            emit(base,rows,asc,(parent/original['reference_video']).read_bytes(),gold,core,bad,tags)
    assert len(cases)==198,len(cases)
    files['manifest.json']=(json.dumps(dict(cases=cases,provenance='Own ER core packets and independent scalar PCM; grouped categories 5..8, ancillary escapes, DRC syntax, fill/reserved areas; explicit PCE/tagged core and PCE+DRC/ancillary chains with matching native speaker order. DRC metadata is parsed, optional gain application is unchanged. Bad fill/length and absent PCE tag occur at frame 1. No private media, foreign codec, process or network.'),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
