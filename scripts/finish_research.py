#!/usr/bin/env python3
import collections,csv,datetime,hashlib,json,pathlib,re
ROOT=pathlib.Path(__file__).resolve().parents[1]
catalog=json.load(open(ROOT/'research/catalog.json'))
# Do not inflate counts with a renamed project, packaging, or a non-media ISA library.
extra_exclude={'rust-av/rav1e':'same encoder lineage as xiph/rav1e','ShiftMediaProject/x265':'build mirror, not a distinct encoder','FFmpeg/nv-codec-headers':'headers, not an independent engine','ashellunts/ffmpeg-to-webrtc':'sample integration','icedland/iced':'CPU instruction toolkit, outside media scope'}
names={r['full_name'].lower() for r in catalog}
for r in json.load(open(ROOT/'research/extra-index.json')):
 if 'full_name' not in r or r['full_name'] in extra_exclude or r['full_name'].lower() in names:continue
 names.add(r['full_name'].lower())
 entry={k:r.get(k) for k in ['full_name','html_url','description','language','stargazers_count','archived','pushed_at','default_branch']}
 entry.update(candidate_index=None,queries=['targeted_primary_projects'],license=(r.get('license') or {}).get('spdx_id','UNKNOWN'),category='targeted_library_framework',review_level='metadata_screened',evidence='research/extra-index.json',inclusion_basis=r.get('description') or r['full_name'])
 catalog.append(entry)
idx={r['repository'].lower():r for r in json.load(open(ROOT/'research/readme-index.json'))}
sources=json.load(open(ROOT/'research/source-index.json'))
source_names={r['repository'].lower() for r in sources if 'error' not in r}
signals={'memory':r'zero.copy|buffer pool|memory pool|allocat|bounded|memory.efficient','scheduling':r'backpressure|schedul|parallel|thread|queue|flow.control','gpu':r'gpu|videotoolbox|vulkan|vaapi|nvenc|metal|webcodecs','time':r'timestamp|timebase|time.base|synchroni|latency','quality':r'vmaf|psnr|ssim|bit.exact|conformance|fuzz','modularity':r'plugin|modular|feature.flag|trait|backend'}
checks={'memory':'Проверить владение, лимиты памяти и жизненный цикл буферов','scheduling':'Проверить очереди, ожидание и бюджет параллелизма','gpu':'Проверить передачу GPU-поверхностей и синхронизацию','time':'Проверить временные шкалы, задержку и discontinuity','quality':'Проверить корпус, качество и воспроизводимость метрик','modularity':'Проверить границы модулей и контракт расширений'}
for r in catalog:
 readme=idx.get(r['full_name'].lower());r['readme_status']=readme.get('status') if readme else 'not_collected';r['readme_sha256']=readme.get('sha256','') if readme else '';r['readme_blob_sha']=readme.get('blob_sha','') if readme else '';r['source_files_reviewed']=r['full_name'].lower() in source_names
 r['readme_signals']=[];r['signal_evidence']=[]
 if readme and readme['status']=='ok':
  path=ROOT/'research/sources'/readme['repository'].replace('/','__')/'README.txt';lines=path.read_text(errors='replace').splitlines()
  for key,pattern in signals.items():
   hit=next(((i+1,l[:350]) for i,l in enumerate(lines) if re.search(pattern,l,re.I)),None)
   if hit:r['readme_signals'].append(key);r['signal_evidence'].append({'signal':key,'line':hit[0],'text':hit[1],'path':str(path.relative_to(ROOT))})
 r['review_level']='selected_source_excerpts' if r['source_files_reviewed'] else 'metadata_manual_readme_automated'
 r['investigation_questions']=[checks[k] for k in r['readme_signals']] or ['Проверить применимость по назначению; README не дал архитектурных сигналов']
 r['advantage_verified']=False
catalog.sort(key=lambda r:r['full_name'].lower())
json.dump(catalog,open(ROOT/'research/catalog-final.json','w'),ensure_ascii=False,indent=2)
fields=['full_name','html_url','category','description','language','stargazers_count','archived','pushed_at','license','review_level','readme_status','readme_blob_sha','readme_sha256','readme_signals','investigation_questions','advantage_verified']
with open(ROOT/'research/catalog-final.csv','w') as f:
 w=csv.DictWriter(f,fieldnames=fields);w.writeheader()
 for r in catalog:w.writerow({k:('; '.join(r[k]) if isinstance(r[k],list) else r[k]) for k in fields})
summary={'created_utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),'broad_queries':12,'broad_unique_candidates':1031,'broad_excluded':444,'broad_included':587,'final_unique_repositories':len(catalog),'excluding_ffmpeg_baseline':len(catalog)-1,'readmes_collected_for_final_catalog':sum(r['readme_status']=='ok' for r in catalog),'readmes_collected_total':sum(r['status']=='ok' for r in idx.values()),'selected_source_projects':len(source_names),'selected_source_files':len([r for r in sources if 'error' not in r]),'categories':dict(collections.Counter(r['category'] for r in catalog)),'extra_exclusions':extra_exclude,'warning':'Repository landscape, not 500 direct substitutes or 500 audited codebases. Signals are automatic keyword hits, not verified implementation or performance claims.'}
json.dump(summary,open(ROOT/'research/summary.json','w'),ensure_ascii=False,indent=2)
def clean(s):return str(s or '').replace('|','/').replace('\n',' ')
lines=['# Каталог медиапроектов','',f"{len(catalog)} уникальных репозитория, включая FFmpeg как базовую линию. Уровень проверки указан явно. Описания принадлежат авторам проектов; превосходство не проверено. Архитектурные вопросы из README выделены автоматически и требуют проверки исходников. Число звёзд не является оценкой качества.",'','| № | Проект | Категория | Назначение по описанию автора | Проверка | Сигналы README |','|---|---|---|---|---|---|']
for i,r in enumerate(catalog,1):lines.append(f"| {i} | [{r['full_name']}]({r['html_url']}) | {r['category']} | {clean(r['description'])} | {r['review_level']} | {', '.join(r['readme_signals']) or 'нет'} |")
(ROOT/'research/CATALOG.md').write_text('\n'.join(lines)+'\n')
manifest=[]
for folder in ['raw','sources']:
 for p in sorted((ROOT/'research'/folder).rglob('*')):
  if p.is_file():manifest.append({'path':str(p.relative_to(ROOT)),'bytes':p.stat().st_size,'sha256':hashlib.sha256(p.read_bytes()).hexdigest()})
json.dump(manifest,open(ROOT/'research/evidence-manifest.json','w'),indent=2)
print(json.dumps(summary,ensure_ascii=False,indent=2))
