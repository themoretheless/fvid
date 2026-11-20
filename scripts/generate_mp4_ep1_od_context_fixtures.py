"""Own LD-SBR repeated-source/different-OD-domain playback qualification."""
import json
from generate_mp4_ep1_od_fixtures import author,fragmented,update,add,clear,half,remove
from generate_mp4_ep1_od_edits_fixtures import edited
from generate_mp4_ep1_epochs_fixtures import eld_control
from generate_mp4_ep1_fixtures import SOURCE,word
from generate_aac_esds_stream_fixtures import descriptor
from generate_he_aac_packet_fixtures import box,boxes
OUT=SOURCE/'mp4-ep1-od-context'
def replay(source,n,refs,form,wrong=False):
    indices=[refs.index(i)+1 for i in range(2,len(refs)+2)];root=indices[:1];children=indices[1:]
    first=[update(root),update(indices),update(root)]if form=='snapshots'else[update(root),add(children),remove(children)]if form=='deltas'else[update(root),clear()+update(indices),clear()+update(root)]
    second=[clear()+update(indices if wrong else root,3),update(indices,3),update(root,3)]
    if form=='deltas':second[1:]=[add(children,3),descriptor(7,half(3<<6)+b''.join(half(i)for i in children))]
    if form=='replace':second[1:]=[descriptor(2,b'\x00\xc0')+update(indices,3),descriptor(2,b'\x00\xc0')+update(root,3)]
    packets=first+second;parts=list(boxes(source));last=max(i for i,(t,p)in enumerate(parts)if t==b'mdat')
    offset=sum(8+len(p)for t,p in parts[:last])+8+len(parts[last][1]);parts[last]=(b'mdat',parts[last][1]+b''.join(packets))
    def track(body):
        d=dict(boxes(body));md=dict(boxes(d[b'mdia']))
        if md[b'hdlr'][8:12]!=b'odsm':return body
        tables={b'stts':word(0,1,6,12*n),b'stsz':word(0,0,6)+word(*map(len,packets)),b'stsc':word(0,1,1,6,1),b'stco':word(0,1,offset)}
        def rewrite(t,p):
            if t in [b'mdia',b'minf',b'stbl']:p=b''.join(rewrite(k,v)for k,v in boxes(p))
            elif t==b'mdhd':p=p[:16]+word(72*n)+p[20:]
            elif t in tables:p=tables[t]
            return box(t,p)
        return b''.join(rewrite(t,p)for t,p in boxes(body))
    return b''.join(box(t,b''.join(box(k,track(v)if k==b'trak'else v)for k,v in boxes(p))if t==b'moov'else p)for t,p in parts)
def generate():
    files={};cases=[];bad=[]
    for n in [480,512]:
      for cfg in [1,7]:
        for dual in [False,True]:
          control,pcm,width,channels=eld_control(n,cfg,dual=dual,frame_count=12);stem=f'n{n}-c{cfg}'+(''if dual else'-x1');rate=48000 if dual else 24000
          files[stem+'.f32le']=bytes(n*channels*4//(1 if dual else 2))+pcm+pcm
          files[stem+'-control.mp4']=edited(control,n,od_replays=True)
          for form in ['snapshots','deltas','replace']:
            for unused in [False,True]:
              raw,_,refs=author(control,n,form,unused=unused);source=replay(raw,n,refs,form)
              for mode in ['indexed','fragmented','hybrid']:
                  name=f'{stem}-{form}'+('-unused'if unused else'')+'-'+mode;video=name+'.mp4'
                  files[video]=edited(source if mode=='indexed'else fragmented(source,n,mode=='hybrid'),n,od_replays=True)
                  cases.append(dict(name=name,video=video,control=stem+'-control.mp4',golden=stem+'.f32le',n=n,channels=channels,rate=rate,frames=12,width=width,mode=mode))
              if not unused:
                  video=f'{stem}-{form}-wrong-second.mp4';files[video]=edited(replay(raw,n,refs,form,True),n,od_replays=True)
                  bad.append(dict(video=video,n=n,width=width,rate=rate,error='missing simultaneous ESC access unit'))
    assert len(cases)==144 and len(bad)==24
    files['manifest.json']=(json.dumps(dict(cases=cases,malformed=bad,provenance='Own twelve-frame LD-SBR AU source shown twice, initial silence, two different OD source domains and object IDs. One-OD-tick CTS offset; indexed/fragmented/hybrid; absent/present inactive tails. Independent scalar/CLDFB PCM repeated directly. Wrong-second-context files require absent active AUs only during the second showing. No private media, foreign codec, process or network.'),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
