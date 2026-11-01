#!/usr/bin/env python3
"""Explicit authored arbitrary-tree inputs. --reference freezes unmodified ISO C output."""
import json,struct,hashlib,sys,subprocess,tempfile,zipfile,copy
from pathlib import Path
from generate_he_aac_packet_fixtures import field,packed,video_fixture
from generate_aac_ld_mps_inband_fixtures import raw,header
from generate_aac_ld_mps_transport_fixtures import envelope,length
from generate_aac_ld_mps_entropy_fixtures import pcm
from generate_aac_eld_fixtures import mkv
ROOT=Path(__file__).resolve().parents[1];FIX=ROOT/'tests/fixtures/playback-errors';OUT=FIX/'aac-ld-mps-arbitrary-tree'
def sha(b):return hashlib.sha256(b).hexdigest()
def authored():
 OUT.mkdir(exist_ok=True);facts=json.loads((FIX/'aac-ld-mps-prepare/facts.json').read_text());blob=bytearray();cases=[]
 for bank in [32,64,128]:
  for bi,bands in enumerate([23,15,12,9,7,5,4]):
   for variant in range(2):
    tree=(bi+variant+bank//32)%8;inputs=[6,6,6,8,8,8,8,2][tree];pattern=(bi+variant)%4
    nodes=[[False]for _ in range(inputs)];nodes[0]=[[True,False,False],[True,True,False,False,False],[True,True,False,False,True,False,False],[True,False,False]][pattern]
    if pattern==3:nodes[-1]=[True,False,False]
    boxes=sum(sum(row)for row in nodes);outputs=inputs+boxes;positions=list(reversed(range(outputs)))if variant else list(range(outputs));mapping=facts[f'QMF{bands}'][:bank]
    surround=facts['surroundGainTable'][(bi+1)%5];lfe=facts['lfeGainTable'][(bi+2)%5];fixed=[1.0]*inputs
    if tree==0:fixed[3]=lfe;fixed[4]=fixed[5]=surround
    elif tree in [1,2]:fixed[5]=lfe;fixed[1]=fixed[3]=surround
    elif tree!=7:
     fixed[7]=lfe
     for c in [1,2,4,5]:fixed[c]=surround
    # Freeze float inputs, including gains, exactly as the C interface receives them.
    f32=lambda x:struct.unpack('<f',struct.pack('<f',x))[0];fixed=list(map(f32,fixed));surround=f32(surround);lfe=f32(lfe);blocks=[]
    for frame,slots in enumerate([1,3,5]):
     ends=[slots-1]if slots==1 else[0,slots-1];cld=[[[facts['dequantCLD'][(frame*11+s*7+b*5+p*3+bi)%31]for p in range(bands)]for b in range(boxes)]for s in range(len(ends))]
     values=[((i*29+frame*13+bi*17+variant*7)%257-128)/32768 for i in range(slots*inputs*bank*2)];at=len(blob);blob.extend(struct.pack('<'+'f'*len(values),*values));blocks.append(dict(slots=slots,endpoints=ends,cld=cld,offset=at))
    cases.append(dict(tree=tree,bank=bank,bands=bands,inputs=inputs,outputs=outputs,nodes=nodes,positions=positions,cutoffs=[bands]*boxes,map=mapping,fixed=fixed,surround=surround,lfe=lfe,blocks=blocks))
 permutations=copy.deepcopy(cases)
 for c in permutations:
  initial=c['positions'];count=len(initial)
  for i,b in enumerate(c['blocks']):
   b['positions']=[initial,list(reversed(initial)),initial[1:]+initial[:1]][i]
   if b['slots']>1:b['endpoints']=[b['slots']//2,b['slots']-1]
 (OUT/'component-input.f32le').write_bytes(blob);(OUT/'component.json').write_text(json.dumps(dict(cases=cases,permutations=permutations,scope='Full-band post-shaping component probes; movie acceptance is recorded separately.'),separators=(',',':'))+'\n')
 # A three-AU mono-core/stereo-base programme with one extra OTT leaf.
 parent=FIX/'aac-ld-mps-downscaled';m=json.loads((parent/'manifest.json').read_text());base=next(c for c in m['cases']if c['tree']==7 and c['n']==512 and c['rate']==24000 and c['profile']==39);source=(parent/'packets.bin').read_bytes();core=next(c for c in m['controls']if c['golden']==base['core_gold']);old_ssc=bytes.fromhex(base['ssc']);data=packed('1000'+'00'+''.join(field(p,5)for p in [0,1,2]));ssc=old_ssc+packed(field(2,4)+length(len(data)))+data
 asc_bits=raw(bytes.fromhex(base['asc']));marker=field(2,4)+length(len(old_ssc))+raw(old_ssc)+'000000';at=asc_bits.rfind(marker);assert at>=0;asc=packed(asc_bits[:at]+field(2,4)+length(len(ssc))+raw(ssc)+'000000');packets=[];rows=[];movie=bytearray();refs=[];extra=[];plain_packets=[];plain_rows=[];plain_blob=bytearray()
 for i,r in enumerate(core['frames'][:3]):
  prefix=raw(source[r['offset']:r['offset']+r['bytes']])[:r['core_bits']];levels=[(i*7+b*3)%31 for b in range(23)];extra.append([facts['dequantCLD'][v]for v in levels]);plain=packed(prefix+envelope(9,1 if i==0 else 0,3,header(old_ssc,bytes.fromhex(base['reference'][i]))if i==0 else bytes.fromhex(base['reference'][i]),True));plain_rows.append(dict(offset=len(plain_blob),bytes=len(plain)));plain_blob.extend(plain);plain_packets.append(plain);child=packed('1111'+'10001'+pcm([levels[b]for b in range(23)for ps in range(2)],31));spatial=bytes.fromhex(base['reference'][i])+bytes([len(child)])+child;refs.append(spatial.hex());packet=packed(prefix+envelope(9,1 if i==0 else 0,3,header(ssc,spatial)if i==0 else spatial,True));rows.append(dict(offset=len(movie),bytes=len(packet)));movie.extend(packet);packets.append(packet)
 case=dict(base,name='tree7-extra-ott',channels=3,asc=asc.hex(),ssc=ssc.hex(),frames=rows,reference=refs,samples=768,video='tree7-extra-ott.mp4',file='tree7-extra-ott.mkv',base_ssc=base['ssc'],base_reference=base['reference'][:3],extra_cld=extra,core_gold='core-control.f32le')
 case.pop('golden',None);(OUT/'tree7-extra-ott.ssc').write_bytes(ssc);(OUT/'tree7-extra-ott-0.spatial').write_bytes(bytes.fromhex(refs[0]))
 (OUT/'core-control.f32le').write_bytes((parent/base['core_gold']).read_bytes()[:768*4]);control=dict(base,name='tree7-standard-control',frames=plain_rows,samples=768,video='tree7-standard-control.mp4',file='tree7-standard-control.mkv');control.pop('golden',None);control['core_gold']='core-control.f32le';control['reference']=base['reference'][:3];(OUT/control['video']).write_bytes(video_fixture([control],plain_blob,2,write=False)['data']);(OUT/control['file']).write_bytes(mkv(bytes.fromhex(base['asc']),plain_packets,base['rate'],base['out_n'],2))
 (OUT/case['video']).write_bytes(video_fixture([case],movie,3,write=False)['data']);(OUT/case['file']).write_bytes(mkv(asc,packets,base['rate'],base['out_n'],3));(OUT/'packets.bin').write_bytes(movie);(OUT/'manifest.json').write_text(json.dumps(dict(cases=[case],controls=[control],errors=[],scope='Full-band additional OTT playback acceptance. Own authored parameter sets and independent saved core PCM; no private/ISO media.'),separators=(',',':'))+'\n');print('Authored',len(cases),'component programmes and one playback acceptance')
def reference(archive):
 m=json.loads((OUT/'component.json').read_text());samples=(OUT/'component-input.f32le').read_bytes();wire=bytearray(struct.pack('<I',len(m['cases'])+len(m['permutations'])));expected_offset=0;cases=[]
 for c in m['cases']+m['permutations']:
  boxes=len(c['cutoffs']);wire.extend(struct.pack('<7I',c['tree'],c['bank'],c['inputs'],c['outputs'],c['bands'],boxes,len(c['blocks'])))
  for row in c['nodes']:wire.extend(struct.pack('<I',len(row))+struct.pack('<'+'I'*len(row),*row))
  wire.extend(struct.pack('<'+'I'*c['outputs'],*c['positions']));wire.extend(struct.pack('<'+'I'*c['bank'],*c['map']));wire.extend(struct.pack('<2f',c['surround'],c['lfe']));blocks=[]
  for b in c['blocks']:
   wire.extend(struct.pack('<'+'I'*c['outputs'],*b.get('positions',c['positions'])));wire.extend(struct.pack('<2I',b['slots'],len(b['endpoints']))+struct.pack('<'+'I'*len(b['endpoints']),*b['endpoints']));vals=[v for s in b['cld']for row in s for v in row];wire.extend(struct.pack('<'+'f'*len(vals),*vals));n=b['slots']*c['inputs']*c['bank']*8;wire.extend(samples[b['offset']:b['offset']+n]);blocks.append(dict(offset=expected_offset));expected_offset+=b['slots']*c['outputs']*c['bank']*8
  cases.append(dict(blocks=blocks))
 with tempfile.TemporaryDirectory(prefix='fvid-arbitrary-reference-')as tmp:
  work=Path(tmp);members={}
  with zipfile.ZipFile(archive)as z:
   for n in z.namelist():
    if n.startswith(('mp4spatialdec/src/','mp4spatialdec/include/','general/'))and n.endswith(('.c','.h')):
     assert '..'not in Path(n).parts;data=z.read(n);p=work/n;p.parent.mkdir(parents=True,exist_ok=True);p.write_bytes(data);members[n]=sha(data)
  (work/'libtsp.h').write_text('typedef struct AFILE AFILE;\n');src=work/'mp4spatialdec/src';driver=ROOT/'scripts/support/aac_ld_mps_arbitrary_tree_reference_probe.c';exe=work/'probe';args=['cc','-O2','-DLD_MPS','-I',str(work),'-I',str(src),'-I',str(work/'mp4spatialdec/include'),str(driver),str(src/'sac_arbitraryTree.c'),'-lm','-o',str(exe)];subprocess.run(args,check=True,capture_output=True,timeout=60);(work/'input.bin').write_bytes(wire);subprocess.run([str(exe),str(work/'input.bin'),str(work/'output.bin')],check=True,timeout=60);expected=(work/'output.bin').read_bytes()
 assert len(expected)==expected_offset;(OUT/'component-expected.f32le').write_bytes(expected);(OUT/'component-oracle.json').write_text(json.dumps(dict(cases=cases,scope='Unmodified ISO arbitraryTreeProcess C; component only.'),separators=(',',':'))+'\n');report=dict(archive_sha256=sha(Path(archive).read_bytes()),algorithm_sha256=members['mp4spatialdec/src/sac_arbitraryTree.c'],initial_gain_source_sha256=members['mp4spatialdec/src/sac_dec.c'],driver_sha256=sha(driver.read_bytes()),generator_sha256=sha(Path(__file__).read_bytes()),input_sha256=sha(wire),expected_sha256=sha(expected),programmes=len(cases),static_programmes=len(m['cases']),position_change_programmes=len(m['permutations']),scope='Explicit reference-only C executable, not production or ordinary tests. Full-band splits and dense output permutation only. Original initialization prevGainAT=1.0.');(ROOT/'docs/validation/aac-ld-mps-arbitrary-tree-reference-2026-11-01.json').write_text(json.dumps(report,indent=2)+'\n');print('Reference bytes:',len(expected))
if __name__=='__main__':
 authored()
 if len(sys.argv)>1:reference(sys.argv[1])
