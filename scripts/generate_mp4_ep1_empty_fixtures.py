"""Own real EP1 MP4s with explicit empty ESC AUs, including fragmented tracks."""
import json
from generate_mp4_ep1_fixtures import SOURCE,make_movie,word
from generate_he_aac_packet_fixtures import box,boxes
OUT=SOURCE/'mp4-ep1-empty'
def fragmented(source,mode):
    parts=list(boxes(source));assert parts[-1][0]==b'moov';tracks=[];prefix=b''.join(box(t,p)for t,p in parts[:-1]);newmoov=[];defaults=[]
    for tag,body in boxes(parts[-1][1]):
        if tag!=b'trak':newmoov.append(box(tag,body));continue
        mdia=dict(boxes(body))[b'mdia'];md=dict(boxes(mdia));handler=md[b'hdlr'][8:12]
        if handler!=b'soun':newmoov.append(box(tag,body));continue
        track_id=int.from_bytes(dict(boxes(body))[b'tkhd'][12:16],'big');stbl=dict(boxes(dict(boxes(md[b'minf']))[b'stbl']));count=int.from_bytes(stbl[b'stsz'][8:12],'big');assert count==3
        sizes=[int.from_bytes(stbl[b'stsz'][12+4*i:16+4*i],'big')for i in range(count)];offset=int.from_bytes(stbl[b'stco'][8:12],'big');raw=[]
        for size in sizes:raw.append(source[offset:offset+size]);offset+=size
        step=int.from_bytes(stbl[b'stts'][12:16],'big');padb=stbl[b'padb'][8:];padding=[((padb[i//2]>>(4 if i%2==0 else 0))&7)for i in range(count)]
        empty=not any(sizes);tracks.append(dict(id=track_id,raw=raw,step=step,padding=padding,empty=empty))
        defaults.append(box(b'trex',word(0,track_id,1,step,0 if mode=='trex-zero'and empty else 7,0)))
        def rewrite(t,p):
            if t in [b'mdia',b'minf',b'stbl']:p=b''.join(rewrite(t,p)for t,p in boxes(p))
            elif t in [b'stts',b'stsc',b'stco']:p=word(0,0)
            elif t==b'stsz':p=word(0,0,0)
            elif t==b'padb':p=word(0,0)
            elif t==b'mdhd':p=p[:16]+word(0)+p[20:]
            return box(t,p)
        newmoov.append(box(tag,b''.join(rewrite(t,p)for t,p in boxes(body))))
    def moof(base):
        offset=base;traf=[];bad=False
        for track in tracks:
            empty=track['empty'];inherited=empty and mode in ['tfhd-zero','trex-zero'];size_flag=not(empty and mode=='trex-zero')
            tfhd=box(b'tfhd',word(0x020008|(0x10 if size_flag else 0),track['id'],track['step'])+(word(0 if inherited else 4)if size_flag else b''))
            entries=b''
            for raw,pad in zip(track['raw'],track['padding']):
                if mode=='padding-empty' and not raw and not bad:pad=1;bad=True
                entries+=(word(len(raw))if not inherited else b'')+word(pad<<17)
            trun=box(b'trun',word(0x401|(0 if inherited else 0x200),3,offset)+entries)
            traf.append(box(b'traf',tfhd+box(b'tfdt',word(0,0))+trun));offset+=sum(map(len,track['raw']))
        return box(b'moof',box(b'mfhd',word(0,1))+b''.join(traf))
    probe=moof(0);payload=b''.join(raw for t in tracks for raw in t['raw'])
    return prefix+box(b'moov',b''.join(newmoov)+box(b'mvex',b''.join(defaults)))+moof(len(probe)+8)+box(b'mdat',payload)
def bad_padding(source):
    changed=False
    def rewrite(t,p):
        nonlocal changed
        if t in [b'moov',b'trak',b'mdia',b'minf',b'stbl']:
            if t==b'stbl':
                child=dict(boxes(p));sizes=child[b'stsz'];count=int.from_bytes(sizes[8:12],'big');rows=[int.from_bytes(sizes[12+4*i:16+4*i],'big')for i in range(count)] if int.from_bytes(sizes[4:8],'big')==0 else []
                if not changed and 0 in rows and b'padb'in child:
                    index=rows.index(0);pad=bytearray(child[b'padb']);pad[8+index//2]|=0x10 if index%2==0 else 1
                    changed=True;return box(t,b''.join(box(a,bytes(pad)if a==b'padb'else b)for a,b in boxes(p)))
            p=b''.join(rewrite(t,p)for t,p in boxes(p))
        return box(t,p)
    result=b''.join(rewrite(t,p)for t,p in boxes(source));assert changed;return result

def unused_ep1_description(ordinary,valid):
    def setups(movie):
        result={}
        for t,p in boxes(dict(boxes(movie))[b'moov']):
            if t!=b'trak':continue
            children=dict(boxes(p));mdia=dict(boxes(children[b'mdia']))
            if mdia[b'hdlr'][8:12]!=b'soun':continue
            ident=int.from_bytes(children[b'tkhd'][12:16],'big');stbl=dict(boxes(dict(boxes(mdia[b'minf']))[b'stbl']))
            result[ident]=stbl[b'stsd'][8:]
        return result
    entries=setups(valid)
    def rewrite(t,p,entry=None):
        if t==b'trak':
            ident=int.from_bytes(dict(boxes(p))[b'tkhd'][12:16],'big');entry=entries.get(ident)
        if t in [b'moov',b'trak',b'mdia',b'minf',b'stbl']:p=b''.join(rewrite(t,p,entry)for t,p in boxes(p))
        elif t==b'stsd'and entry is not None:p=p[:4]+word(2)+p[8:]+entry
        return box(t,p)
    return b''.join(rewrite(t,p)for t,p in boxes(ordinary))

def generate():
    parent=SOURCE/'aac-ep1';m=json.loads((parent/'manifest.json').read_bytes());units=(parent/'access-units.bin').read_bytes();files={};cases=[];bad=[]
    names=['aac-er-ltp-resilient-960-7-mono','sbr-mono-17-15-64-explicit','sbr-multi-19-14-16-64']
    for base in m['cases']:
        if base['name']not in names:continue
        name=base['name'];seed=(parent/base['reference_video']).read_bytes();movie=make_movie(base,seed,units,allow_empty=True)
        files[name+'.f32le']=(parent/base['golden']).read_bytes()[:3*base['container_frame_samples']*base['channels']*4]
        for mode in ['indexed','per-row','tfhd-zero','trex-zero']:
            video=name+'-'+mode+'.mp4';files[video]=movie if mode=='indexed'else fragmented(movie,mode)
            cases.append(dict(name=name+'-'+mode,video=video,golden=name+'.f32le',channels=base['channels'],rate=base['rate'],frames=3,mode=mode))
        if name==names[0]:
            files['padding-on-empty.mp4']=bad_padding(movie);bad.append(dict(video='padding-on-empty.mp4',error='MP4 sample padding exceeds stored bits'))
            files['fragment-padding-on-empty.mp4']=fragmented(movie,'padding-empty');bad.append(dict(video='fragment-padding-on-empty.mp4',error='MP4 sample padding exceeds stored bits'))
            files['missing-empty.mp4']=make_movie(base,seed,units,'omit-empty',allow_empty=True);bad.append(dict(video='missing-empty.mp4',kind='group',error='EP1 MP4 missing simultaneous ESC access unit'))
            ordinary=dict(base,asc=base['plain_asc']);files['ordinary-zero.mp4']=make_movie(ordinary,seed,units,allow_empty=True);bad.append(dict(video='ordinary-zero.mp4',error='zero MP4 sample size'))
            files['unused-ep1-description.mp4']=unused_ep1_description(files['ordinary-zero.mp4'],movie);bad.append(dict(video='unused-ep1-description.mp4',error='zero MP4 sample size'))
    assert len(cases)==12
    files['manifest.json']=(json.dumps(dict(cases=cases,malformed=bad,provenance='Own AVC/ER-LTP/SBR movies with real separate EP1 ESC tracks. Explicit empty AUs retain source clocks and dependencies; they are not omitted AUs. Independent saved PCM. No foreign codec, private media, process or network.'),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
