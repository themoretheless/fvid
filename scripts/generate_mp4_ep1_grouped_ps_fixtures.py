"""Own grouped PS EP1 movies, saved logical/core controls, offline only."""
import json
from generate_mp4_ep1_fixtures import SOURCE
from generate_mp4_ep1_grouped_sbr_fixtures import grouped
from generate_mp4_ep1_empty_fixtures import fragmented
from generate_mp4_ep1_active_chain_fixtures import streams
from generate_aac_er_pce_fixtures import build_er_pce_asc
from generate_aac_esds_stream_fixtures import descriptor
from generate_he_aac_packet_fixtures import box,boxes
OUT=SOURCE/'mp4-ep1-grouped-ps'
def replace_asc(movie,asc):
    esds=box(b'esds',bytes(4)+descriptor(3,bytes(3)+descriptor(4,b'\x40\x14'+bytes(11)+descriptor(5,asc))+descriptor(6,b'\x02')))
    def rewrite(t,p,audio=False):
        if t==b'trak':audio=dict(boxes(dict(boxes(p))[b'mdia']))[b'hdlr'][8:12]==b'soun'
        if t in [b'moov',b'trak',b'mdia',b'minf',b'stbl']:p=b''.join(rewrite(k,v,audio)for k,v in boxes(p))
        elif t==b'stsd'and audio:
            entries=[]
            for kind,entry in boxes(p[8:]):
                assert kind==b'mp4a'
                entries.append(box(kind,entry[:28]+b''.join(esds if k==b'esds'else box(k,v)for k,v in boxes(entry[28:]))))
            p=p[:8]+b''.join(entries)
        return box(t,p)
    return b''.join(rewrite(t,p)for t,p in boxes(movie))

def generate():
    parent=SOURCE/'mp4-ep1-ps';m=json.loads((parent/'manifest.json').read_bytes());files={};cases=[]
    for base in m['cases']:
        if base['mode']=='fragmented':continue
        original=(parent/base['video']).read_bytes();stem=base['name'].rsplit('-',1)[0]
        for pce in [False,True]:
            source=original;plain_asc=base['plain_asc'];asc=base['asc'];logical='../mp4-ep1-ps/'+base['logical'];label=stem
            if pce:
                label+='-pce';aot=19 if 'aot19'in stem else 17
                options=dict(aot=aot,short_frame=base['n']==960,rate=24000,front=[(False,3)],side=[],back=[],lfe=[],pce_profile=3 if aot==19 else 1,ext_rate=base['rate'])
                if base['signal']=='explicit':options['outer_aot']=29
                elif base['signal']=='sync':options.update(sync_sbr=True,sync_ps=True)
                plain_asc=build_er_pce_asc(**options).hex();asc=build_er_pce_asc(**options,ep_config=1).hex()
                originals=streams(original);edits={}
                for i,data in enumerate(originals[0]['raw']):
                    bits=''.join(f'{v:08b}'for v in data)[:len(data)*8-originals[0]['pads'][i]];assert bits[:4]=='0000'
                    edits[i,0]='0011'+bits[4:]
                source=replace_asc(grouped(original,0,'custom',partitions=[[i]for i in range(4)],edits=edits)[0],bytes.fromhex(asc))
                logical=label+'.bin';raw=bytearray((parent/base['logical']).read_bytes())
                for row in base['frames']:
                    at=row['offset'];assert raw[at]>>4==0;raw[at]=(raw[at]&15)|0x30
                files[logical]=bytes(raw)
            variants={'whole':[[0,1,2,3]],'core':[[0,1],[2],[3]],'zigzag':[[0,1],[2,3]]}
            for grouping,parts in variants.items():
                movie,_=grouped(source,0,'custom',partitions=parts)
                for mode in (['indexed','fragmented']if base['mode']=='indexed'else['edits']):
                    name=label+'-'+grouping+'-'+mode;video=name+'.mp4'
                    files[video]=fragmented(movie,'per-row')if mode=='fragmented'else movie
                    control='../mp4-ep1-ps/'+stem+'-'+mode+'.mp4'
                    if pce:
                        control=label+'-'+mode+'-control.mp4';files[control]=fragmented(source,'per-row')if mode=='fragmented'else source
                    case=dict(base,name=name,video=video,mode=mode,grouping=grouping,partitions=parts,control=control,plain_asc=plain_asc,asc=asc,logical=logical,pce=pce)
                    case['core_reference']='../mp4-ep1-ps/'+base['core_reference']
                    if pce and grouping=='whole'and mode=='indexed':
                        track=streams(source)[0];data=track['raw'][1]
                        bits=''.join(f'{v:08b}'for v in data)[:len(data)*8-track['pads'][1]];assert bits[:4]=='0011'
                        broken=label+'-absent-tag.mp4';files[broken]=grouped(source,0,'custom',partitions=parts,edits={(1,0):'1011'+bits[4:]})[0]
                        case['malformed_tag']=broken
                    cases.append(case)
    assert len(cases)==432
    files['manifest.json']=(json.dumps(dict(cases=cases,provenance='Own six-frame ER-LC/LTP PS in whole/core/pairwise grouped EP1 AUs; indexed, fragmented and edit-list storage; explicit/sync/implicit PS, core/double rates, ordinary and explicit tagged PCE configurations. Saved logical unprotected control and independent core PCM with shared PS DSP stage, not an independent complete PS oracle. No private media, foreign codec, process or network.'),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
