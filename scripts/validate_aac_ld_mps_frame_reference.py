"""Explicit frame syntax comparison with retained ISO defect exclusions/controls.
No reference code, compiler or process is used by ordinary Rust tests.
"""
import hashlib,json,math,re,struct,subprocess,sys,tempfile
from pathlib import Path
from zipfile import ZipFile
from generate_aac_ld_mps_config_fixtures import TREES
from extract_aac_ld_mps_huffman_facts import tree, words
ROOT=Path(__file__).resolve().parents[1]
CORPUS=ROOT/'tests/fixtures/playback-errors/aac-ld-mps-frame'
def sha(b):return hashlib.sha256(b).hexdigest()
def descriptor(r,c,separate_namespace=False):
    parts=r['id'].split(':');name=parts[0];high=component=0;index=int(parts[1])if parts[1]!='all'else 0;n=len(c['regions']);ott=sum(x['id'].startswith('ott-cld:')for x in c['regions']);nt=int(c['tree']in [2,3,4])
    if name=='ott-cld':category=0;param=index
    elif name=='ott-icc':category=1;param=index
    elif name.startswith('ttt-'):
        high=int(parts[2]);component=int(parts[3])if len(parts)>3 else 0;category={'ttt-cpc':2,'ttt-cld':3,'ttt-icc':4}[name];param=ott+4*index+2*high+component
    elif name=='downmix':category=5;param=ott+4*nt+index
    elif name=='arb':category=0;index=ott+int(parts[2]);param=index+(4*nt+TREES[c['tree']][0] if separate_namespace else 0)
    else:raise AssertionError(name)
    return [category,index,high,component,r['start'],r['start']+len(r['values'][0]),{'CLD':0,'ICC':1,'CPC':2}[r['kind']],0,param]
