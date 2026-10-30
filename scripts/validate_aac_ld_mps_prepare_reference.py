"""Explicit physical/smoothing/interpolation comparison, outside normal tests.
Original float outcomes remain recorded; widened local energy functions are a
separate precision control, not code shipped or linked into FVid.
"""
import hashlib,json,re,struct,subprocess,sys,tempfile
from pathlib import Path
from zipfile import ZipFile
from extract_aac_ld_mps_prepare_facts import load
ROOT=Path(__file__).resolve().parents[1]
CORPUS=ROOT/'tests/fixtures/playback-errors/aac-ld-mps-prepare'
FRAMES=ROOT/'tests/fixtures/playback-errors/aac-ld-mps-frame'
def sha(b):return hashlib.sha256(b).hexdigest()
def widened(source):
    text=source.decode();patches=[]
    for m in re.finditer(r'static (?:float|void)\s+factor(?:Funct|CLD)\s*\(',text):
        start=m.start();at=text.index('{',m.end());depth=1;end=at+1
        while depth:
            depth+=(text[end]=='{')-(text[end]=='}');end+=1
        before=text[start:end];after=re.sub(r'\bfloat\b','double',before);patches.append((start,end,before,after))
    assert len(patches)==4
    for start,end,before,after in reversed(patches):text=text[:start]+after+text[end:]
    return text.encode(),[dict(original_sha256=sha(a.encode()),control_sha256=sha(b.encode()),declaration=a.split('{')[0].strip())for _,_,a,b in patches]
