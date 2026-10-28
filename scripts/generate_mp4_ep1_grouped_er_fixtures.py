"""Own ER-LC/LTP/LD grouped EP1 AUs, independent saved PCM, offline."""
import json
from generate_mp4_ep1_fixtures import SOURCE,make_movie
from generate_mp4_ep1_grouped_sbr_fixtures import grouped
OUT=SOURCE/'mp4-ep1-grouped-er'
def generate():
    parent=SOURCE/'aac-ep1';m=json.loads((parent/'manifest.json').read_bytes());blob=(parent/'access-units.bin').read_bytes();files={};cases=[];long_cases=[]
    for base in m['cases']:
        if base['aot']==39:continue
        name=base['name'];movie=make_movie(base,(parent/base['reference_video']).read_bytes(),blob,allow_empty=True,frame_count=3,file_dependencies=True)
        # Category/occurrence stream IDs are assigned in dependency order, but
        # CPE cores repeat categories. Follow the explicit chain instead.
        units=base['frames'][0]['units'];chain=[];previous=None
        while len(chain)<len(units):
            unit=next(u for u in units if u['depends_on']==previous);chain.append(unit);previous=unit['stream_id']
        count=len(chain);core=next((i for i,u in enumerate(chain)if u['category']==5),count)
        files[name+'-control.mp4']=movie
        files[name+'.f32le']=(parent/base['golden']).read_bytes()[:3*base['container_frame_samples']*base['channels']*4]
        variants={'whole':[list(range(count))], 'core':[list(range(core))]+[[i]for i in range(core,count)], 'zigzag':[list(range(i,min(i+2,count)))for i in range(0,count,2)]}
        for mode,parts in variants.items():
            video=name+'-'+mode+'.mp4';files[video]=grouped(movie,0,'custom',partitions=parts)[0];bad=[]
            if base['frames'][1]['alignment_bits']:
                final=chain[-1]['stream_id'];unit=next(u for u in base['frames'][1]['units']if u['stream_id']==final)
                raw=blob[unit['offset']:unit['offset']+unit['bytes']];bits=''.join(f'{v:08b}'for v in raw)[:unit['bits']]
                edits={(1,count-1):bits[:-1]+'1'};broken=name+'-'+mode+'-alignment.mp4'
                files[broken]=grouped(movie,0,'custom',partitions=parts,edits=edits)[0];bad.append(dict(video=broken,error='nonzero AAC ESC alignment bits'))
            cases.append(dict(name=name+'-'+mode,video=video,control=name+'-control.mp4',golden=name+'.f32le',rate=base['rate'],channels=base['channels'],n=base['n'],frames=3,aot=base['aot'],streams=len(parts),partitions=parts,malformed=bad))
            if name=='aac-er-ltp-resilient-1024-7-common-2'and mode=='zigzag':
                unit=next(u for u in base['frames'][0]['units']if u['stream_id']==chain[-1]['stream_id'])
                bits=''.join(f'{v:08b}'for v in blob[unit['offset']:unit['offset']+unit['bytes']])[:unit['bits']]
                assert bits=='0000'
                broken=name+'-'+mode+'-terminal-alignment.mp4'
                files[broken]=grouped(movie,0,'custom',partitions=parts,edits={(0,count-1):'0001'})[0]
                cases[-1]['terminal_alignment']=broken

    # A single grouped track can retain the complete source sequence even when
    # TNS categories appear/disappear internally; no physical track is omitted.
    from generate_he_aac_packet_fixtures import packed
    logical=(parent/'logical-packets.bin').read_bytes()
    for base in m['cases'][:6]:
        payload=bytearray();rows=[]
        for index,frame in enumerate(base['frames']):
            previous=None;bits='';qty=0
            while qty<len(frame['units']):
                unit=next(u for u in frame['units']if u['depends_on']==previous)
                bits+=''.join(f'{v:08b}'for v in blob[unit['offset']:unit['offset']+unit['bytes']])[:unit['bits']]
                previous=unit['stream_id'];qty+=1
            data=packed(bits)
            assert data==logical[frame['offset']:frame['offset']+frame['bytes']]
            rows.append(dict(units=[dict(stream_id=1,depends_on=None,offset=len(payload),bytes=len(data),bits=len(bits),timescale=24000,duration=base['n'])]));payload.extend(data)
        fixture=dict(base,frames=rows);video=base['name']+'-complete-sequence.mp4'
        files[video]=make_movie(fixture,(parent/base['reference_video']).read_bytes(),payload,allow_empty=True,frame_count=len(rows),file_dependencies=True)
        long_cases.append(dict(name=base['name'],video=video,control='../aac-ep1/'+base['reference_video'],golden='../aac-ep1/'+base['golden'],rate=base['rate'],channels=base['channels'],n=base['n'],frames=len(rows),aot=base['aot']))
    assert len(cases)==282,len(cases)
    files['manifest.json']=(json.dumps(dict(cases=cases,long_cases=long_cases,provenance='Own ER-LC/LTP/LD core and standard SBR, explicit/sync signaling, mono/multichannel layouts; complete, core-only and pairwise grouped EP1 AUs. Saved independent scalar PCM from own sources. Corrupt final logical alignment remains inside declared transport bits. No private media, foreign codec, subprocess or network.'),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
