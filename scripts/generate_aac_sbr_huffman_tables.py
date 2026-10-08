#!/usr/bin/env python3
"""Explicit offline extraction of normative SBR codewords, not decoder source.
Input: locally saved UTF-8 GOST R53556.4-2013 HTML from rags.ru gost_50462.
Ordinary tests consume the checked-in tables and never run this generator.
"""
import argparse, html, json, re
from pathlib import Path
p=argparse.ArgumentParser();p.add_argument('standard_html',type=Path);p.add_argument('iso_draft_text',type=Path);a=p.parse_args()
s=a.standard_html.read_text();s=s[s.index('id="Par44422"'):]
labels=[(0,'t_huffman_env_1_5dB')]
for m in re.finditer('<div[^>]*>(.*?)</div>',s,re.S):
 t=re.sub(r'\s+','',html.unescape(re.sub('<[^>]+>','',m[1])))
 if re.fullmatch('[tf]_huffman_(env|noise).*',t,re.I):labels.append((m.start(),t))
 if len(labels)==10:break
assert len(labels)==10
counts=[121,121,49,49,63,63,25,25,63,25]
tables=[]
for i,(start,name) in enumerate(labels):
 end=labels[i+1][0] if i<9 else s.index('Таблица А.89',start)
 rows=[]
 for r in re.findall('<tr[^>]*>(.*?)</tr>',s[start:end],re.S):
  cells=[re.sub(r'\s+','',html.unescape(re.sub('<[^>]+>','',c))) for c in re.findall('<td[^>]*>(.*?)</td>',r,re.S)]
  if len(cells)==3 and cells[0].isdigit() and all(c.startswith('0x') for c in cells[1:]):
   rows.append([int(cells[0]),int(cells[1],16),int(cells[2],16)])
 # Translation has sparse index numbering in the three 25-symbol books.
 if i in (6,7,9):
  assert [r[0] for r in rows]==list(range(13))+list(range(32,44))
  rows=[[j,n,c] for j,(_,n,c) in enumerate(rows)]
 # Translation typos verified against ISO w4611 Tables1.A.3-1.A.11.
 corrections={1:{31:(19,0x3ffe1,19,0x7ffe1),32:(18,0x3eee0,18,0x3ffe0),101:(20,0xfffe0,20,0xffff0),103:(20,0xfffe1,20,0xffff1)},
  2:{38:(16,0xffea,16,0xfffa)},3:{1:(18,0x3efe3,18,0x3ffe3)},8:{26:(13,0x7f2,11,0x7f2)}}
 for j,(oldn,oldc,newn,newc) in corrections.get(i,{}).items():
  assert rows[j]==[j,oldn,oldc]
  rows[j]=[j,newn,newc]
 assert len(rows)==counts[i],(name,len(rows))
 assert [r[0] for r in rows]==list(range(counts[i]))
 maximum=max(r[1] for r in rows)
 assert sum(1<<(maximum-r[1]) for r in rows)==1<<maximum
 for j,(_,n,code) in enumerate(rows):
  assert 1<=n<=20 and code<1<<n
  for other_index,m,other in rows[j+1:]:
   assert not (code==other>>(m-n) if n<=m else other==code>>(n-m)), (name,j,n,hex(code),other_index,m,hex(other))
 tables.append({'name':name,'offset':counts[i]//2,'rows':rows})
# Independent ISO text layout: two columns per line rather than HTML rows.
d=a.iso_draft_text.read_text();start=d.index('Table 1.A.1');stop=d.index('Table 1.A.12 Coefficients')
d=d[start:stop];heads=list(re.finditer(r'Table 1\.A\.(?:[3-9]|10|11)\b',d))
bounds=[0]+[m.start() for m in heads]+[len(d)]
assert len(bounds)==11
for i in range(10):
 block=d[bounds[i]:bounds[i+1]]
 ref={int(n):(int(length,16),int(code,16)) for n,length,code in re.findall(r'(\d+)\s+0x([0-9A-Fa-f]+)\s+0x([0-9A-Fa-f]+)',block)}
 assert len(ref)==counts[i],(i,len(ref))
 assert all(ref[j]==(n,c) for j,n,c in tables[i]['rows']),(i,'ISO codeword mismatch')
root=Path(__file__).resolve().parents[1]
(root/'tests/fixtures/playback-errors/aac-sbr-huffman-codewords.json').write_text(json.dumps(tables,separators=(',',':'))+'\n')
lines=['// Normative protocol codewords, GOST R53556.4-2013 tables A.79-A.88.', '// Generated explicitly by scripts/generate_aac_sbr_huffman_tables.py.']
for i,t in enumerate(tables):
 lines.append(f'pub(super) const BOOK_{i}: &[(u8, u32)] = &[')
 lines.extend(f'    ({n}, 0x{code:x}),' for _,n,code in t['rows']);lines.append('];')
(root/'crates/fvid-media/src/owned_aac/aac_sbr_huffman_tables.rs').write_text('\n'.join(lines)+'\n')
print('10 complete prefix-free tables; symbols:',sum(counts))
