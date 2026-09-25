#!/usr/bin/env python3
"""Translate normative AV1 CDF tables (no decoder implementation) to Rust.
Usage: python3 scripts/generate_av1_tables.py /tmp/fvid-av1-tables.md
Source: https://raw.githubusercontent.com/AOMediaCodec/av1-spec/master/10.additional.tables.md
"""
import ast, hashlib, pathlib, re, sys
source = pathlib.Path(sys.argv[1]).read_text()
out = ['//! Generated normative AV1 default CDF tables; see scripts/generate_av1_tables.py.',
       '//! Source SHA-256: ' + hashlib.sha256(source.encode()).hexdigest(),
       '#![allow(dead_code)]',
       'use crate::{Result, invalid};',
       '/// Table ids, in `Cdfs::new` insertion order; call sites name them instead of hashing a string.',
       '<<IDS>>',
       '#[derive(Clone)] pub struct Table { pub shape: &\'static [usize], pub values: Vec<u16> }',
       '#[derive(Clone)] pub struct Cdfs { tables: Vec<Table> }',
       'impl Cdfs {',
       '''fn insert(&mut self, table: Table) { self.tables.push(table); }''',
       'pub fn new(q: u8) -> Self {',
       'let qi = if q <= 20 {0} else if q <= 60 {1} else if q <= 120 {2} else {3};',
       'let mut cdfs = Self { tables: Vec::with_capacity(128) };']
ids=[]
constants=[]
for match in re.finditer(r'^(Default_\w+_Cdf)\s*((?:\[[^\]]+\]\s*)+)\s*=\s*\{', source, re.M):
 name,decl=match.group(1,2); start=match.end()-1; depth=1; end=start+1
 while depth:
  depth += (source[end]=='{')-(source[end]=='}'); end+=1
 raw=re.sub(r'/\*.*?\*/|//[^\n]*','',source[start:end],flags=re.S)
 raw=re.sub(r'(\d+)\s*\*\s*(\d+)', lambda m: str(int(m[1])*int(m[2])), raw)
 data=ast.literal_eval(raw.replace('{','[').replace('}',']'))
 def shape(v):
  if isinstance(v,int): return []
  inner=shape(v[0]); assert all(shape(x)==inner for x in v), name
  return [len(v)]+inner
 dims=shape(data)
 def flatten(v):
  return [v] if isinstance(v,int) else [n for x in v for n in flatten(x)]
 flat=flatten(data); symbol=name.upper(); key=name.removeprefix('Default_').removesuffix('_Cdf')
 constants.append('const '+symbol+': &[u16] = &'+repr(flat)+';')
 if 'COEFF_CDF_Q_CTXS' in decl:
  stride=len(flat)//dims[0]; dims=dims[1:]
  init=f'{symbol}[qi*{stride}..(qi+1)*{stride}].to_vec()'
 else: init=f'{symbol}.to_vec()'
 if key.startswith('Mv_'):
  if key in ['Mv_Class0_Hp','Mv_Hp','Mv_Sign','Mv_Bit','Mv_Class0_Bit']:
   dims=[2,2]+dims; init=f'{init}.repeat(4)'
  else:
   dims=[2]+dims; init=f'{init}.repeat(2)'
 ids.append(f'pub const {key.upper()}: usize = {len(ids)};')
 out.append(f'cdfs.insert(Table {{ shape: &{dims!r}, values: {init} }});')
out[out.index('<<IDS>>')] = '\n'.join(ids)
out += ['cdfs', '}', '''pub fn reset_counts(&mut self) { for table in &mut self.tables { let n=*table.shape.last().unwrap(); for cdf in table.values.chunks_exact_mut(n) { cdf[n-1]=0; } } }''', '''pub fn get(&mut self, id: usize, indices: &[usize]) -> Result<&mut [u16]> {
let table = self.tables.get_mut(id).ok_or_else(|| invalid("unknown AV1 CDF table"))?;
if indices.len()+1 != table.shape.len() { return Err(invalid("invalid AV1 CDF rank")); }
let mut offset = 0;
for (i, index) in indices.iter().enumerate() {
if *index >= table.shape[i] { return Err(invalid("AV1 CDF index outside table")); }
offset = offset * table.shape[i] + index;
}
let n = *table.shape.last().unwrap();
Ok(&mut table.values[offset*n..(offset+1)*n])
}''','}']+constants
pathlib.Path('src/codec/av1_cdfs.rs').write_text('\n'.join(out)+'\n')
print(len(constants), 'tables generated')