def run(archive):
    assert sys.byteorder=='little'
    facts,provenance=load(archive);assert facts==json.loads((CORPUS/'facts.json').read_text())
    runtime=(ROOT/'crates/fvid-media/src/owned_aac/aac_ld_mps_prepare_facts.rs').read_text()
    names={'dequantCLD':'CLD','dequantICC':'ICC','dequantCPC':'CPC','surroundGainTable':'SURROUND','lfeGainTable':'LFE','clipGainTable':'DOWNMIX','smgTimeTable':'SMOOTH_TIMES'}
    for name,values in facts.items():
        label=names.get(name,name);body=re.search(r'\b'+label+r': \[[^]]+\]\s*=\s*\[(.*?)\];',runtime,re.S).group(1)
        assert [float(x.strip())for x in body.split(',')if x.strip()]==values,name
    m=json.loads((CORPUS/'manifest.json').read_text());
    for name,digest in m['inputs'].items():assert sha((FRAMES/name).read_bytes())==digest
    fm=json.loads((FRAMES/'manifest.json').read_text());records=bytearray();expected=[]
    for row in m['scalar']:
        records.extend(struct.pack('<Iiid',1,row['index'],row['mode'],row['input']));expected.append((1,row))
    smooth_count=0
    for c in fm['cases']:
        from generate_aac_ld_mps_prepare_fixtures import smooth
        previous=dict(time=0,mask=[False]*c['bands'])
        for fi,f in enumerate(c['frames']):
            extend=f['slots'][-1]!=c['frame_slots']-1;controls,previous=smooth(f['smoothing'],previous,c['bands']);controls+=controls[-1:]if extend else[]
            records.extend(struct.pack('<5I',2,c['bands'],len(f['slots']),extend,fi==0))
            for s in f['smoothing']:
                records.extend(struct.pack('<4I',s['mode'],s['time']or 0,[1,2,5,23].index(s['stride'])if s['stride']else 0,len(s['groups'])));records.extend(struct.pack('<'+'I'*len(s['groups']),*s['groups']))
            expected.append((2,dict(controls=controls,name=c['name']+':'+str(fi))));smooth_count+=1
    for c in m['matrix']:
        for fi,f in enumerate(c['frames']):
            records.extend(struct.pack('<9I',3,c['bands'],*c['layout'],len(f['slots']),f['frame_slots'],f['residual_cutoff'],fi==0))
            for slot,s,raw in zip(f['slots'],f['controls'],f['raw']):
                records.extend(struct.pack('<2I',slot,s['time']));records.extend(struct.pack('<'+'I'*len(s['mask']),*s['mask']));records.extend(struct.pack('<'+'d'*len(raw),*raw))
            expected.append((3,f))
    reports=[];members={}
    with tempfile.TemporaryDirectory(prefix='fvid-physical-reference-')as tmp:
        work=Path(tmp)
        with ZipFile(archive)as z:
            for name in z.namelist():
                if name.startswith(('mp4spatialdec/src/','mp4spatialdec/include/','general/'))and name.endswith(('.c','.h')):
                    assert '..'not in Path(name).parts
                    data=z.read(name);dest=work/name;dest.parent.mkdir(parents=True,exist_ok=True);dest.write_bytes(data);members[name]=sha(data)
        src=work/'mp4spatialdec/src';original=(src/'sac_bitdec.c').read_bytes();control,patches=widened(original);(work/'libtsp.h').write_text('typedef struct AFILE AFILE;\n');(work/'records.bin').write_bytes(records)
        driver=ROOT/'scripts/support/aac_ld_mps_prepare_reference_probe.c';wrapper=ROOT/'scripts/support/aac_ld_mps_interp_reference_wrapper.c'
        for name,source in [('original-float',original),('energy-double-control',control)]:
            (work/'physical-reference.c').write_bytes(source);exe=work/'probe'
            args=['cc','-O2','-DLD_MPS','-ffunction-sections','-fdata-sections','-I',str(work),'-I',str(src),'-I',str(work/'mp4spatialdec/include'),str(driver),str(wrapper),str(src/'sac_smoothing.c')]
            if name!='original-float':args+=['-DPROBE_DOUBLE']
            args+=['-Wl,-dead_strip']if sys.platform=='darwin'else['-Wl,--gc-sections']
            build=subprocess.run(args+['-lm','-o',str(exe)],capture_output=True,timeout=60)
            if build.returncode:raise RuntimeError(build.stderr.decode())
            result=subprocess.run([str(exe),str(work/'records.bin')],capture_output=True,text=True,timeout=60)
            if result.returncode:raise RuntimeError((result.returncode,result.stderr,result.stdout[-1000:]))
            actual=[json.loads(line)for line in result.stdout.splitlines()];assert len(actual)==len(expected);nonfinite=[];indices=[];max_energy=0;max_matrix=0
            for i,((kind,want),got)in enumerate(zip(expected,actual)):
                assert got['type']==kind
                if kind==1:
                    if got['index']!=want['adjusted']:indices.append(dict(case=i,expected=want,actual=got))
                    for field in ['left','right']:
                        if got[field]is None:nonfinite.append(dict(case=i,field=field,expected=want,actual=got))
                        else:max_energy=max(max_energy,abs(got[field]-want[field]))
                elif kind==2:assert got['controls']==want['controls'],(want['name'],got,want)
                else:
                    for key in ['endpoints','output']:
                        assert len(got[key])==len(want[key])
                        for a,b in zip(got[key],want[key]):
                            assert len(a)==len(b)
                            for x,y in zip(a,b):max_matrix=max(max_matrix,abs(x-y))
            assert max_matrix<1e-6,max_matrix
            if name=='energy-double-control':assert not nonfinite and not indices and max_energy<1e-10,(len(nonfinite),len(indices),max_energy)
            reports.append(dict(control=name,energy_cases=len(m['scalar']),index_mismatches=indices,nonfinite_energy=nonfinite,max_finite_energy_abs_error=max_energy,smoothing_cases=smooth_count,matrix_cases=sum(len(c['frames'])for c in m['matrix']),max_matrix_abs_error=max_matrix,stdout_sha256=sha(result.stdout.encode())))
    report=dict(rejected_harness_setup=dict(case='tree0-res1-v0:first frame',cause='Initial probe omitted implicit bsSmoothControl=1 set by SpatialDecParseFrame.',actual_time=0,expected_time=256,correction='Set the implicit frame control before decodeAndMapFrameSmg; no reference source change.'),archive_sha256=provenance['archive_sha256'],reference_members=members,driver_sha256=sha(driver.read_bytes()),interpolation_wrapper_sha256=sha(wrapper.read_bytes()),input_sha256=sha(records),original_sha256=sha(original),control_sha256=sha(control),precision_control=patches,results=reports,scope='Physical scalar energy, smoothing controls and matrix temporal components only. Original float nonfinite outcomes retained. Decimal/Fraction oracles independently qualify the Rust f64 path. No spatial PCM/playback/seek acceptance or reference code in production.')
    (ROOT/'docs/validation/aac-ld-mps-prepare-reference-2026-10-30.json').write_text(json.dumps(report,indent=2)+'\n');print([(r['control'],len(r['index_mismatches']),len(r['nonfinite_energy']),r['max_finite_energy_abs_error'],r['max_matrix_abs_error'])for r in reports])
if __name__=='__main__':run(Path(sys.argv[1]))