def run(archive):
    assert sys.byteorder == 'little', 'Research probe record format requires little-endian host'
    known=json.loads((ROOT/'docs/validation/aac-ld-qmf-source-2026-10-30.json').read_text());digest=sha(archive.read_bytes());assert digest==known['archive_sha256']
    m=json.loads((CORPUS/'manifest.json').read_text());configs=(CORPUS/'configs.bin').read_bytes();packets=(CORPUS/'packets.bin').read_bytes();members={};reports=[]
    with tempfile.TemporaryDirectory(prefix='fvid-frame-reference-')as tmp:
        work=Path(tmp)
        with ZipFile(archive)as z:
            for name in z.namelist():
                if name.startswith(('mp4spatialdec/src/','mp4spatialdec/include/','general/'))and name.endswith(('.c','.h')):
                    assert '..'not in Path(name).parts
                    data=z.read(name);p=work/name;p.parent.mkdir(parents=True,exist_ok=True);p.write_bytes(data);members[name]=sha(data)
        src=work/'mp4spatialdec/src'
        numeric=words(tree((src/'sac_nodes_reshape.c').read_text(),'huffReshapeNodes')[0])
        assert numeric==json.loads((CORPUS/'reshape-codewords.json').read_text())
        facts=(ROOT/'crates/fvid-media/src/owned_aac/aac_ld_mps_frame_facts.rs').read_text()
        assert numeric==[list(map(int,r))for r in re.findall(r'\((\d+),\s*(\d+),\s*(-?\d+)\)',facts)]
        original=(src/'sac_bitdec.c').read_bytes();control=original;changes=[]
        for before,after in [(b'int i1, i2, x1, xi, x2;',b'int i1=0, i2, x1, xi, x2;'),(b'static int tempShapeChanTable[][7] = { {5, 5, 4, 6, 6, 4, 4}, {5, 5, 5, 7, 7, 4, 4} };',b'static int tempShapeChanTable[][8] = { {5, 5, 4, 6, 6, 4, 4, 2}, {5, 5, 5, 7, 7, 4, 4, 2} };')]:
            assert control.count(before)==1;control=control.replace(before,after);changes.append(dict(before=before.decode(),after=after.decode()))
        before=b't_CLD, self->numOttBoxes+i, self->numOttBoxes+i, 0, self->bsConfig.bsOttBandsAT[i]'
        after=b't_CLD, self->numOttBoxes+i, self->numOttBoxes+4*self->numTttBoxes+self->numInputChannels+i, 0, self->bsConfig.bsOttBandsAT[i]'
        assert control.count(before)==1;control=control.replace(before,after);changes.append(dict(before=before.decode(),after=after.decode()))
        (work/'libtsp.h').write_text('typedef struct AFILE AFILE;\n');driver=ROOT/'scripts/support/aac_ld_mps_frame_reference_probe.c'
        for version in ['original-safe-paths','initialized-anchor-tree7-table-and-namespace-control']:
            data=bytearray();expected=[];names=[];excluded=[]
            for c in m['cases']:
                selected=[]
                for fi,f in enumerate(c['frames']):
                    if f['opaque_residual'] or version.startswith('original')and(f['leading_interpolation']or c['tree']==7 and f['temporal']['kind']!='disabled'):
                        excluded.append(c['name']+':'+str(fi));continue
                    selected.append((fi,f))
                if not selected:continue
                cfg=configs[c['config_offset']:c['config_offset']+c['config_bytes']];data.extend(struct.pack('<2I',len(cfg),len(selected)));data.extend(cfg)
                byid={r['id']:r for r in c['regions']+[r for rs in c['extension_regions']for r in rs]}
                for fi,f in selected:
                    rs=f['parameters']+[r for ext in f['extensions']for r in ext['parameters']];p=packets[f['offset']:f['offset']+f['bytes']]+b'\x5a\xa5';data.extend(struct.pack('<2I',len(p),len(rs)));data.extend(p)
                    for r in rs:
                        d=descriptor(r,c,not version.startswith('original'));d[7]=byid[r['id']]['default'];data.extend(struct.pack('<9I',*d))
                    expected.append((c,f,rs));names.append(c['name']+':'+str(fi))
            (work/'frame-reference.c').write_bytes(original if version.startswith('original')else control);(work/'records.bin').write_bytes(data);exe=work/'probe'
            args=['cc','-O2','-DLD_MPS','-ffunction-sections','-fdata-sections','-I',str(work),'-I',str(src),'-I',str(work/'mp4spatialdec/include'),str(driver),str(src/'sac_nlc_dec.c'),str(src/'sac_bitinput.c')]+[str(src/f'sac_nodes_{k}.c')for k in ['cld','icc','cpc','pt0','reshape']]
            args+=['-Wl,-dead_strip']if sys.platform=='darwin'else['-Wl,--gc-sections']
            build=subprocess.run(args+['-lm','-o',str(exe)],capture_output=True,timeout=60)
            if build.returncode:raise RuntimeError(build.stderr.decode())
            result=subprocess.run([str(exe),str(work/'records.bin')],capture_output=True,text=True,timeout=30)
            if result.returncode:raise RuntimeError((result.returncode,result.stderr,result.stdout[-1000:]))
            actual=[json.loads(line)for line in result.stdout.splitlines()];assert len(actual)==len(expected);mismatches=[]
            for name,(c,f,rs),a in zip(names,expected,actual):
                assert a['bits']==f['bits']and a['slots']==f['slots']and bool(a['independent'])==f['independent'],(name,'framing',a,f)
                if a['regions']!=[r['values']for r in rs]:
                    assert version.startswith('original')and c['variant']==5 and f is c['frames'][0],(name,'unexpected regions',a['regions'],rs)
                    mismatches.append(dict(name=name,expected=[r['values']for r in rs],actual=a['regions'],cause='Arbitrary-tree lossless metadata aliases downmix/TTT metadata indices.'))
                assert a['smoothing']==f['smoothing'],(name,'smoothing',a['smoothing'],f['smoothing'])
                t=f['temporal'];count=[[5,5,4,6,6,4,4,2],[5,5,5,7,7,4,4,2]][c['variant']%3-1][c['tree']]if c['variant']%3 else 0
                assert len(a['shape'])==count
                for ch,value in enumerate(a['shape']):
                    enabled=t['enabled'][ch]if t['kind']=='stp'else t['envelopes'][ch]is not None if t['kind']=='ges'else False
                    assert bool(value['enabled'])==enabled,(name,'shape enabled',ch,value,t)
                    want=[2**(x/2-1)for x in t['envelopes'][ch]]if t['kind']=='ges'and enabled else[]
                    assert len(value['envelope'])==len(want)and all(abs(x-y)<1e-6 for x,y in zip(value['envelope'],want)),(name,'shape values',value,want)
            reports.append(dict(control=version,checked=len(actual),matched=len(actual)-len(mismatches),mismatches=mismatches,excluded=excluded,input_sha256=sha(data),stdout_sha256=sha(result.stdout.encode()),names_sha256=sha(json.dumps(names).encode())))
    report=dict(later_source=json.loads((ROOT/'docs/validation/aac-ld-mps-frame-source-2026-10-30.json').read_text()),archive_sha256=digest,reference_members=members,driver_sha256=sha(driver.read_bytes()),original_sha256=sha(original),control_sha256=sha(control),changes=changes,results=reports,scope='Frame syntax/fine parameter indices and raw smoothing/STP/GES only. Residual AAC payloads excluded; no spatial PCM or movie/playback/seek acceptance. Original undefined interpolation/tree7 shape accesses are not executed.')
    (ROOT/'docs/validation/aac-ld-mps-frame-reference-2026-10-30.json').write_text(json.dumps(report,indent=2)+'\n');print([(r['control'],r['matched'],len(r['excluded']))for r in reports])
if __name__=='__main__':run(Path(sys.argv[1]))
