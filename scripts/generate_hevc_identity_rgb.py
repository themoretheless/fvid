#!/usr/bin/env python3
"""Identity RGB VUI on authored PCM; independent GBR/range/display sample oracles."""
import hashlib,json,subprocess
from generate_hevc_matroska_formats import D,boxes,child,unpack,movie
from hevc_fixture_mp4 import box
ROOT=D.parents[2]
def patch_private(private,stem,full):
    out=bytearray(private[:23]);at=23
    for _ in range(private[22]):
        tag=private[at];count=int.from_bytes(private[at+1:at+3],'big');at+=3;out+=bytes([tag])+count.to_bytes(2,'big')
        for index in range(count):
            length=int.from_bytes(private[at:at+2],'big');at+=2;nal=private[at:at+length];at+=length
            if tag&63==33:
                source=D/f'{stem}-{index}-source.sps';target=D/f'{stem}-{index}.sps';source.write_bytes(nal)
                subprocess.run([str(ROOT/'target/debug/examples/hevc_identity_fixture_sps'),str(source),'full' if full else 'limited',str(target)],check=True)
                nal=target.read_bytes()
            out+=len(nal).to_bytes(2,'big')+nal
    assert at==len(private)
    return bytes(out)
def replace_mp4(data,private):
    def update(body,path):
        result=b''
        for tag,value in boxes(body):
            if tag==path[0]:
                if len(path)==1:value=private
                elif tag==b'stsd':
                    entries=b''
                    for kind,entry in boxes(value[8:]):entries+=box(kind,entry[:78]+update(entry[78:],path[1:]))
                    value=value[:8]+entries
                else:value=update(value,path[1:])
            result+=box(tag,value)
        return result
    return update(data,[b'moov',b'trak',b'mdia',b'minf',b'stbl',b'stsd',b'hvcC'])
def display(values,w,h,crop,rotation):
    l,t,r,b=crop;rows=[[values[(y*w+x)*3:(y*w+x)*3+3] for x in range(l,w-r)] for y in range(t,h-b)]
    if rotation==90:rows=[list(row) for row in zip(*rows[::-1])]
    elif rotation==180:rows=[row[::-1] for row in rows[::-1]]
    elif rotation==270:rows=[list(row) for row in zip(*rows)][::-1]
    return [v for row in rows for pixel in row for v in pixel]
def main():
    cases=[]
    for depth,suffix in [(8,'reference-rext8'),(10,'full10-rext10'),(12,'full12-rext12'),(14,'deep-full-rext14'),(16,'deep-full-rext16')]:
        source=f'hevc-pcm-444-{suffix}.mp4';original,packets,w,h,rate=unpack(D/source);unit=1 if depth==8 else 2;gold=(D/source.replace('.mp4','.yuv')).read_bytes()
        for full in [True,False]:
            stem=f'hevc-identity-rgb-{depth}-'+('full' if full else 'limited');private=patch_private(original,stem,full)
            config=stem+'.hvcc';(D/config).write_bytes(private);mp4=stem+'.mp4';(D/mp4).write_bytes(replace_mp4((D/source).read_bytes(),private))
            inputs=[]
            for i,p in enumerate(packets):
                raw=gold[i*w*h*3*unit:(i+1)*w*h*3*unit]
                values=list(raw) if unit==1 else [int.from_bytes(raw[j:j+2],'little') for j in range(0,len(raw),2)]
                n=w*h;rgb=[v for i in range(n) for v in [values[2*n+i],values[i],values[n+i]]]
                file=f'{stem}-{i}.packet';(D/file).write_bytes(p);inputs.append(dict(packet=file,rgb=rgb))
            for rotation in [0,90,180,270]:
                for crop in [[0,0,0,0],[2,4,6,8]]:
                    name=stem+f'-r{rotation}-crop{crop[0]}';mkv=name+'.mkv';data=movie(private,packets,w,h,rate,rotation,crop);(D/mkv).write_bytes(data);frames=[]
                    for i,f in enumerate(inputs):
                        serialize=lambda v:bytes(v) if unit==1 else b''.join(n.to_bytes(2,'little') for n in v)
                        def rgb8(v):
                            if full:den=(1<<depth)-1;offset=0
                            else:den=219*(1<<(depth-8));offset=16*(1<<(depth-8))
                            return bytes(min(255,max(0,((n-offset)*255+den//2)//den)) for n in v)
                        turned=display(f['rgb'],w,h,crop,rotation);paths=dict(packet=f['packet'])
                        for key,content in [('rgb',serialize(f['rgb'])),('rgb8',rgb8(f['rgb'])),('display',serialize(turned)),('display8',rgb8(turned))]:
                            file=f'{name}-{i}.{key}';(D/file).write_bytes(content);paths[key]=file
                        frames.append(paths)
                    cases.append(dict(file=mkv,mp4=mp4,configuration=config,size=[w,h],depth=depth,full_range=full,rotation=rotation,crop=crop,rate=rate,frames=frames,sha256=hashlib.sha256(data).hexdigest()))
    (D/'hevc-identity-rgb.json').write_text(json.dumps(dict(cases=cases,provenance='Authored PCM samples relabelled G/B/R by identity matrix VUI, exact R/G/B packing and integer full/limited RGB normalization. Offline owned SPS helper proves all non-VUI parameters unchanged; it does not decode pixel oracles. No private media, external codec or network.'),indent=2)+'\n')
if __name__=='__main__':main()
