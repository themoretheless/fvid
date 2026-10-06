# Собственные контейнеры и кодеки: состояние реализации

Цель: собственные H.264, H.265 и AAC в FVid; production-модуль `media`
не зависит от FFmpeg.

## Актуальный статус зависимости 2026-10-05

Production API библиотеки, `media`, `media-cuda` и `cuda-hw` используют собственный
backend. `legacy-ffmpeg` стал пустым compatibility marker для отдельного
reference benchmark; он не включает libav в библиотеку. Обязательный offline
аудит всех 11 normal/build графов, включая `--all-features`, прошёл для macOS,
Linux и Windows. В графах нет FFmpeg/libav adapter packages.

Core/player library tests прошли с пустым executable PATH: 893 passed,
23 ignored. Для текущих CLI/export исправлений отдельно прошли 18 CLI unit,
2 media-play CLI, 6 AVC lossless и 18 native export tests. Также прошли 48 тестов AVC multislice/scaling и HEVC multislice/playback/RExt
с явным software-входом, независимо от наличия VideoToolbox. Полный integration
аудит остаётся отдельной проверкой. Native `media play` больше не обращается
к удалённому legacy player API. SDK FFmpeg для production не нужен.

Это проверка независимости, а не доказательство полного покрытия profiles/tools,
кодекового паритета или 60 fps. Аппаратная NVIDIA-проверка отложена пользователем;
оставшиеся codec ограничения требуют собственных acceptance-тестов. Датированные
секции ниже сохраняют историю миграции и могут описывать уже удалённые legacy edges.

## Актуальная проверка 2026-10-01

Проверены текущие исходники и выполнена команда без feature `media`:

```sh
cargo test --locked --offline --no-default-features \
  --test avc_residual --test hevc_playback --test native_aac_960 \
  --test native_aac_media --test native_aac_selection --test native_aac_streaming
```

Все 33 теста прошли: 1 AVC residual, 9 HEVC playback, 2 AAC 960,
14 AAC media, 1 AAC selection, 6 AAC streaming. HEVC сравнивается с сохранёнными
независимыми YUV-эталонами Main/Main10 и weighted temporal prediction;
AAC 960 — с сохранёнными PCM-эталонами. Эта команда не доказывает полную
conformance H.264/H.265/AAC и не покрывает все инструменты их профилей.

Проверка миграции API: `src/media.rs` ещё содержит вызовы `fvid_media`
для неподдержанных вариантов remux/trim/concat, transcode, subtitle и
video filters; `src/media_cli.rs` использует legacy `play_paths` и hardware
filter/decode. `media = ["dep:fvid-media"]` по-прежнему связывает публичную
feature с адаптером. Для завершения требуется миграция этих операций с
сохранением поведения, а не удаление fallback или замена на ошибки.

AAC source metadata теперь содержит фактическую маску декодера. Собственный
mix сохраняет PCE mask 0xff для ADTS/MP4/Matroska; тест двух одинаковых
источников совпадает с одиночным WAV export побайтно, включая header.

Текущие явные границы собственного декодирования:

- `src/codec/avc_decoder.rs`: повторённые и обновлённые in-band SPS/PPS
  принимаются до slices, включая `avc3`; смена SPS требует IDR и нового DPB.
  Reset возвращает исходную конфигурацию. Main/CABAC I/P/P поток со сменой
  64x64 -> 96x64 совпадает с независимым YUV-эталоном; native playback и
  camera bridge проходят смену размера и перемотку. Scaling matrices 4x4/8x8 подключены к intra/inter reconstruction с правилами
  наследования SPS/PPS; JVT/custom I/P/B совпадают с YUV-эталонами. Multiple
  slices и другие неподключённые инструменты ещё ограничены. Transform bypass
  при QP-prime 0 подключён; 8/10-bit CABAC/CAVLC lossless I/P
  совпадают с исходным YUV, включая rewind. Lossless B этими фикстурами не проверены.
- `src/codec/hevc_decoder.rs`: независимые multi-slice headers разбираются
  через `slice_headers`, с проверкой общей picture identity и порядка CTU.
  Независимые slices восстанавливаются в общие planes с WPP, CABAC reset,
  slice-aware prediction/deblocking/SAO. Main/Main10 I/P/B сравниваются
  побайтно с независимыми YUV-эталонами, включая rewind и усечения.
  Поток из 18 кадров покрывает temporal MVP, несколько active references,
  B reorder и восстановление DPB после seek назад/вперёд после EOF.
  Slice-local motion indices переводятся по POC в общую таблицу перед
  deblocking и публикацией DPB; перестановки/пересечения проверены unit test.
  Независимый encoded oracle с различающимися slice reference lists ещё нужен.
  Dependent segments наследуют header, CABAC и QP state; WPP переносит
  контексты второго CTU строки. HM-generated intra/inter streams с segment
  boundaries внутри строки и между строками совпадают с HM и FFmpeg raw
  reconstruction. Изменённые in-band PPS принимаются перед slices при известном SPS; reset возвращает исходную конфигурацию. Изменённые SPS принимаются перед slices; переход к другому SPS требует random-access picture и очищает старые references. Идентичные повторённые parameter sets принимаются.
- `src/codec/aac_native.rs`: декодер AAC-LC поддерживает стандартные 1–6 каналов,
  configurations 7 (7.1 wide), 11 (6.1 back), 12 (7.1) и однозначные горизонтальные PCE layouts. Восьмиканальный PCE в MP4 проверен
  по каждому динамику с независимым PCM-эталоном; собственные CLI/API WAV
  exports сохраняют mask 0xff и учитывают priming/edit list. Configuration 7
  проверена со strict независимым PCM-эталоном; coupling/height PCE layouts и
  неподдержанные fill-extension tools ещё не подключены к полному пути.
- ADTS configuration=0 декодируется, если PCE предшествует аудио в первом
  пакете; перед PCE допускаются DSE и FIL с обычной и расширенной длиной.
  FIL принимает fill/fill-data; SBR и другие неподдержанные tools отклоняются.
  Потоковый и индексируемый пути совпадают с независимым восьмиканальным
  PCM-эталоном; CLI/API WAV сохраняют mask 0xff. Matroska и MP4 remux/concat
  сохраняют полную PCE-конфигурацию, пакеты и PCM; MP4 ES descriptor
  поддерживает многобайтовую длину с вложенным SLConfigDescriptor.
- `src/codec/config.rs`: принимается AAC-LC object type 2; это не HE-AAC/SBR.
- `Cargo.toml`: native core и camera FFI работают без `fvid-media`, однако
  optional feature `media` по-прежнему включает legacy FFmpeg adapter.
  Независимость camera-only сборки не доказывает независимость всех операций.

Camera bridge отдельно прошёл 9 Swift-наборов и 9 сравнений AVC/HEVC/VP9/AV1
с прямым software decode, с проверкой dependency graph и линковки без FFmpeg.
Сравнение двух путей FVid проверяет мост и выбор кадра, а не независимую
codec conformance. Установка камеры блокируется реальными provisioning
profiles Apple; текущие доказательства и команда сборки описаны в
[VIRTUAL_CAMERA.md](VIRTUAL_CAMERA.md).

Разделы ниже — история реализации. Формулировки «ещё не разбирается» или
«пока только syntax» относятся к указанному этапу и не заменяют текущий код
и тесты.

## Выполнено

- `src/container/mp4.rs`: собственный индексируемый MP4-demux для
  нефрагментированных self-contained файлов. Поддержаны 32/64-bit box sizes,
  `stsz`, `stts`, `ctts` v0/v1, `stsc`, `stco/co64`, `stss`, track IDs,
  track/movie timescale, edit lists v0/v1 с единичной скоростью.
- Извлекаются sample entries `avc1/avc3`, `hvc1/hev1`, `mp4a` и сырые
  конфигурации `avcC/hvcC/esds`. Это не декодирование видео или аудио.
- API возвращает track-media DTS/PTS и отдельный edit list; edit list пока
  не применяется. Нельзя использовать эти PTS как movie presentation time
  без обработки edit list, особенно для AAC priming и начала B-frame видео.
- Пакеты читаются по индексу в переиспользуемый буфер. Проверяются границы
  файла, родительского box, `mdat`, согласованность таблиц и лимиты.
  Поиск содержащего `mdat` двоичный, без полного прохода на каждый chunk.
- Ограничения по умолчанию: moov 32 MiB, суммарно 1 млн samples,
  64 tracks, пакет 32 MiB. Это лимиты отдельных структур, не RSS.
- `src/codec/bits.rs`: собственный MSB bit reader, unsigned/signed
  exponential-Golomb, RBSP unescape и проверка trailing bits.
- `src/codec/config.rs`: собственные avcC/hvcC parsers с проверкой
  длины, reserved bits и типов parameter-set NAL; borrowed iterator
  length-prefixed NAL units; esds descriptor reader и AAC-LC
  AudioSpecificConfig (частота, каналы, frame length, core-coder delay).
  HEVC VPS/SPS/PPS пока возвращаются как NAL bytes;
  их внутренняя syntax ещё не разбирается. AAC PCE, HE-AAC/SBR и ER
  tools отклоняются. Конфигурационный parser не является декодером.

- `src/codec/avc.rs`: собственные SPS/PPS H.264, включая chroma/bit depth,
  cropping, POC types 0/1/2, scaling-list syntax, VUI/HRD, параметры
  квантования, CABAC/CAVLC flags и slice-group mapping syntax. Проверяются
  RBSP trailing bits, ссылки PPS→SPS, диапазоны параметров и геометрия.
  Это разбор syntax, не полная проверка profile/level conformance.
  Scaling lists пока сохраняются в scan order; fallback A/B и применение
  матриц будут частью декодера. Извлечение slice-group параметров не
  означает, что восстановление FMO-карт и декодирование slices уже работают.

- `src/codec/avc_transform.rs`: inverse scan 4×4 для frame/field,
  обратное квантование с масштабными весами и QP, integer inverse transform,
  DC Hadamard для Intra16 luma и 4:2:0 chroma, сложение residual/prediction
  с clipping. Промежуточные вычисления i64; недопустимый диапазон residual
  отклоняется. Поддерживается bit depth 8–14; 8×8 transform реализован отдельно ниже; bypass ещё нет.
- `src/codec/avc_prediction.rs`: все четыре Intra16 режима и реконструкция
  luma-макроблока из отдельного DC и 16 AC-блоков. API принимает уже
  декодированные коэффициенты. Подключён к CAVLC picture decoder ниже.
  Intra deblocking подключён на уровне полного кадра ниже.
- Проверки вычислений: независимая матричная формулировка transform на
  базисных векторах, все QP 0–87 на DC, clipping, affine plane prediction,
  отсутствие соседей и размещение residual-блоков в макроблоке. Это не
  заменяет побайтовое сравнение декодированных видеокадров с эталоном.

- `src/codec/avc_slice.rs`: разбор обычных slice NAL 1/5, выбор PPS,
  I/P/B/SP/SI header syntax, frame/field/POC fields, reference-list modifications,
  weighted prediction tables, MMCO syntax, QP/QS, deblocking offsets,
  changing slice-group cycle и CABAC alignment. Возвращает RBSP и точное
  bit offset начала entropy payload. Проверяет ограничения списков и полей.
  Data partitioning NAL 2/3/4 и расширения SVC/MVC не поддержаны.
  MMCO пока только разобраны, не применены к decoded picture buffer.
- Дифференциальная проверка 24 slice headers (High 10 и AVC I/P/B)
  сравнивает first_mb, frame_num, slice_type, PPS ID, QP delta и точное
  положение начала entropy payload с FFmpeg `trace_headers`.
  Дополнительно разобраны 3600 Baseline slices локального тестового файла.
  Unit-тесты включают сохранённый Baseline IDR header и побайтовые мутации.

- `src/codec/cavlc.rs`: собственное residual CAVLC decoding, включая все
  coeff_token contexts, trailing-one signs, level prefix/suffix escapes,
  total_zeros и run_before для 4×4/AC15 и chroma DC 2×2/2×4. Выход —
  коэффициенты в scan order и TotalCoeff для соседних блоков. Лимит
  level_prefix=31 защищает числовой диапазон; profile-specific ограничения
  должен применить будущий slice decoder.
- `src/codec/cavlc_tables.rs`: числовые таблицы 9-5, 9-7…9-10 из
  ITU-T H.264 (02/2016). `scripts/generate_cavlc_tables.py` воспроизводит
  их из официального PDF с проверкой SHA-256; зависимость pdfplumber
  нужна только для генератора. Повторная генерация побайтово совпала.
- Проверены все VLC-коды и их prefix-free свойство, фиксированные
  bitstream vectors и 2400 roundtrip блоков с разными contexts/levels.
  Интеграционный тест восстанавливает известный блок пикселей из CAVLC
  битов через inverse scan/scaling/transform. CAVLC подключён к intra
  macroblock reader и сборке полного intra picture ниже; CABAC syntax ещё отсутствует.

- `src/codec/avc_intra.rs`: все девять Intra4×4 и четыре 4:2:0 chroma
  prediction modes, вывод prev/rem режима, top-right substitution и
  проверка доступности соседей. Фиксированные векторы проверяют каждый
  режим; affine/constant vectors проверяют plane и разрядности 8–14.
- `src/codec/avc_macroblock.rs`: последовательный reader progressive
  4:2:0 CAVLC I-slices. Читает Intra4/Intra16/PCM syntax, QP, CBP,
  DC/AC коэффициенты; поддерживает контексты соседних блоков. Возвращает
  блоки и коэффициенты, не изображение. Intra8×8 подключён; FMO, interlace,
  CABAC и P/B macroblocks явно не реализованы. Context budget ограничен.
- Проверка нового Baseline fixture: все 72 макроблока трёх I-slices
  разобраны до RBSP trailing bits; дополнительно разобраны 15 I-slices
  длинного 960×540 ролика (30600 макроблоков). Это проверка syntax/coverage,
  а не доказательство совпадения всех коэффициентов или целых кадров
  с эталоном. Побайтовое сравнение reconstructed frames описано ниже.

- `src/codec/avc_picture.rs`: сборка целого single-slice progressive 4:2:0
  CAVLC/CABAC I-кадра с Intra4/Intra8/Intra16/PCM, chroma QP mapping, cropping и
  planar YUV export. Поддерживаются 8–14-bit вычисления; с реальным
  эталонным потоком проверены 8 и 10 бит. Требуются стандартные плоские
  scaling weights; custom matrices и bypass
  явно отклоняются. Проверяются полнота кадра и бюджет planes/context grids.
- `src/codec/avc_deblock.rs`: собственный progressive intra deblocking,
  bS=4 на границах макроблоков и bS=3 внутри, с учётом 8×8 transforms,
  отдельные luma/chroma QP,
  alpha/beta offsets, 8–14-bit thresholds. Фильтрация после reconstruction
  сохраняет неотфильтрованных соседей для intra prediction. Inter bS=0–2
  и границы между slices пока не реализованы.
- `examples/decode_avc_intra.rs` соединяет собственные MP4, SPS/PPS,
  slice/CAVLC, prediction и inverse transforms без FFmpeg. Это пример
  ограниченного декодера, ещё не общий backend плеера.
- `benchmarks/validate_native_avc_intra.py`: все YUV bytes совпали с
  независимым FFmpeg decoder на 42 кадрах: pattern 96×64 (27648 bytes),
  cropped 66×50 (14850), flat 64×48 (13824), High 10 96×64 (55296).
  Дополнительно проверены deblocking 8-bit (27648 bytes), 10-bit (55296)
  и offsets +3/−2 на cropped 66×50 (14850). CABAC Main (27648),
  CABAC High 10 (55296) и CABAC High cropped (14850) также совпали.
  Четыре fixtures 128×96 проверяют 8×8: CABAC 8-bit (55296), CABAC
  10-bit (110592), CABAC mixed Intra4/8 (55296), CAVLC (55296).
  Encoder statistics подтверждают ненулевое использование 8×8 transforms.
  FFmpeg используется только в benchmark как encoder/oracle.
  Unit-тесты отдельно проверяют видимый crop, нейтральные пиксели,
  недостаточный budget, неполный кадр и усечённый payload.

- `src/codec/cabac.rs`: собственное арифметическое ядро CABAC — decision,
  bypass, terminate, renormalization и H.264 context initialization из m/n/QP.
  Проверяются начальный offset, alignment, усечение и обращения после
  termination; неудачное чтение сохраняет прежнее состояние контекста/ядра.
  Таблицы range/state transitions воспроизводятся из официального PDF
  скриптом `scripts/generate_cabac_tables.py` с проверкой SHA-256.
- CABAC core проверен фиксированной арифметической трассой и 1024
  смешанными последовательностями всех 64 состояний. Независимый тестовый
  encoder хранит целый интервал в u128; совпадают bins, итоговые contexts
  и точная позиция потока. Это проверка арифметики, не CABAC video decoding:
  I-macroblock syntax подключена ниже; P/B syntax ещё предстоит реализовать.

- `src/codec/avc_cabac_init.rs`: таблицы m/n для contexts 0–459,
  I/SI и всех трёх cabac_init_idc, извлечённые из таблиц 9-12…9-24
  официального PDF. Неиспользуемые contexts представлены как None и
  отклоняются при обращении. Генератор проверяет заполненность и диапазоны.
- `src/codec/avc_cabac.rs`: банк контекстов и residual_block_cabac для
  luma DC/AC/4×4 и chroma DC/AC (4:2:0 и 4:2:2), frame/field significance,
  implicit last coefficient, reverse level order, UEG0 escapes и sign bins.
  Соседний coded-block context передаётся вызывающим macroblock reader.
  Проверки syntax traces подтверждают context indices и коэффициенты;
  реальное CABAC intra-видео проверено через reader ниже. 8×8 residual syntax
  подключена к picture reader ниже.
  Расширенные contexts 460–1023 ещё не подключены.

- `src/codec/avc_cabac_macroblock.rs`: progressive 4:2:0 CABAC I-slice
  reader для Intra4/Intra8/Intra16/PCM, mb_type/CBP/QP/mode binarization,
  контекстов соседних макроблоков и coded-block flags. PCM перезапускает
  arithmetic engine с сохранением context bank. Reader выдаёт тот же
  IntraMacroblock, что CAVLC, и подключён к picture reconstruction/deblocking.
  I_PCM путь реализован, но пока не покрыт реальным PCM fixture.
- CABAC termination проверяется после каждого макроблока; extra zero words
  проверяются отдельно. Неиспользуемые биты последнего арифметического
  байта допускаются для совместимости с x264 flush padding. Это не строгая
  проверка всех RBSP alignment bits. Сохранённый 16×16 Main IDR проверяет
  известные пиксели без FFmpeg, каждое побайтовое усечение и мутации payload.

- `src/codec/avc_transform8.rs`: inverse scan frame/field, scaling с шестью
  normalization classes и QP, integer inverse transform 8×8 и reconstruction
  с clipping. Все 64 basis coefficients обоих знаков проверены независимым
  матричным вычислением; DC проверен на QP 0–87. Wide intermediates
  предотвращают overflow, выход за i32 явно отклоняется.
- `src/codec/avc_8x8_tables.rs`: scan и CABAC significance maps из таблиц
  8-14 и 9-43 официального PDF; воспроизводятся генератором CABAC tables.
  `AvcCabac::residual8` реализует 64 coefficients с inferred coded-block flag
  для 4:2:0/4:2:2 luma. Frame/field syntax traces проверяют context selection.
  Intra8 prediction и подключение к macroblock/picture завершены для
  текущего progressive 4:2:0 intra subset; сравнение кадров описано выше.
- `intra8` в `src/codec/avc_intra.rs`: фильтрация reference samples,
  девять режимов prediction, top-right replication и mode derivation через
  общую сетку Intra4/8 modes. CAVLC deinterleaves четыре residual streams
  в 64 coefficients; CABAC использует отдельные 8×8 contexts. Deblocking
  пропускает внутренние luma edges на 4 и 12 при transform_size_8x8_flag.
  Проверены filtered edges, отрицательные directional branches и отсутствие
  соседей. Общие Intra4/8 формулы сохраняют прежние Intra4 проверки.

- `src/codec/avc_motion.rs`: собственная progressive motion compensation:
  quarter-pel luma с 6-tap фильтром и без промежуточного clipping при
  diagonal half-pel, eighth-pel bilinear chroma, signed vectors и edge extension.
  ReferencePlane проверяет geometry/stride/bit depth; prediction пишет в
  предоставленный буфер partition до 16×16 без allocations.
- Реализованы explicit weighted prediction, default bi-prediction и применение
  переданных implicit weights. Offsets масштабируются по bit depth.
  Derivation implicit weights из POC реализована отдельно ниже. Проверены все 16
  luma fractions на impulse, luma/chroma на affine field со signed vectors,
  extreme motion vectors, padding stride, diagonal negative filter taps,
  rounding и clipping. Это primitive tests, не доказательство P/B decoding:
  macroblock syntax, motion-vector prediction и reference-picture management
  ещё предстоит соединить с этими примитивами.

- `src/codec/avc_mv.rs`: progressive motion-vector predictors — median,
  единственная matching reference, directional 16×8/8×16, C→D fallback,
  P-skip, проверяемое сложение predictor+difference. Недоступный сосед
  отличается от доступного intra/другого списка без prediction.
- Temporal direct vector scaling и implicit B weights вычисляются из POC
  с clipping distances, long-term/zero-distance fallback и защитой от
  переполнения. Векторы вне signed 16-bit отклоняются. Проверены partition
  preferences, neighbour availability, reference matching, P-skip,
  forward/reverse temporal distances, long-term и числовые границы.
  Это ещё не DPB и не inter macroblock reader: выбор и хранение reference
  pictures и извлечение соседей остаются вызывающему слою; POC описан ниже.

- `src/codec/avc_poc.rs`: stateful POC types 0/1/2 с LSB/frame_num
  wraparound, cycle offsets, non-reference history, field POC и MMCO 5 reset.
  Возвращает значения до и после reference marking, не смешивая текущую
  temporal prediction с последующим хранением reference picture. Ошибка
  не изменяет state; проверяются IDR origin и signed 32-bit range.
- POC unit vectors покрывают B-order, обе границы half-range wrap, cycle,
  frame_num wrap, MMCO 5 и bottom field. MP4 verifier сравнивает 27 POC
  с presentation timestamps x264 fixtures (включая B-кадры) внутри каждого
  IDR sequence. Это ещё не output reorder queue или DPB. Вызывать API
  нужно один раз на picture; gap-inferred pictures должен передать DPB слой.
  Начало без IDR и смена POC configuration без IDR пока отклоняются.

## Проверки

```sh
cargo test --locked --offline --no-default-features --lib --test mp4
cargo run --locked --offline --no-default-features --example mp4_packets -- input.mp4
python3 benchmarks/validate_native_mp4.py
python3 benchmarks/validate_native_avc_intra.py
```

MP4-тесты проверяют содержимое пакетов, знаковые composition offsets,
разные sample-to-chunk runs, edit list, co64, extended size, усечения по
каждому байту, испорченные таблицы, внешние references и бюджет.
Побайтовые мутации небольшого контейнера проверяют отсутствие panic.
Это не заменяет полноценный fuzzing на большом корпусе.

Дифференциальный скрипт в `benchmarks` создаёт AVC+B/AAC и HEVC fixtures
и сравнивает все packet offsets, sizes, DTS, PTS, durations и sync flags
с ffprobe (`-ignore_editlist 1`, то есть тот же media timeline).
Пример также разбирает codec configuration каждой дорожки и проверяет
NAL framing всех AVC/HEVC пакетов, SPS/PPS H.264 и совпадение видимых
размеров с ffprobe. Дополнительно генерируется High 10 файл 66×50
с cropping и SAR 4:3 (12 пакетов); Baseline SPS 960×540 проверяется
на сохранённом NAL fixture, включая coded size 960×544 и timing.
На локальном запуске совпали 60 AVC/AAC и 12 HEVC пакетов; дополнительно
проверены 3600 пакетов длинного AVC fixture. FFmpeg/ffprobe вызываются
только эталонным скриптом, не новым production-reader.

## Ограничения MP4

Пока нет fragmented MP4, compact sample-size `stz2`, multiple sample
entries, encrypted/external tracks, QuickTime audio entry v1/v2.
Такие входы явно отклоняются. Произвольный codec sample entry тоже
отклоняется. Codec configuration ещё должна быть валидирована декодером.

## Следующие обязательные шаги

1. Разбор внутренней syntax HEVC parameter sets; AAC program config
   elements и дополнительные профили поверх реализованного AAC-LC config.
2. H.264 reconstruction: multi-slice pictures, P/B macroblock syntax,
   CABAC P/B macroblocks, motion-vector prediction и подключение
   motion compensation, scaling-list fallback,
   reference-picture management, inter deblocking, display reordering.
3. H.265 reconstruction с собственными CABAC, prediction, transforms,
   reference management, deblocking и SAO.
4. AAC decoding: syntax, Huffman, inverse quantization, stereo tools,
   filterbank/overlap, priming. Затем синхронизация и звук плеера.
5. Подключить эти декодеры к контейнеру и плееру; переносить `media` и MCP
   с сохранением нужных операций. Убрать libav из production build graph.
6. Проверять декодированные пиксели/PCM, timestamps, seek, повреждённые
   данные и расход памяти на независимом корпусе. Сравнение packet index
   не является доказательством работы декодеров.

Legacy `fvid-media` пока остаётся зависимым от FFmpeg. Его наличие не
маскируется переименованием или фиктивными заглушками.

## Форматы и справочные материалы

- [MP4RA: зарегистрированные box types](https://mp4ra.org/registered-types/boxes).
- [W3C ISO BMFF byte stream format](https://www.w3.org/TR/mse-byte-stream-format-isobmff/).
  Описание fragmented media не означает, что reader уже его поддерживает.

- [W3C AAC registration](https://www.w3.org/TR/webcodecs-aac-codec-registration/):
  связь raw AAC с AudioSpecificConfig.
- [W3C HEVC registration](https://www.w3.org/TR/webcodecs-hevc-codec-registration/):
  связь length-prefixed потока с HEVCDecoderConfigurationRecord.

- [ITU-T H.264](https://www.itu.int/rec/t-rec-h.264): SPS/PPS syntax,
  scaling lists и VUI/HRD (разделы 7.3/7.4 и приложение E).

### Списки опорных кадров AVC

`codec::avc_references` строит списки прогрессивных P/SP/B-кадров:
FrameNumWrap, сортировка по POC, перестановка одинаковых B-списков,
short-term/long-term modifications с сохранением повторов в изменённом
префиксе. Проверяются отсутствующие и повторные записи, границы operands.
`avc_dpb::ReferenceBuffer<T>` хранит опорные кадры через `Arc<T>`,
применяет IDR reset, sliding window и MMCO 1–6 атомарно при ошибках.
Списки связаны с буфером; MP4 inspector прогоняет его на метаданных
реальных потоков, пока без межкадровой реконструкции пикселей.
Non-existing pictures, frame-number gaps, field lists и очередь показа
ещё не реализованы.
Проверка: `cargo test --locked --offline --no-default-features` — 98 тестов.

### Inter prediction syntax AVC

`avc_inter` implements P/SP/B mb_type and sub_mb_type partition tables,
including mixed L0/L1/Bi prediction, direct partitions and I-type offsets.
The progressive CAVLC prediction reader handles truncated-Golomb reference
indices, P_8x8ref0 inference and signed motion differences in syntax order;
reader state is restored on error. Two tests cover geometry for every table
entry and fixed bitstreams for reference/MVD order and truncation.
All 100 no-default-features tests pass. This is not yet a full P/B macroblock
reader: residuals, motion-neighbour state, direct derivation and inter
reconstruction still need integration; CABAC inter syntax remains pending.

`avc_compensation::Reference420` validates deblocked Y/Cb/Cr planes once and
produces packed partition predictions without heap allocations. It connects
quarter-luma and eighth-chroma interpolation for progressive 4:2:0, including
edge extension, uni weights and bi weights at 8–14-bit sample depth. Fixed
buffers carry the prediction into future residual reconstruction. Tests cover
fractional affine samples, chroma coordinates, blend/offset depth scaling,
extreme vectors and malformed geometry. Full suite: 102 tests pass; this does
not yet prove complete P/B picture reconstruction.

`MacroblockPrediction` assembles partition predictors into 16x16 luma and
8x8 chroma buffers, validates sample depth and rejects overlap, out-of-range
origins and incomplete coverage. A mixed 16x8/8x8 test checks chroma/luma
placement and failed-insert atomicity. Full no-default-features suite: 103 tests.

Inter macroblock predictors now accept raster luma residuals (4x4 or 8x8)
and chroma DC/AC coefficients, apply component quantization/scaling lists,
and reconstruct clipped pixels before deblocking. Component QP derivation and
scaling-list fallback remain caller responsibilities. Tests cover signed DC,
block placement, saturation, chroma DC and zero-residual identity. All 104
no-default-features tests pass. Entropy/motion-field integration and inter
boundary-strength derivation are still required for complete P/B decoding.

## Дополнение цели: веб-камера

Пользователь подтвердил направление: видеофайл из FVid передаётся другим
приложениям как виртуальная камера. Захват физической камеры не входит
в это дополнение. Сейчас соответствующих backend/API в FVid нет. Эта часть входит в общую цель и ещё не выполнена;
успешные тесты кодековых примитивов её не подтверждают.

`avc_boundary::strength` derives progressive bS=0..4 from intra/switching
mode, nonzero luma coefficients and motion/reference identity. It handles
swapped B lists and the ambiguous same-picture bipred pairing, with widened
arithmetic for extreme vectors. Tests verify priority and the quarter-sample
threshold; the full suite passes 110 tests. Integration with inter edge
filtering and picture motion metadata remains pending.

The deblocking pixel kernel now handles bS=0/1/2 as well as intra bS=3/4,
using the normative clipping tables for each weak strength. Public
`avc_deblock::filter_samples` validates parameters and sample depth. All 111
Rust tests pass; the 42-frame intra oracle corpus still matches FFmpeg YUV
byte-for-byte. Inter picture edge traversal is not yet connected.

`avc_deblock::inter_plane` now traverses progressive luma/4:2:0 chroma
macroblocks with per-segment strengths, averaged component QPs and slice
offsets. It skips absent outer edges and internal 4x4 luma edges for 8x8
transforms. Invalid metadata is rejected before mutation. A luma/chroma test
checks distinct segment strengths and error atomicity. All 112 tests pass.
Producing these edge grids from decoded P/B macroblock state is still pending.

`avc_boundary::picture_edges` connects decoded 4x4 motion/coefficient metadata
to the filter edge grids, including neighbour coordinates, component QP
averaging, 8x8 edge suppression and disable_deblocking_filter_idc 1/2 slice
rules. Tests cover mixed slice IDs, coefficient/intra strengths and chroma QP.
The complete Rust suite passes 113 tests. The full inter entropy decoder still
needs to populate this metadata; P/B bitstream decoding is not yet complete.

Intra reconstruction now uses the same metadata-driven edge traversal as inter
deblocking, removing the duplicate pixel traversal. The temporary edge-grid
allocation is included in the picture memory budget. All 113 tests pass, and
42 intra frames remain byte-identical to the FFmpeg oracle, exercising the
shared path on CAVLC/CABAC, cropped, 8/10-bit and 8x8-transform inputs.

`avc_motion_field::MotionField` provides budgeted 4x4 progressive motion storage,
slice-aware neighbour lookup and prediction-plus-MVD reconstruction for both
lists. It rejects overlap and invalid partitions; failed vector derivation
does not publish partial state. P-skip/B-direct supply their derived vectors
through the same store API. All 114 Rust tests pass. The field is not yet
connected to the complete P/B entropy loop.

MotionField now accepts parsed non-direct macroblock partitions as a unit,
derives directional 16x8/8x16 predictors in decode order, checks full coverage
and restores all cells if any partition fails. P-skip derives and stores its
inferred vector. An integration test feeds real CAVLC prediction bits through
partition parsing into the field, exercises late-partition failure rollback,
and decodes the following P-skip. All 115 tests pass. B-direct integration and
the surrounding residual/macroblock entropy loop remain pending.

`avc_inter_prediction::predict_macroblock` connects parsed partitions and final
motion vectors to reference planes, interpolation, per-component weighting and
macroblock assembly. It validates prediction/list agreement and missing
references. A CAVLC B_Bi bitstream test now runs through motion derivation to
expected Y/Cb/Cr pixels. All 116 Rust tests pass. This joins prediction stages;
complete slice decoding and residual syntax integration remain unfinished.

CAVLC residual control now includes inter coded_block_pattern mapping,
conditional transform_size_8x8_flag and mb_qp_delta. Table 9-4 and QP wrapping
are shared with the active intra reader. Invalid control syntax restores the
bit cursor. All 118 tests pass; all 42 intra oracle frames still match. The
inter coefficient reader and full slice loop remain to be connected.

`read_inter_header` now joins mb_type, subpartition prediction syntax, CBP,
conditional transform8 and QP delta with transactional cursor handling.
Transform8 eligibility follows partition sizes and direct_8x8_inference_flag.
Tests cover every P/B subpartition type and a complete fixed inter header;
all 120 Rust tests pass. Coefficient parsing and slice iteration remain pending.

`avc_inter_coefficients` reads progressive 4:2:0 CAVLC inter residuals with
local nC propagation and externally supplied neighbour counts. It reconstructs
raster 4x4/8x8 luma levels and chroma DC/AC, returning counts for subsequent
macroblocks. Fixed vectors cover zero and nonzero coefficients plus cursor
rollback on truncation. All 122 tests pass. Full-slice state integration and
independent inter-frame pixel validation are still required.

`CoefficientField` stores CAVLC counts with slice identity and a memory budget.
Its `read_inter` combines the header and coefficient parser, committing cursor
and neighbour counts only after successful decoding. Skip/intra/PCM counts
can be published through the same field. Tests cover complete inter syntax,
top/left propagation, slice isolation and failure rollback. All 123 tests pass.
Full-slice dispatch and inter-frame oracle validation remain pending.

`InterCavlcSlice` iterates progressive 4:2:0 inter macroblocks, expands bounded
mb_skip_run, propagates QP/counts and validates RBSP trailing bits. Fatal parse
errors poison the reader. Tests cover skip/coded/skip sequences and excessive
runs; all 125 tests pass. Intra macroblocks within P/B slices remain explicitly
unsupported, and picture assembly must enforce complete coverage. This reader
is an integration stage, not a completed P/B decoder.

`avc_inter_picture::decode_p_picture` now connects single-slice CAVLC P parsing,
motion fields, reference interpolation, residual reconstruction and deblocking
into coded Y/Cb/Cr planes. Current supported path is progressive 4:2:0, flat
scaling, unweighted prediction; intra-in-P is still rejected. The memory budget
uses a conservative per-macroblock bound. Full skip and zero-residual picture
tests reproduce a patterned reference exactly. All 126 tests passed before the
final reference-identity validation adjustment; the targeted picture test also
passes after it. Real inter-bitstream oracle coverage is still pending.

`examples/decode_avc_ip.rs` integrates MP4 packet reading, POC, reference lists,
reference-frame storage and native I/P picture decoding. It rejects display
reordering rather than writing misordered output. `validate_native_avc_ip.py`
checks three real static-color streams, each one I and four P frames: all 15
frames match FFmpeg bytes. This corpus principally exercises reference/skip
integration, not general inter residual/motion coverage. A changing-luma
fixture currently fails at intra-in-P macroblock dispatch, establishing the
next required integration. The Rust suite passes 126 tests.

IntraCavlcReader now exposes shared-context construction and embedded intra
macroblock parsing after a P/B dispatcher maps mb_type to the I table. Inter
counts can be imported and completed intra counts exported, avoiding a second
intra entropy implementation. Existing I-slice decoding uses the same body.
All 126 tests pass; 42 intra oracle frames remain byte-identical. Mixed-slice
dispatch and shared pixel reconstruction are not yet wired into decode_p_picture.

Mixed CAVLC slice dispatch now parses embedded intra macroblocks and exchanges
counts with the inter context; PCM preserves the preceding slice QP. The MP4
inspector uses this path. The changing-luma fixture that previously stopped
now parses all seven P slices: each contains one inter and three intra blocks.
All 126 tests pass. Pixel reconstruction for embedded intra blocks remains
unconnected in decode_p_picture, so this is entropy proof, not pixel proof.

Embedded intra reconstruction is now connected to decode_p_picture through
shared reconstruction code used by the I decoder. Intra blocks update motion
availability, coefficient contexts, QPs and deblocking metadata. The changing
luma MP4 now decodes all eight frames byte-identically to FFmpeg and is retained
in the I/P oracle script. All 126 Rust tests pass, as do 23 I/P and 42 intra
oracle frames. Constrained intra prediction, weighted P, CABAC P and B-picture
assembly remain unsupported in this path.

P reconstruction now applies explicit per-reference luma/chroma weights,
including skip predictions. An eight-frame weighted ramp with confirmed
luma_weight_l0_flag=1 matches FFmpeg; a 12-frame moving CRF fixture also matches.
The expanded persistent oracle adds motion at fixed QP=20 and currently FAILS
its pixel comparison. This unresolved regression must not be treated as broad
P-decoder conformance. The Rust suite still passes 126 tests; those tests do
not cover the newly found moving-frame discrepancy.

The fixed-QP moving-frame regression is resolved: an available inter neighbour
contributes DC to intra mode derivation, rather than making the neighbour
unavailable (unless constrained_intra_pred excludes it). The incorrect mode
first affected an intra block inside P frame 10 and propagated through its
reference. The persistent I/P oracle now passes all 43 frames byte-for-byte,
including fixed-QP motion and explicit weights. A focused regression covers
both constrained and unconstrained neighbour mode derivation.

Constrained intra prediction is now connected in progressive CAVLC P pictures.
The shared intra reconstruction masks unavailable neighbours for Intra4,
Intra8, Intra16 and chroma; inter pixels enter the availability grid only when
constrained_intra_pred is false. A focused pixel regression checks all three
luma block sizes and both chroma planes with available and excluded neighbours.
The I/P oracle now passes 55 frames (including 12 constrained motion frames
with the PPS flag verified); all 42 intra oracle frames still match. The full
127-test suite and the newly added pixel regression pass. This does not add
CABAC P, B reconstruction, HEVC or AAC decoding.

The I/P access-unit integration now lives in the library as
codec::avc_decoder::AvcDecoder, rather than being implemented by the example.
It owns parameter sets, POC and decoded references; enforces a combined
reference/reconstruction budget; rejects multiple slices, unsupported in-band
NALs, frame-number gaps and output reordering explicitly; and requires reset
after an error. Returned pictures are shared Arcs; additional output storage
retained by callers is outside the decoder budget. The MP4 I/P example now
uses this API and all 55 oracle frames match. The Rust suite passes 130 tests.
GUI playback and the camera source still need this API connected; they remain
Y4M-only. This integration does not imply complete H.264 support.

playback_mp4::Mp4AvcReader now joins the own MP4 demuxer and AvcDecoder as a
library frame source. It selects or explicitly accepts an AVC video track,
returns shared decoded pictures with exact track PTS/duration/sample index,
reuses packet storage, and resets decoder/reference state on rewind. Errors
require rewind before retry. Track edit lists and movie timescale are exposed;
this layer returns media timestamps and does not yet apply presentation edits.
The I/P example uses this source and verifies first-frame pixels and timestamps
after rewind. All 55 oracle frames match. RGB conversion, presentation scheduling,
audio and GUI/camera integration are still outstanding.

The GUI player now uses playback_native::NativeReader for both Y4M and the
supported MP4/AVC subset. AVC pixels are cropped and converted to RGB using
VUI limited/full range and BT.601/709 matrices (unspecified defaults to 601),
including 10-bit input. Chroma upsampling is nearest neighbour. Per-sample
durations drive the player timer; non-contiguous timestamps, non-identity
edit lists and other colour matrices currently fail explicitly. File opening,
drag/drop, pause and restart share the native source. This is video-only.
The headless GUI-source example decodes a 12-frame moving MP4 and verifies
rewind. The persistent playback oracle covers 48 flat-colour frames, range,
matrix and depth variants; maximum RGB difference is 3/255 versus FFmpeg.
All 132 no-default-feature tests pass. The player dependency graph contains
neither fvid-media nor FFmpeg. The separate media feature remains legacy.

NativeCameraSource now connects the shared Y4M/MP4 RGB reader to the bounded
BGRA camera buffer. Frame selection uses exact rational source intervals;
backward seek, EOF hold and fractional boundaries are verified. The moving
MP4 camera regression selects frames 0,1,10,0 at 0/40/400/0 ms with exact BGRA
and timestamp checks. All 133 Rust tests pass. OS extension transport and
installation remain unconnected, as do the remaining codec and media work.

The persistent AVC I/P oracle now also covers 24 moving High Profile frames.
The own MP4/entropy inspector confirms 58 inter macroblocks actually use 8x8
transforms, so the fixture cannot silently become a 4x4-only test. All 79 I/P
frames match the FFmpeg planar oracle byte-for-byte. This strengthens evidence
for the existing CAVLC path; CABAC P and B picture reconstruction remain absent.

CABAC inter syntax now has own P/B mb_type/sub_mb_type binarizations, embedded
intra suffixes, mb_skip_flag, bounded unary reference indices and signed UEG3
motion differences. Context indices and neighbour conditions follow H.264
9.3.2/9.3.3, tables 9-37..9-41 in the pinned 2016 standard. The routines operate
on the existing AvcCabac engine; spatial neighbour inputs remain the caller's
responsibility. Four tests pass: explicit bin/context traces for P/B/intra,
reference and UEG3 boundary checks, and a saved real 32x32 CABAC P slice whose
four skip blocks reach exact arithmetic/RBSP termination. No external codec
is required by these Rust tests. Inter context grids, residual integration and
complete CABAC P/B picture dispatch remain unfinished; the public decoder
still rejects those pictures.

CABAC inter prediction now has a bounded spatial context grid. It reads P/B
subtypes, both lists' reference indices, then both lists' MVDs in normative
syntax order, updating every covered 4x4 cell. Left/top derivation observes
slice IDs; intra, skip and direct neighbours contribute zero. It reuses the
existing partition tables and supplies Partition values for reconstruction.
Malformed entropy poisons the context grid; geometry, duplicate macroblocks
and memory limits are checked. Three new trace tests cover intra-macroblock
updates, left/top cross-macroblock contexts, slice boundaries, B bi-prediction,
direct/skip context suppression and failure handling. All 141 Rust tests pass.
The grid is not yet wired into complete CABAC mixed-slice/residual decoding.

The existing IntraCabacReader now supports shared I/P/B arithmetic/context
construction, embedded intra-body decoding after external mb_type dispatch,
and publication of inter/skip neighbour entropy state with end-of-slice handling.
This reuses the tested intra prediction/residual path instead of copying it.
A saved real P slice containing an embedded intra macroblock reconstructs
Y=235/Cb=Cr=128 exactly and terminates correctly; another real P slice verifies
four skip-context updates. All 143 Rust tests pass and the 42-frame intra oracle
is unchanged. Complete mixed CABAC dispatch and inter residual parsing are
still required before enabling CABAC P/B in the public picture decoder.

CABAC P reconstruction is now connected end-to-end. Inter residual decoding
shares CBP/QP/coefficient machinery with intra, with unavailable-neighbour
coded-block defaults selected for the current prediction mode. InterCabacSlice
joins skip, P/B type syntax, motion contexts, embedded intra and inter residuals,
poisons on errors, and consumes slice termination. decode_p_picture dispatches
CAVLC or CABAC from PPS without a third-party decoder. A saved real P residual
fixture reconstructs flat luma 64 -> 66 exactly. The persistent oracle adds
24 Main-profile motion frames, 24 High/8x8 motion frames and 12 High10 motion
frames: all 139 I/P frames match FFmpeg planar output byte-for-byte. All 144 Rust
tests pass and all 42 intra oracle frames remain exact. B syntax can be iterated
but B picture reconstruction/output reordering remain unsupported; this is not
full AVC conformance. HEVC/AAC decoding and legacy media replacement remain open.

ReferenceMotionField now stores a complete picture's per-4x4 motion with stable
reference-picture IDs, original list indices and vectors. MotionField.snapshot
resolves indices, rejects incomplete/mixed-slice mappings and enforces storage
limits. Co-located lookup prefers L0 then L1; temporal-direct reference mapping
uses identity and handles reordered/duplicate list entries. P reconstruction
can now return its completed motion field through decode_p_picture_with_motion;
the existing image-only API delegates to the same implementation. Two tests
cover identity remapping, L1 fallback, intra cells, missing references, mixed
slices and budget rejection. All 146 Rust tests pass and all 139 I/P oracle
frames still match. Retention alongside DPB references and B direct/picture
assembly remain to be connected.

AvcDecoder's DPB now owns DecodedReferencePicture entries containing both the
image and optional persistent motion. Reference P pictures snapshot resolved
list identities before marking; intra-only entries need no motion allocation.
The decoder budget reserves the worst-case image+motion storage per reference
and the transient snapshot allocation. DPB marking/eviction drops both parts
together. A saved real IDR/P/P stream verifies exact luma 64/66/68, motion IDs
0->1 across list reuse, release of evicted payloads, IDR clearing and reset.
All 147 Rust tests pass; the 139-frame I/P oracle remains byte-identical.
B direct derivation, B image assembly and display reordering remain unfinished.

### Spatial direct motion derivation

Added progressive H.264 spatial direct derivation (8.4.1.2.2) using shared
macroblock-neighbour prediction. It selects the smallest nonnegative reference
per list, handles C-to-D substitution and absent lists, and applies co-located
zeroing only for short-term list1[0], co-located reference index zero, and vectors
in [-1, 1]. Intra co-located blocks do not trigger that zeroing. Tests cover these
boundaries, long-term references, both-lists-absent fallback and invalid indices.
This primitive is not yet connected to B-picture reconstruction.

### Direct motion connected to the working field

`DirectPrediction` now derives spatial or temporal direct vectors from the
retained motion of list1[0], mapping temporal references by stable picture ID.
Progressive direct-8x8 inference selects luma block indices 0/5/10/15 as required
by 8.4.1.2.1. The motion-field decoder accepts mixed explicit/direct partitions,
uses macroblock-level neighbours for direct prediction, and rolls back every
cell on a late error. Integration tests cover inference on/off, reordered
references, intra co-located fallback and late-error recovery. Native no-default-
features tests pass. B-picture pixel reconstruction/output reordering remain
unconnected; this is not yet end-to-end B-frame support.

### Progressive B-picture pixel reconstruction

Generalized the single-slice inter-picture reconstruction entry point to accept
both reference lists and DirectPrediction metadata. It now connects direct
motion to pixel prediction, handles B-skip, explicit weights on either list,
implicit bidirectional weights, residual reconstruction and cross-list reference
identity for deblocking. Tests reconstruct B-skip pixels for spatial/temporal
modes with unweighted and implicit weighted predictions and verify retained
motion identities. The existing 139-frame I/P oracle remains byte-exact; the
full native test suite passes. This entry point is not yet wired into AvcDecoder
or MP4 display-order output; real encoded B-picture oracle coverage remains due.

### Stateful B-picture decoding and real-stream oracle

AvcDecoder now offers decode_order for access-unit-order I/P/B decoding. Both
reference lists and co-located motion are sourced from the DPB, including retained
reference B pictures. The existing decode entry point retains its monotonic-POC
check, so playback cannot silently display reordered pictures incorrectly.

Added a bounded short-clip oracle example that sorts decoded outputs by MP4 PTS,
and validate_native_avc_b.py. All four 24-frame streams match FFmpeg byte-for-byte:
CABAC spatial direct, CABAC temporal direct with implicit weights, CAVLC temporal
direct with implicit weights, and a reference-B pyramid. Each includes 15 B
pictures (96 total frames). Production MP4 output reordering is still pending;
this oracle's whole-clip buffering is explicitly not the playback implementation.

### MP4 streaming presentation order

Mp4AvcReader now uses decode-order AVC output and a PTS reorder queue. A bounded
suffix-minimum index over the demuxed samples proves when a queued frame can be
emitted; only required future access units are decoded. Half the supplied memory
budget is reserved for decoding, half for the index and retained output frames;
over-budget queues fail explicitly. Rewind clears both DPB and queued pictures.
The B oracle now additionally exercises this production reader and rewind: all
96 frames in four B-stream configurations remain byte-exact. NativeReader still
requires its supported edit/timeline shape; common nonzero media-start edits in
B-frame MP4 files need timeline integration before general GUI playback.

### Native playback of B-frame MP4 timeline offsets

NativeReader now maps a single positive, rate-one MP4 edit to playback time zero,
decodes/discards preroll pictures, and clips frame intervals at edit boundaries.
Edits with fractional media-tick endpoints, empty edits and multiple ranges are
still explicitly unsupported. The RGB playback regression passes all 48 color
frames. Both I/P and reference-B motion clips pass native virtual-camera selection
at 0/40/400/0 ms with exact BGRA comparison and rewind. This verifies the native
source/buffer path, not installed macOS cross-process camera consumption.

### VFR presentation duration after B reordering

Reproduced a NativeReader failure on a real 12-frame B-pyramid clip changing
from 25 fps to 12.5 fps: decode-order sample durations caused false discontinuity
errors after display reordering. Mp4AvcReader now derives each non-final output
interval from the next composition timestamp across queued and future samples;
the final frame retains its declared sample duration. Equal/non-increasing PTS
and duration overflow fail explicitly. The VFR clip now decodes all 12 frames,
and camera selection at 400 ms correctly returns presentation frame 8 rather
than frame 10. Extended playback regression covers exact BGRA at 0/40/400/0 ms
for I/P, B and VFR-B clips; the four 24-frame B oracle streams still match exactly.

### HEVC NAL input foundation

Added independent H.265 NAL-header parsing and budgeted RBSP extraction. hvcC
configuration validation now uses the same header parser. Layer ID and temporal
ID are retained explicitly; unsupported multilayer decoding has a separate guard.
All 65,536 two-byte header patterns are tested, including forbidden bits and zero
TemporalIdPlus1, together with payload escaping, truncation and allocation limits.
No HEVC pixel decoder is claimed by this step. Syntax references: ITU-T H.265
7.3.1 and RFC 7798 section 1.1.4. The official PDF URL currently downloads an HTML login page; the local
/tmp/fvid-h265-standard.pdf is not a usable standard document. Parameter-set
work must obtain the actual specification before relying on that path.

### HEVC profile/tier/level

Added profile_tier_level parsing per H.265 7.3.3: profile space, tier, profile ID,
compatibility flags, all 48 constraint bits, general level, optional sublayer
profiles/levels and reserved-bit validation for the specified profile branches.
Tests parse VPS/SPS from a real x265 64x64 stream, verify identical Main-profile
metadata and exact bit alignment before SPS ID/chroma/geometry fields. Truncated
profiles, reserved bits and invalid sublayer counts are rejected.

A usable 716-page H.265 August 2021 PDF is now at /tmp/fvid-h265-2021.pdf,
downloaded from the Texas Instruments-hosted copy of the ITU document. The earlier
/tmp/fvid-h265-standard.pdf remains an HTML error page and must not be used.

### HEVC base-layer VPS

Added VPS parsing with NAL/layer/temporal validation, profile/tier/level,
sublayer ordering limits, optional timing clock and POC tick ratio, and exact
RBSP termination. Shared ordering parsing infers omitted lower-sublayer values
and validates DPB/reorder bounds and monotonicity. The reserved VPS 16-bit value
is ignored as explicitly required by 7.4.3.1. Tests verify real x265 VPS values,
every truncation, trailing data, memory limits, sublayer inference and invalid
ordering. Multilayer/external base layers, HRD and VPS extensions remain explicit
unsupported cases and must be implemented before claiming complete VPS support.

### HEVC HRD syntax and VPS integration

Added Annex E HRD parsing with common-parameter inheritance, NAL/VCL CPB lists,
sub-picture parameters, delay bit widths, rate/size scale factors and per-sublayer
fixed-rate/low-delay inference. Counts are bounded to seven sublayers and 32 CPBs;
integer syntax bounds are checked. Base-layer VPS now accepts its optional HRD
entry instead of rejecting every nonzero HRD count. Tests cover omitted common
syntax, inferred fields, simultaneous NAL/VCL sub-picture CPBs and truncation.
This parses HRD metadata; it does not simulate conformance buffer fullness.

### HEVC VUI metadata

Added Annex E.2.1 VUI parsing: aspect ratio/extended SAR, overscan, video range
and colour description, chroma location, field flags, display-window offsets,
timing/POC ratio, optional HRD and bitstream restrictions. Values remain typed
and available to future playback integration. Tests parse VUI inside the saved
real x265 SPS at the independently traced bit offset and reach exact SPS trailing
bits; another fixture covers all non-HRD optional fields, legal upper bounds and
truncations. This supplies the VUI component for the pending full SPS parser.

### HEVC SPS base syntax

Added SPS parsing for base-layer geometry, chroma format/separate planes,
conformance cropping, bit depths, POC width, temporal ordering limits,
coding/transform block sizes, hierarchy depths, default scaling-list enablement,
AMP/SAO/PCM flags, explicit short-term sets, long-term references, temporal MVP,
strong intra smoothing and VUI. Exact RBSP termination is checked. Real x265
fixtures cover 8-bit 64x64 and Main10 coded 72x56 cropped to 66x50, including
clock metadata and every truncation of the first fixture. Explicit scaling-list
data, predicted short-term sets and SPS extensions remain unsupported; cross-VPS
and level/geometry-dependent conformance validation is still required.

### HEVC predicted short-term reference sets

Extracted short-term RPS parsing into a shared SPS/slice reader. It now handles
inter-set prediction, slice-local predictor distance, signed POC shifts, inferred
use_delta flags, zero-POC exclusion and normative negative/positive ordering.
SPS parsing uses the shared reader and no longer rejects predicted sets. Tests
cover explicit syntax, POC sign crossing, retained-but-unused references,
non-adjacent slice predictors, DPB capacity, malformed predictor ordering,
truncation and arithmetic overflow. Existing real Main/Main10 SPS tests pass.

### HEVC scaling lists

Added scaling-list decoding and connected it to SPS: disabled lists become flat
16, enabled unsignalled lists use normative intra/inter defaults, signalled lists
support prior-matrix prediction, signed deltas/modulo wrap and separate DC terms.
The factor accessor expands diagonal-scan coefficients to 4/8/16/32-square
matrices, including 4:4:4 32-square chroma inference from the 16-square lists.
Tests cover default tables, explicit ramps, matrix copying, DC replacement,
wraparound, chroma inheritance, invalid indices and truncated syntax. Pixel
inverse quantization is not connected to these matrices yet.

### HEVC PPS base syntax

Added PPS parsing bound to its referenced SPS: dependent slice/output flags,
CABAC/sign hiding, default references, QP/chroma offsets, transform skip and
transquant bypass, tiles/WPP, deblocking controls, optional scaling-list override,
list modification, merge level and slice-header extensions. Uniform and explicit
tile dimensions are resolved to CTU widths/heights with bounds and coverage
checks. Real x265 PPS and all truncations pass the focused regression; tile tests
cover non-divisible uniform geometry, explicit sizes and overflow of the extent.
PPS range/multilayer/3D/SCC extensions are still unsupported. Parsing WPP/tiles
metadata does not yet implement their entropy synchronization or reconstruction.

### HEVC IDR slice headers

Added IDR slice-header parsing connected to SPS/PPS: parameter-set selection,
first/non-first independent CTU addressing, picture output, colour plane, SAO,
QP/chroma offsets, deblocking overrides, cross-slice filtering, entry-point
length syntax and header extension bytes. Byte alignment is validated and the
RBSP/CABAC boundary retained. A real x265 IDR matches trace_headers (QP 33,
SAO luma/chroma true, CABAC at RBSP byte 3). Truncation and malformed alignment
are rejected. Entry-point lengths remain in escaped-NAL units pending substream
translation. Dependent segments and non-IDR/P/B headers remain unsupported;
CABAC pixel decoding is not implemented yet.

### HEVC CABAC initialization and initial context banks

The shared FVid binary arithmetic engine now exposes HEVC context initialization
from initValue (H.265 9.3.2.2). Added HEVC-specific banks for SAO, coding-tree
split, transquant bypass, skip, prediction/partition mode and intra prediction,
with I/P/B table selection and cabac_init swapping. Invalid context selection or
arithmetic error poisons the HEVC wrapper; no external entropy decoder is used.
Tests exhaust all 256 initValue entries across clipped/unclipped QPs and verify
P/B table swapping and unsupported-context rejection. Full native tests pass.
Residual context tables, binarization and coding-tree reconstruction remain due.

### HEVC transform/residual CABAC context banks

Added context banks for transform splitting, luma/chroma CBF, QP delta, transform
skip, independent last-X/last-Y positions, coded coefficient groups, significant
coefficients, greater1/greater2 levels and inter root CBF. Generated normative
initialization values from H.265 (08/2021) tables with exact count checks using
scripts/generate_hevc_cabac_tables.py. Noncontiguous chroma-CBF and significant-
coefficient extension contexts are mapped explicitly into local bank indices.
Tests check I/P/B bank layouts, special indices and independent X/Y adaptation.
Residual binarization/context-increment derivation still needs implementation.

### HEVC last-significant coefficient position

Added residual-bin interface connected to HevcCabac and last-position decoding:
both truncated-unary context prefixes are read before bypass suffixes, context
increments depend on transform size/component, and vertical scanning swaps the
coordinates. Exhaustive scripted tests cover every coordinate of 4/8/16/32-square
blocks, luma/chroma and diagonal/horizontal/vertical scans, validating bin order
against independent normative context-index arrays. All HEVC tests pass. This is
a primitive for the forthcoming full residual block reader, not yet a coefficient
or picture decoder.

### HEVC coefficient significance context selection

Added coded-sub-block and significant-coefficient CABAC context derivation from
the right/bottom group flags, transform size, colour component and scan order.
The 4x4 map, DC position, larger-block offsets and range-extension skip-context
banks are handled explicitly. Out-of-bounds groups and invalid map shapes fail;
outside-picture neighbours contribute zero. Tests cover neighbour combinations,
edge groups, luma/chroma banks, directional scan offsets and inferred 4x4 corner
significance. Full coefficient flag/level traversal is still pending.

### HEVC coefficient remainder and Rice adaptation

Added Main/Main10 coeff_abs_level_remaining decoding: truncated Rice prefix,
EGk escape, bounded bypass reads and checked unsigned result. Non-persistent
RiceState adapts from the preceding absolute level, caps the parameter at four,
resets per coefficient group and commits state only after successful decoding.
Tests round-trip all remainders 0..4095 at each Rice parameter plus large boundary
values, validate adaptation/capping and reject overlong codes/absolute overflow.
Persistent Rice adaptation and extended-precision limited EGk remain separate
range-extension work; full coefficient traversal is still pending.

### HEVC complete residual block traversal

Added Main/Main10 residual block reading into raster-order signed coefficients:
reverse group/scan traversal, implicit last/DC significance, context-coded levels,
bypass signs, sign hiding and Rice remainders. Level contexts carry across skipped
groups, while Rice state resets per group. Scripted tests verify exact syntax/bin
order for a DC remainder, hidden-sign parity and an 8x8 block with three coded
groups plus a skipped group, including inferred DC and cross-group context state.
The caller supplies scan/sign-hiding eligibility and reads transform-skip syntax;
range-extension RDPCM and persistent Rice are outside this reader's scope.
This is coefficient syntax support, not yet HEVC picture decoding; real-bitstream
pixel comparison still requires coding-tree traversal and reconstruction.

### HEVC inverse scaling and residual reconstruction

Added Main/Main10 inverse quantization with SPS/PPS scaling lists, 4/8/16/32 DCT,
intra-luma 4x4 DST, 4x4 transform skip and transquant bypass. Reconstruction uses
wide integer intermediates, normative signed rounding and intermediate clipping;
invalid geometry, coefficient range, matrix index and derived QP fail explicitly.
The caller supplies the derived component QP and selected transform type.
DCT constants are extracted from H.265 (08/2021) equations 8-319/8-321 by
scripts/generate_hevc_transform_tables.py, with dimensions and symmetry checked.
Tests cover all sizes and 8/10-bit DC normalization, DST impulse values, distinct
skip/bypass behavior, nonflat matrices, saturation, and dense 4x4 blocks against
an independent factored transform across the entire supported QP range. All 35
HEVC tests pass. Prediction, coding-tree traversal and loop filters are still
needed before a HEVC picture can be decoded and compared with an oracle.

### HEVC intra prediction and mode derivation

Added all 35 Main/Main10 4:2:0 intra sample modes for 4/8/16/32 blocks, reference
substitution in normative bottom-left-to-top-right order, mode-dependent weak
smoothing, conditional 32x32 strong smoothing, DC/axis boundary correction and
fractional angular interpolation with negative-angle reference extension.
The caller supplies availability after z-scan, slice/tile and constrained-intra
checks; this module does not infer those conditions from picture coordinates.
Luma MPM candidates/remainder mapping and 4:2:0 chroma mode derivation are also
implemented, with separate caller obligations for neighbouring mode availability.
Tests cover exact planar/DC/angular vectors, fractional angles, clipping,
substitution gaps, strict strong-smoothing thresholds, every mode/size/component
on absent references, transposed angular predictions, and all 1,225 neighbouring
mode pairs with exact candidate/remainder complement coverage. Coding-tree
integration and real-picture oracle validation remain pending.

### HEVC intra CABAC mode syntax

Added single/four-part intra luma syntax reading through the existing HEVC CABAC
bin interface. All previous-mode flags precede any MPM indices/remainders, as
required for NxN coding units. MPM indices use truncated unary bypass bins and
remaining modes use five bypass bits. Chroma mode uses one context bin followed
by two bypass bins only for explicit modes. Luma codes resolve through the
existing neighbour-dependent mode derivation after syntax reading. Tests check
mixed NxN bin ordering, every remainder/chroma value and truncated inputs.
This still requires coding-unit traversal before real slice reconstruction.

### HEVC CTU SAO syntax

Added Main/Main10 SAO parameter reading before coding-tree syntax: conditional
left/up merging, shared chroma type and edge class, independent Cb/Cr offsets,
truncated unary magnitudes, inferred edge signs, explicit nonzero band signs,
and band positions. Disabled slices consume no bins. The caller supplies only
merge neighbours permitted by slice-segment/tile boundaries. Tests verify exact
bin consumption for merges, shared chroma edge metadata, maximum 8/10-bit offset
values and zero-sign omission. These are parsed parameters; applying the SAO
pixel filter and whole-picture decoding remain pending.

### HEVC SAO sample filtering

Added SAO sample application and edge-neighbour direction selection. Band lookup
wraps modulo 32, edge classification handles all five sign-sum categories, missing
edge neighbours suppress filtering, and results clip to 8/10-bit sample range.
Parameters and samples are checked before arithmetic. Tests cover categories,
directions, equal neighbours, missing neighbours, band wrap and saturation.
The picture caller must supply deblocked (not already SAO-filtered) neighbours
and enforce tile/slice/PCM/transquant exclusions. Whole-picture filter traversal
and real HEVC decode validation remain pending.

### HEVC coding-quadtree traversal

Added bounded CTU traversal with z-order child visitation, CABAC split context
selection from available left/top CU depths, inferred picture-edge splitting and
implicit minimum-size leaves. The visitor receives each node after its split
decision for quantization-group state reset, and each leaf with the same entropy
stream for CU decoding. Geometry validation and widened coordinates prevent
invalid shifts/overflow. Tests verify context changes from completed siblings,
exact leaf order, fully inferred edge traversal and truncated split input.
The picture-level visitor and transform-tree/CU decoding are still pending.

### HEVC intra transform-tree syntax

Added intra 4:2:0 transform-tree traversal with inferred/explicit splitting,
parent-conditioned chroma CBF reading, depth-specific luma/chroma contexts and
synchronous transform-unit callbacks on the shared CABAC stream. Four 4x4 luma
children retain the parent chroma flags for QP-state decisions, but only the last
child owns/consumes the shared 4x4 chroma residuals. Geometry/configuration checks
bound traversal. Tests cover unsplit context selection, forced NxN splitting,
chroma ownership/origins, exact bin consumption and truncated input. Integration
with CU metadata, residual decoding and picture reconstruction remains pending.

### HEVC residual block decoding pipeline

Connected transform-skip syntax, intra-dependent scan selection, coefficient
reading and inverse reconstruction in hevc_block. The adapter chooses DCT/DST,
skip/bypass, intra/inter scaling-list index and sign-hiding eligibility from block
configuration, validating geometry/depth/QP before consuming bins. Tests follow
scripted syntax through signed pixel residuals and cover scan selection for all
modes, sizes and 4:2:0 components. Whole-picture CU integration remains pending.

### HEVC quantization parameter derivation

Added CABAC delta-QP prefix/EG0 suffix/sign decoding with asymmetric conformance
bounds, neighbour/previous-QP prediction with signed modulo wrapping, and the
4:2:0 chroma QP mapping plus bit-depth offsets. Tests cover every legal 8/10-bit
delta, invalid adjacent values, negative prediction rounding, range wrapping and
chroma mapping plateaus. Picture integration still needs quantization-group
ownership and slice/tile/WPP reset handling before real-frame validation.

### Real HEVC IDR entropy integration

Added an embedded libx265 64x64 gray IDR integration fixture joining SPS/PPS/slice
parsing, CABAC initialization, SAO, coding-quadtree traversal, intra mode syntax,
transform-tree CBFs, QP delta and residual reconstruction. It reaches the real
end-of-slice termination bin and verifies four 32x32 CUs, disabled SAO, and one
32x32 residual block containing -2 throughout. The fixture harness assumes full
intra partitions and simple QP/mode neighbours; it is deliberately a test, not a
general picture decoder. Picture reconstruction and pixel-oracle comparison are
still required before claiming native HEVC playback.

### First real HEVC intra picture reconstruction

Added a budgeted plane builder that tracks decoded samples, gathers available
intra references, applies prediction plus residual with wide signed arithmetic,
clips by bit depth, and rejects overlapping/out-of-bounds block writes. Additional
slice/tile/constrained-intra availability is supplied by the picture caller.
Extended the gray IDR fixture through neighbour mode derivation and reconstruction
of all Y/Cb/Cr samples. The test verifies Y=126 for all 4,096 samples and Cb=Cr=128
for all 2,048 chroma samples. A local FFmpeg oracle independently confirmed exactly
those 6,144 values. SAO is off in this fixture and its uniform edges need no
deblocking changes. General CU/QP state, PCM/NxN paths, loop filters and varied
picture oracles remain required; this fixture is not a production HEVC decoder.

### Detailed HEVC IDR pixel oracle and NxN integration

Extended the fixture decoder to publish per-prediction-block intra modes, derive
NxN sibling MPMs in order, select modes for individual transform blocks and add
the NxN transform-depth increment. Fixed transform-tree validation to allow an
SPS hierarchy-depth limit greater than a particular CU's available depth; minimum
transform size still terminates traversal. Added a generated 32x32 testsrc2 IDR
with loop filters/AQ disabled: 10 CUs, 31 prediction blocks and 47 residual blocks.
All 1,536 reconstructed Y/Cb/Cr samples match the embedded FFmpeg oracle exactly.
Both gray and detailed integration tests pass. Production picture API, general
QP-group state and loop filters still require integration and validation.

### HEVC Main10 detailed picture oracle

Added a 10-bit version of the detailed IDR fixture and generalized the integration
harness to compare u16 samples without downconversion. All 1,536 Main10 Y/Cb/Cr
values match the embedded independent decoder output, including NxN prediction,
coefficient decoding and inverse reconstruction. The 8-bit detailed and gray
fixtures also still pass. Loop filters remain disabled for the detailed fixtures;
these tests do not establish general HEVC stream or playback support.

### Native HEVC IDR picture API

Promoted the verified intra reconstruction path into hevc_picture::decode_idr.
It returns coded Y/Cb/Cr planes with crop/depth metadata, traverses raster CTUs,
shares CABAC state and checks termination against picture extent. Decode memory
is bounded before allocation, parameter fields are validated, and incomplete
pictures fail without publishing output. Current supported scope is a single
4:2:0 Main/Main10 IDR slice with fixed QP, no PCM/tiles/WPP and disabled loop
filters; unsupported tools are rejected explicitly. The 8/10-bit detailed
oracles now exercise the library API, including budget/truncation/tool rejection.
A new four-CTU 64x64 fixture matches all 6,144 oracle values across CTU rows.
General QP groups, loop filters, inter pictures and playback wiring remain due.

### HEVC picture-level SAO integration

The IDR API now reads SAO before every CTU, resolves left/up merges and applies
the filter after all prediction/reconstruction is complete. Planes use unchanged
input neighbours and publish filtered output only on success; picture budget
includes the extra sample buffer. Resolved SAO parameters are retained in raster
CTU order. A four-CTU fixture with nonzero SAO offsets matches all 6,144 oracle
samples; previous 8/10-bit fixtures still pass. SAO with transquant bypass is
rejected until per-CU filter exclusions are integrated. Deblocking, adaptive QP,
other previously unsupported tools and inter pictures remain pending.

### HEVC chroma deblocking primitives

Added normative tC table, 4:2:0 chroma threshold derivation from adjacent luma QPs
and PPS-only chroma offset, bit-depth scaling, and chroma sample-pair filtering.
Per-side PCM/bypass exclusions preserve original inputs for delta derivation;
signed deltas and output samples are clipped independently. Tests cover QP mapping
plateaus, offset limits, 8/10-bit thresholds, signed rounding, clipping and side
exclusions. Luma deblocking and picture-edge traversal remain unimplemented, so
the picture API still rejects streams with enabled deblocking.

### HEVC luma sample deblocking

Added strong three-sample and weak one/two-sample luma filters, strict raw-delta
gating, per-sample change limits, bit-depth clipping and independent side
exclusions. All outputs derive from original p/q values. Tests cover exact strong
and weak outputs, bounded strong updates, disabled sides and threshold equality.
Edge-level beta/tC decisions and picture traversal remain pending; the native
picture decoder continues to reject enabled deblocking until those are connected.

### HEVC luma edge filter decision

Added boundary-strength/QP/offset-derived beta and tC, bit-depth scaling, and the
four-sample segment decision from endpoint curvature. Strong filtering requires
both endpoint lines to pass; weak filtering independently selects second-sample
updates on p/q sides. Tests cover threshold equality, one-endpoint rejection,
independent second-sample decisions and 10-bit scaling. Picture boundary strength
metadata and vertical/horizontal traversal remain to be connected.

### WebM indexing and VP9 header decoding

Added an FVid-owned seekable EBML/WebM reader: track metadata, timestamp scale,
known/unknown Segment and Cluster sizes, SimpleBlock/BlockGroup packet indexing,
signed relative timestamps, and bounded payload reads. Lacing and track content
encoding/encryption are rejected. This is an initial video indexing subset, not
complete Matroska support (track timing transformations and audio remain pending).

Added VP9 superframe framing, stateful uncompressed headers, reference dimensions,
profile/color configuration, loop-filter deltas, quantizers, segmentation, tile
geometry and bounded tile splitting. Malformed header parsing preserves prior
state. The own Boolean arithmetic decoder and compressed-header parser apply
normative probability updates; default probability tables are reproducibly
extracted from the VP9 v0.7 specification by `scripts/generate_vp9_tables.py`.
Probability adaptation from decoded symbols is not implemented. The diagnostic
example supports frame-parallel context refresh and explicitly rejects streams
requiring symbol adaptation.

Local validation on the user-provided WebM: 1280×720 VP9 profile 0, 8-bit 4:2:0,
BT.709 limited range; 14,185 packets/headers, 141 keyframes, 56,740 tile markers.
Every payload, uncompressed header, compressed header and tile boundary passed.
An independent ffprobe packet count agrees. The private media is not a repository
fixture. A separate three-frame synthetic IVF fixture supplies regression checks;
its quantizers, filter levels and header lengths agree with FFmpeg trace_headers.

**WebM playback remains unavailable:** VP9 block syntax, coefficient decoding,
prediction, inverse transforms, pixel filtering and the playback adapter still
need implementation. The player detects EBML instead of incorrectly interpreting
it as MP4, reports the unsupported format and includes the opened path in errors.
No FFmpeg or external-codec fallback was added.

### Native VP9 picture reconstruction and WebM playback

Implemented all ten intra prediction modes, DCT/ADST 4–32 transforms, lossless
WHT, coefficient scans/Pareto token probabilities, dequantization and loop
filtering. The 448-block independent libvpx transform corpus matches exactly.
Added single-reference inter block syntax, neighboring/previous-frame motion
candidates, sub-8x8 motion, high-precision MV syntax and four subpixel filters.
Reference slots and frame-parallel probability contexts are managed in the native
VP9 decoder. Decode errors poison the decoder until reset.

NativeReader now selects WebM/Matroska VP9 from EBML input. The adapter supplies
RGB, source-clock intervals, bounded lookahead, EOF and rewind to the player and
NativeCameraSource. The player file picker accepts webm/mkv; Zed run tasks now
use release mode. No runtime FFmpeg/libvpx dependency was introduced.

Initial local oracle validation: the user's first 100 1280x720 frames match every
YUV byte. Synthetic 10-frame motion/reference-refresh, 10-bit 70x50 and 12-bit
lossless sequences also match independent raw-pixel oracles. The complete private
recording is validated separately; no private media is stored in fixtures.

Limitations: segmentation, compound references, scaled references, decoded-symbol
probability adaptation, 4:2:2/4:4:4, VP8 and audio are not implemented. The
native player reports unsupported tools rather than substituting another decoder.

Full private-recording verification completed: all 14,185 displayed frames,
19,609,344,000 raw yuv420p bytes, match the independent FFmpeg oracle SHA-256:
`5563489ccf22278ec76316bb0970244dd6cf5bdd6fd0a46209987afb08d8faa0`.
Both pipelines exited successfully; raw frames were streamed into SHA-256 without
storing the complete decoded recording. Packet PTS are strictly increasing.
The diagnostic run (including output and SHA-256) took about 607 seconds for
479 seconds of media, so real-time 720p30 is not claimed for this implementation.
The native suite passes 246 tests (213 unit + 33 integration), including the
additional malformed compressed-frame corpus. Player release and camera FFI
checks pass without warnings. `otool -L` on the release player lists only system
frameworks/libraries, with no FFmpeg/libvpx linkage.

## AV1: native picture decoding and WebM playback (2026-09-21)

FVid now reconstructs a tested AV1 subset in safe Rust with no external runtime
codec. The native WebM reader selects `V_AV1`, preserves source timestamps,
converts decoded planes to RGB and resets the decoder on Restart.

Implemented: bounded OBU/frame/tile parsing, normative adaptive CDFs, residual
coefficients and quantization, all AV1 transform sizes and types, intra modes,
CFL/filter-intra, single and average/distance compound prediction, spatial MV
stacks, variable transform trees, local affine warp, reference/CDF refresh,
hidden/show-existing frames, deblocking and CDEF. Reconstruction supports
4:2:0 8/10/12-bit, lossy and lossless; no FFmpeg/libaom/dav1d fallback exists.

Pixel oracles cover all-keyframe sequences, I/P lossless, eight tiled 10-bit
I/P frames, odd frame dimensions, 12-bit lossless, and a default SVT-AV1
24-frame 192x128 random-access sequence including compound/local warp.
Every displayed YUV sample agrees with the independent decoder. Separate
oracles cover inverse transforms and 30,720 adaptive/nonadaptive symbols.
Fixtures and regeneration instructions are in `tests/fixtures/av1/README.md`.

This is **not full AV1 conformance**. Temporal motion fields, global motion,
OBMC, masked compound/inter-intra, palette/intrabc, segmentation, quantization
matrices, restoration, superres/reference scaling, film grain, 4:2:2/4:4:4,
short reference signaling/inter frame IDs, separate FrameHeader/TileGroup OBUs,
and layered operating points still return explicit errors. MP4 AV1 dispatch
and audio remain unimplemented. See `NATIVE_PLAYBACK.md` for playback limits.

Validation: `cargo test --locked --offline --no-default-features` passes all
262 unit/integration tests. The release player build and `fvid-camera-ffi`
check complete without warnings. `otool -L target/release/fvid` lists only
macOS system libraries/frameworks. The release player was launched with the
24-frame AV1 WebM fixture. No full-spec conformance or general real-time claim
is inferred from these tests.


## 2026-09-22: native HEVC playback

The native MP4 source now dispatches `hvc1`/`hev1` to `HevcDecoder`. It owns
POC derivation, short-term reference lists and storage, slice CABAC, spatial and
temporal merge/AMVP, integer/fractional motion compensation, weighted P/B
prediction and I/P/B reconstruction. WPP restores probability states after the
second CTU and translates escaped entry-point byte counts into bounded RBSP
substreams. CU QP deltas, deblocking and SAO now run in the picture pipeline.
CRA seeking suppresses unavailable leading RASL pictures; normal sequential
playback keeps them. PTS reordering, crop, VUI range/matrix and player seeking
reuse the indexed MP4 presentation path. No external HEVC decoder is called.

Main/Main10 4:2:0 fixtures compare every output sample over 17-frame sequences,
rewind and open-GOP seek. The weighted fixture exercises nondefault luma/chroma
weights, temporal MVP and CRA/RASL. A local 886x1920 Main screen recording's first
1,000 frames match the oracle YUV checksums. Unsupported tools are listed in
NATIVE_PLAYBACK.md; this replaces the earlier IDR-only limitation, not a claim
of complete HEVC conformance. `Mp4VideoReader` is the codec-neutral API;
`Mp4AvcReader` remains an alias for source compatibility.

## 2026-09-22: HEVC playback throughput

Motion interpolation now reuses separable intermediate rows and applies taps
across contiguous sample spans. Integer single-reference blocks use row copies.
Inverse DCT uses factored fixed-size kernels and a DC-only path; scaling-list
lookup uses a direct diagonal index. Residual scan tables are shared, and
coefficient-group and boundary-strength scratch arrays avoid per-group allocation.

For pictures of at least 128x96 without constrained-intra prediction, CABAC and
motion metadata run ahead of inverse transforms and pixel reconstruction through
a bounded two-row channel. Reconstruction retains bitstream order, so intra
sample availability and reference-picture publication remain unchanged. Filters
run after reconstruction joins. Small/constrained-intra pictures use the serial
path; malformed late rows close the worker without publishing partial pictures.

On the local 886x1920 recording, the heaviest 10-second packet-size window
starts near 90 seconds (22.1 Mbps). A 1,200-frame decode/conversion run from
that position reached 68.62 fps (14.57 ms/frame); an earlier 600-frame probe
at the same position ran at 56.59 fps. Paced runs at
90, 430 and 480 seconds delivered 60.02–60.06 fps with no decoder waits over
1 ms; startup buffering is excluded. These are workload-specific headless
measurements, not a guarantee for every HEVC stream or display. The first 1,000
frames still match the oracle's visible YUV checksums exactly. Dense-transform,
all-phase edge interpolation and late-row failure regressions are also tested.

Pixel availability reductions and residual additions operate on contiguous
rows; saturating i32 arithmetic preserves the earlier wide-integer clipping at
both integer limits. CABAC short-field reads avoid the general byte-gather loop.
WPP context banks have fixed storage and restart from saved states without
per-row context allocation or initialization. The CABAC arithmetic tests also
cover the shared AVC path.

Validation after the throughput changes: all 276 unit/integration tests pass
without default features; all eight HEVC playback tests also pass with the
player feature. The release player builds without warnings.

The converter now signals a freed stage slot to the decoder immediately,
removing up to 2 ms of polling latency under presentation backpressure. With
that change, a continuous window run from the beginning submitted 7,500 new
frames over 125.025 seconds, including the 90–110-second high-bitrate interval.
Every 300-interval report was 59.98–60.00 fps, with zero overdue empty-queue
polls. A separate window run starting at 90 seconds also stayed in that range.
These counters measure new frames handed to the renderer, not physical display
scanout. Playback remains the native HEVC implementation, without FFmpeg or
VideoToolbox decoding. Two additional threaded regressions cover complete frame
delivery/rewind and dropping a player with both channels full; both pass with
and without the player feature. The eight HEVC playback tests pass again.

## 2026-09-22: VP9 backward probability adaptation

The native decoder now collects per-frame branch counts for transform tokens,
EOB decisions, partition/mode/reference/transform choices and motion vectors.
Counts are shared across tiles and applied to the saved probability context
only after successful picture reconstruction. The compressed-header context
remains immutable while decoding tiles. Intra frames adapt coefficients while
preserving their header-updated skip/transform tables; context refresh and
keyframe/error-resilient reset keep their existing lifetime rules.

The implementation follows VP9 probability adaptation (specification section
8.4), including the previous-keyframe coefficient factor, implicit partition
branches at picture edges, and implied MV high-precision bits. No external
codec was added. Synthetic 8/10-bit fixtures exercise context refresh, multiple
keyframes, reset and two tile columns; every visible sample matches the external
pixel oracle. The reported local 720x1280, 30 fps WebM also decodes completely:
all 453 displayed frames match the independent raw YUV decode byte for byte.
The private recording is not included in the repository. Compound prediction,
segmentation and the other documented VP9 limits remain separate limitations.

Final validation: 282 tests pass without default features; the release player
build completes without warnings. The full local recording's native/oracle YUV
SHA-256 is `a970e17cfb55c313e4eb4ded062677854409b0ef06e5aa0c208ade5a8fe778f2`.

### In-band HEVC PPS updates

The decoder accepts a validated PPS replacement/addition before picture slices,
using an already known SPS. A changed PPS after slices is rejected before picture
publication. Reset restores configuration-record parameter sets. A saved synthetic
length-prefixed access unit (`hevc-pps-update.packet`) changes constrained intra
prediction on the first intra picture from `hevc-multislice-main.mp4`; the regression
reconstructs the mutation using the PPS syntax and compares all decoded planes
to the original picture. This proves the update/reset path, not arbitrary SPS
changes or every PPS tool combination. Tests require no external decoder.

PPS-update verification now includes a saved independent-decoder YUV oracle:
all 24,576 bytes match the unchanged intra picture. Tests also cover a parameter-only
packet followed by its picture, persistent PPS state, and every truncated PPS
prefix with unchanged parameters and reset recovery. The regeneration helper is
`scripts/generate_hevc_pps_oracle.py`, used only for reference fixture generation.

### HEVC in-band SPS sequence changes

The decoder now keeps SPS records and original PPS NALs independently, so a
validated in-band SPS replacement/addition can rebuild PPS-derived geometry.
Updates are validated before installation, and changed parameter sets after
slices are refused. Switching the decoded SPS requires a random-access picture;
it clears reference storage and POC history before reconstruction. Reset restores
the original configuration-record SPS/PPS, including their encoded PPS records.

A synthetic 128x128 I/B/P sequence populates references before switching to a
96x64 I/P/P sequence. All three new pictures match the saved independent YUV
oracle. Tests cover both a separate parameter-only packet and parameters prefixed
to the picture, then reset/repeat; late SPS and non-random-access transitions are
rejected with reset recovery. This verifies native decoder sequence transitions,
not arbitrary non-IRAP SPS changes, unsupported profiles, or MP4 display timing
across a changing sample description.

### Stable camera output across HEVC source-size changes

`NativeCameraSource` now fits cropped RGB directly into the existing fixed BGRA
publication buffer when source dimensions or pixel aspect change. The output
format and its initial sample aspect stay fixed; borders are opaque black. This
uses no extra frame allocation. Unchanged geometry retains the original direct
crop/copy path. A six-frame synthetic HEVC MP4 verifies 128x128 -> 96x64 -> backward
seek, exact fitted pixels, timestamps, crop and anamorphic output. This is native
file-to-camera buffer proof; activation/delivery from the installed macOS camera
extension remains unverified pending valid provisioning profiles.

### Owned ASS subtitle conversion path

`convert-subtitles` now handles a selected `S_TEXT/ASS` Matroska track through the
owned reader and writer, retaining CodecPrivate styling, complete event bytes,
PTS/duration, name and language. Publication remains atomic and refuses an
existing destination. Ordinary no-default-features tests exercise default and
explicit selection through CLI, compare every selected packet and the header,
and verify destination preservation. The same checks pass through the public
`media` API with that feature enabled. Independent FFmpeg ASS extraction of the
source and native output matched byte-for-byte (two cues, including alignment,
line break and comma-containing text). FFmpeg was used only as this reference.
Other subtitle codecs, non-UTF8 text and remaining media operations still retain
legacy paths; this change does not establish whole-feature FFmpeg independence.

### AVC in-band sequence and picture parameter updates

The native AVC decoder accepts repeated SPS/PPS, validates replacements before
installation, and retains encoded PPS records to rebuild geometry against updated
SPS records. Parameter-only access units persist until the following picture.
A changed decoded SPS requires IDR even when the SPS ID is reused; IDR rebuilds
reference storage and POC handling. Reset restores original parameter records.
The synthetic avc3 regression exercises CABAC I/P/P reconstruction before and
after a resolution change, exact saved independent YUV, native RGB playback,
camera fixed-format output and rewind. Late parameter changes and non-IDR
sequence switches fail without publishing a picture and recover after reset.
This does not add AVC multiple-slice reconstruction or custom scaling matrices.

### AVC custom scaling matrix reconstruction

Added an owned raster matrix resolver with normative 4x4/8x8 defaults and SPS/PPS
fallback rules A/B. Matrices are resolved once per picture and shared with row
workers. Intra luma 4x4/8x8/DC, chroma DC/AC, mixed intra blocks in inter pictures,
and P/B residual reconstruction now use their resolved component matrices.
Flat streams retain their previous weights; transform bypass remains unsupported.

Two eight-frame synthetic High/CABAC I/P/B streams with JVT and nonuniform custom
weights match every saved independent-decoder YUV byte, including rewind. The
custom fixture explicitly exercises nonzero intra 8x8 residuals. Resolver tests
cover scan order, standard defaults, SPS/PPS inheritance and invalid weights;
encoded SPS fallback variants and other chroma profiles remain outside this proof.
Reference encoders/decoders run only during fixture generation.

### AVC lossless transform bypass

Connected SPS transform-bypass signaling to reconstruction when luma QP-prime is
zero. Intra 4x4/8x8, full Intra16 and chroma residuals use unscaled coefficients;
horizontal/vertical intra modes accumulate residual DPCM across the complete
prediction block. Inter residuals bypass transforms without intra DPCM. Nonzero
QP still follows the ordinary scaling/transform path. Checked accumulation rejects
numeric overflow. The avcC parser now accepts the profile-244 extension used by
the independently encoded 4:2:0 lossless files. This does not add 4:4:4 reconstruction.

Four synthetic eight-frame QP-prime-0 streams exercise 8/10-bit CABAC and CAVLC, directional
intra residuals and I/P reconstruction. Their saved independent-decoder output
matches the original synthetic YUV exactly; native decoding matches all samples
twice with rewind. The x264 reference encoder disables B pictures in lossless
mode, so encoded lossless B conformance is not proven by these fixtures.
Both ten-bit streams also pass the software NativeReader-to-camera BGRA bridge
with exact pixel preservation, timestamps and backward seek. This checks the
bridge, not macOS extension registration. DPCM/DC tile-boundary arithmetic has
separate unit tests.

### Explicit audio selection through the public decode API

The media decode and planning entrypoints now admit containers with multiple
owned ALAC/AAC tracks before resolving the explicit selection. Previously a
multi-ALAC source fell into the legacy adapter despite an explicit supported
track. A saved synthetic two-track ALAC MP4 verifies both selections against
the owned PCM exporter and checks that omitted selection fails without creating
an output. Fixture generation is separate from ordinary test execution.

The CLI uses the same multi-track admission for `media decode-audio` and its
plan command. Both ALAC selections produce the owned plan and byte-identical
WAV output in builds with and without the optional legacy `media` feature.

### Native camera file repetition

`NativeCameraSource::with_end_behavior(CameraEndBehavior::Loop)` repeats the
native source after discovering its presentation end at EOF. Default `Hold`
behavior remains available. Source selection wraps while published camera ticks
retain their original sequence, host and media timestamps. Tests cover exact
Y4M boundaries and skips, plus all eight 10-bit AVC frames across three cycles.
This is the portable frame bridge; macOS camera registration remains separate.

The C boundary exposes `int32_t fvid_camera_set_loop(CameraSource *handle,
uint32_t enabled)` (0 = Hold, 1 = Loop). Existing open/frame/close signatures
remain unchanged. Calls on a handle must be serialized. Invalid values return
-1 with the thread-local diagnostic and leave the handle usable. The FFI test
checks enable/disable/re-enable on the saved 10-bit AVC source and compares
first-frame pixels at repeated EOF boundaries.

### Multiple AAC tracks and irregular packet timing

The no-legacy CLI test now selects each of two synthetic AAC tracks (44.1 kHz
mono and 48 kHz stereo), comparing plans and WAV with the owned exporter.
A separate saved Matroska-to-MP4 copy reproduces a 1016-sample interior packet
followed by ordinary 1024-sample AAC frames. That file now exports each packet within its container-assigned presentation
window while consuming complete packets for AAC overlap state. The regression
checks 48008 output samples against independently assembled decoded packet
windows and verifies the 10–30 ms interval. Zero/overlong durations and
non-contiguous timestamps remain rejected.
Both fixtures are generated by `scripts/generate_audio_selection_sample.py`;
ordinary tests do not invoke FFmpeg.

### MP4 AAC playback packet windows

`AudioStream::packet_sample_limit` optionally supplies exact container presentation
windows in decoded sample frames. MP4 AAC derives this from duration, rate and
timescale, rejecting zero or non-aligned windows. The playback worker consumes
complete compressed packets before trimming PCM tails; other stream types leave
the limit unspecified. A real worker with a capture backend verifies all 48
synthetic packets: 1016 samples in packet 1 and 1024 in ordinary packets. After the
single media edit, the first packet contributes 16 samples and the last 896,
for 48008 total. Captured playback PCM matches the native export exactly.
The first timestamp is zero; duration, pacing and seek use the edit origin. Single-range AAC media edits now trim priming/padding before device delivery;
complex multi-range and empty edits still require separate scheduling.

The AAC single-edit seek regression verifies that a 50 ms presentation request
lands at source tick 3064 and anchors the device at 42.833333 ms. `seek_to`
returns source ticks for `time_of` to translate once; subtracting the priming
offset in both places caused an incorrect device anchor and is fixed.

For single-range MP4 AAC edits, playback seek rebuilds decoder state from the
beginning and suppresses preroll PCM until the landed packet. Captured PCM after
a 50 ms seek and subsequent backward seek matches continuous decoding exactly.
This currently takes linear decode work; bounded state checkpoints remain a
performance task. Raw `Mp4AudioReader::seek` retains its packet-cursor behavior;
the player-facing `AudioStream::seek_to` performs the preroll. Complex edit lists and non-MP4 AAC containers are not covered by this path.
MP4 AAC without edit lists now uses the same suppressed preroll and has a
separate continuous-PCM seek regression (`aac-no-edit.m4a`).

A threaded preroll-control regression sends Pause from the first access-unit
decode, confirms that no second packet is decoded and no preroll PCM reaches
the backend, then sends Stop and joins the worker. Existing per-packet command
draining provides this interruption boundary; this is not a throughput or
maximum-latency benchmark.

### ADTS AAC playback seek

The ADTS playback reader now primes decoder state from the beginning after seek,
using its exact frame timestamps to suppress all preroll PCM before the landed
frame. A worker regression compares mono AAC output after forward/backward seeks
with continuous decoding byte-for-byte. This currently requires linear decode
work, as in MP4; checkpoints are not yet implemented. Seeking returns the landed
timestamp, while subsequent encoded packets include decoder-only preroll.

### Matroska AAC playback seek and clock

The AAC Matroska playback adapter primes decoder state from the start after seek
and suppresses preceding PCM. Its presentation hook restores nanosecond packet
timestamps instead of interpreting them with the AAC sample-rate clock. A real
worker test compares the post-seek packet timestamps and PCM with continuous
decoding of the saved synthetic stereo Matroska fixture. This does not change
other Matroska codecs or implement codec-delay/discard-padding scheduling.

AAC playback pacing now uses the end of the actual decoded/presented float PCM
(timestamp plus sample count/rate), including packet-tail and edit trimming.
This avoids treating Matroska's unspecified packet duration as a zero-length
AAC frame. The worker regression checks the final 48 kHz frame adds exactly
21.333333 ms to the queue frontier. Other codecs retain their existing pacing.

### 2026-10-01 verification after AAC playback timeline work

At commit `3a8b4d4`, the current checkout passed:

- `cargo test --locked --offline --no-default-features --lib`: 656 passed, 3 ignored.
- `cargo test --locked --offline --no-default-features --features player --lib`: 1187 passed, 24 ignored.
- `cargo build --locked --offline --no-default-features --features player --bin fvid`.

The normal dependency tree for this player configuration contains no `fvid-media`
or FFmpeg/libav dependency; `otool -L target/debug/fvid` shows no libav linkage.
macOS VideoToolbox/Metal frameworks remain linked for platform playback paths;
this linkage check does not prove that every codec stream is software-supported.
The tests above include software codec fixtures but omit ignored reference tests.
Other concurrent checkout edits were present during these runs, so these totals
verify the tested checkout, not an isolated clean release tree.

`systemextensionsctl list` still does not list FVid. The installed
`/Applications/FVid Camera.app` and its extension contain no embedded provisioning
profiles. The portable camera bridge is tested; OS registration and cross-process
camera delivery remain unverified. The optional legacy `media` feature still
contains unmigrated FFmpeg operations and is outside the no-libav build claim.

### Standalone ASS conversion through owned APIs

UTF-8 `.ass` files with the canonical Events field format now convert to
Matroska through the owned parser/writer in both CLI and media API. Script/style
header lines, layer, style/name/margins/effect, comma-containing text and override
commands are preserved. Dialogue start/end centiseconds become packet timing;
read-order numbers preserve original script order while packets are sorted by
start time. Tests cover overlapping/nonchronological cues, header and payload
bytes, CLI/API equality, existing-output protection and invalid timing with no
output. Noncanonical Events layouts and non-UTF8 files retain the legacy adapter
before publication; SSA and arbitrary field layouts are not covered here.

ASS Events declarations may now reorder the ten supported field names, with
Text last so commas remain unambiguous. The parser maps each field to canonical
Matroska packet order and normalizes the Events Format header accordingly; styles
and text are preserved. Duplicate/missing fields and Text in a nonfinal position
retain the legacy path before publication. A reordered synthetic cue verifies
layer, actor/style/margins, text overrides and exact timing.

### AVC raster-ordered multi-slice reconstruction

The saved eight-frame 128x96 CABAC I/P/B stream contains two slices per access
unit. Native reconstruction now decodes every frame and matches all saved YUV
samples exactly, including rewind. The acceptance gate in `tests/avc_multislice.rs`
is enabled. Preparation checks picture identity, reference marking and ordered
macroblock ranges before POC/DPB changes. Entropy contexts and neighbour
availability restart per slice, while planes and motion storage belong to the
whole picture. Intra deblocking uses each slice's settings and boundary policy.

Inter reconstruction resolves L0/L1 separately for each slice through the DPB,
including its own co-located B-direct reference. Motion snapshots use each
slice's stable picture identities. Spatial/temporal selection is slice-local,
tested with distinct retained vectors in one picture. A two-slice P-skip test
selects different reference pictures, verifies both output halves and retained
identities. A hand-authored three-frame AVC stream now exercises different encoded L0
list modifications: its two P-slices select different short-term pictures.
All planar samples match a saved independent decode, including rewind. FMO/ASO and mixed slice types
remain unsupported. Fixtures are synthetic; ordinary tests require neither
FFmpeg nor network.

The multi-slice oracle matrix also covers CAVLC 8-bit and CABAC/CAVLC 10-bit
4:2:0 I/P/B streams: eight frames each, exact planar YUV and rewind. The native
camera bridge regression publishes all eight frames for three cycles, then seeks
back, checking BGRA pixels and preserved camera tick metadata. This does not
prove OS camera registration, which still needs valid provisioning profiles.

A four-frame hand-authored Main-profile temporal-direct fixture exposes an
unstable independent oracle when co-located slices use different local L0
mappings. FFmpeg's default threading yields B luma halves (180,130), while
`-threads 1` yields (130,80). Native yields (180,80), matching the explicitly
constructed samples from H.264 8.4.1.2.3's per-co-located-macroblock reference
identity mapping. Tests preserve both external outputs and verify all native
samples against the constructed expected picture. JM 19.0 from the official HHI archive independently produced all four
frames matching native output exactly. The acceptance gate now compares the
saved JM YUV and is enabled; both conflicting FFmpeg outputs remain fixtures
for an oracle-instability regression. VideoToolbox
initialisation failed, so the attempted hardware oracle was a software fallback
and is not separate evidence. Oracle generation uses raw AVC with passthrough
frame timing, since synthetic MP4 timestamps otherwise drop/duplicate frames.
Specification: https://hlevkin.com/hlevkin/Standards/H.264-201602-LatestStandards.pdf
Implementation investigated: https://raw.githubusercontent.com/FFmpeg/FFmpeg/master/libavcodec/h264_direct.c

JM archive: https://iphome.hhi.de/suehring/tml/download/jm19.0.zip
SHA256: `5a87ec1b112423748897fb771f249ac6b7cc8a50c3b56350275857346eed9e1f`.
Built only the reference decoder with `make -j4 CC=clang` in its `ldecod`
directory. Generation can reproduce the saved JM output with
`python3 scripts/generate_avc_slice_lists_sample.py --temporal-direct --jm-decoder /path/to/JM/bin/ldecod.exe`.
JM is used only to create the test oracle and is not linked into FVid.

Validation of the current shared checkout after the multi-slice implementation:
core library 660 passed / 3 ignored; player library 1191 passed / 24 ignored;
multi-slice integration 9 passed / 0 ignored. Existing bypass/scaling/parameter
fixtures also passed (12 tests). Checks used `--locked --offline
--no-default-features`; player tests additionally used `--features player`.
This checkout also contains concurrent playback edits, so these totals are not
an isolated release audit. The player normal dependency tree contains
`fvid-media-info` but no `fvid-media`, FFmpeg or libav dependency.

### MP4 AAC multi-range playback scheduling

The playback worker now accepts bounded encoded, ready-PCM and decoder-reset
steps. Complex AAC edits use the same cumulative sample timeline as PCM export:
empty edits emit silence in at most 1024-frame blocks; each media range resets
codec history and replays suppressed preroll before output. Presentation seek
locates the requested range (including repeated ranges and gaps), resets codec
state and trims output to the requested sample. A synthetic 220 ms gap/repeat
fixture matches all PCM bytes from owned export, then matches continuous tails
after six seeks (gap, range start, interior ranges, backwards and EOF), plus
rewind. Timestamp conversion uses integer nanoseconds, avoiding a floating-point
1 ns anchor discrepancy. Focused verification: 13 worker and 10 MP4 audio tests
passed. Seeking/repeating still decodes linearly from source start; no AAC state
checkpoint optimization is claimed. Integration checks passed: core 661 tests (3 ignored), player 1196 tests
(24 ignored), with zero failures. These totals cover this concurrent checkout;
the AAC commit excludes unrelated video playback changes.

The AAC edit scheduler also has a nonzero-media-start fixture: each repeated
100 ms range starts at source sample 4800 after 20 ms of silence. Its PCM is
verified against samples 4800..9600 from a separately decoded continuous
no-edit source, then playback/seek/rewind are compared against that output.
This checks actual overlap preroll and source selection rather than only
agreement between two consumers of the same edit evaluator.

### Mixed AVC P/B slice types in one picture

Resolved inter reconstruction now selects P/B reference counts, direct prediction
and weighted prediction per slice instead of inheriting the first slice's type.
Two short hand-authored 32x16 streams cover P/B and B/P raster order. Both fail
before this change with `invalid resolved inter slice contexts`; acceptance
checks confirm the last access unit actually contains both types, compare every
YUV byte for all four pictures with independent JM 19 output, and repeat after
rewind. Generation uses `generate_avc_slice_lists_sample.py --mixed-pb` or
`--mixed-bp`, optionally `--jm-decoder /path/to/JM/bin/ldecod.exe` (the same
verified JM archive described above). Neither FFmpeg nor JM is used by tests or
production. Both saved JM outputs SHA256:
`ce18deee9aa42806a99282f54887fbf72b236164190ca60f7a5c65ecced07ada`.
Mixed I/inter slices, FMO/ASO and additional chroma/profiles remain incomplete.
Verification: 11 integration and 13 AVC inter unit tests passed.

### Mixed AVC I/P slice reconstruction

Access units whose first slice is I now select intra-only reconstruction only
when every slice is I. Mixed I/P pictures use shared inter reconstruction, with
independent existing I CAVLC/CABAC entropy readers and empty reference lists for
I slices. Their intra blocks retain slice-local availability, motion identity
and deblocking metadata. Synthetic I/P and P/I streams reproduce the previous
`intra reconstruction requires I slices` / resolved-context refusal and now
match every JM 19 YUV byte twice through playback/rewind. The generator options
are `--mixed-ip` and `--mixed-pi`; these fixtures use monotonically increasing
POC to avoid the mux oracle's inferred timestamp mismatch for reordered I/P.
The synthetic examples cover CAVLC PCM I blocks; additional CABAC mixed-slice
oracle coverage, I/B combinations and FMO/ASO remain to be completed.

Mixed I/B and B/I coverage now exercises the same shared reconstruction with
an I PCM slice and a temporal-direct B slice. Both four-frame CAVLC fixtures
match JM 19 for every plane byte before and after rewind. Generate using
`--mixed-ib` / `--mixed-bi` plus the optional independent JM decoder argument.
These are acceptance tests (no refusal or ignored expectation). They extend
the six mixed-type raster-order fixtures; CABAC mixed-type oracle coverage
still remains outstanding.

### Native decode latency baseline (2026-10-01)

`cargo build --locked --offline --release --no-default-features --example
native_decode_bench` adds a no-output reader benchmark. Invocation is
`native_decode_bench INPUT raw|rgb [FRAME_LIMIT] [software|auto]`. It reports
frame timing percentiles and the count exceeding 16.67 ms; raw mode hands back
the owned decoded frame, RGB includes CPU conversion. Neither mode measures GPU
render, device presentation pacing or end-to-end camera delivery.

On Apple M4 Max, a user-provided 886x1920 HEVC file (no media content committed),
two sequential 600-frame software runs measured:

| Mode | Throughput fps | p50 ms | p95 ms | p99 ms | Frames >16.67 ms |
| --- | ---: | ---: | ---: | ---: | ---: |
| Raw decode | 107.56 | 4.74 | 36.30 | 50.22 | 153/600 |
| Decode + CPU RGB | 105.83 | 5.36 | 33.97 | 46.99 | 152/600 |

The reader has enough average throughput for 60 fps on this sample, but decode
latency tails exceed a frame budget. These measurements do not prove smooth
60 fps playback; render/presentation instrumentation and queue behavior still
need verification. Small committed VP9/HEVC fixtures also ran successfully,
but their throughput is not representative of this target resolution.

### Exact native AAC packet-boundary checkpoints

The owned AAC decoder can save and restore opaque packet-boundary checkpoints,
including all channel overlap/window state, perceptual-noise state, configured
PCE/layout and ASC. Restoration rejects incompatible configurations before any
state mutation. The caller must bind checkpoints to its source/packet cursor;
a matching codec configuration alone does not identify a stream. A regression
captures state after three packets, decodes the remaining synthetic AAC fixture,
resets and restores, then compares every resulting float bit with continuous
PCM. Incompatible-config restoration is also checked for no mutation.
All seven native AAC tests passed. This is decoder support for seek acceleration;
no player seek-cache integration or accelerated seek latency is claimed yet.

### Repeated AAC export ranges reuse exact decoder state

MP4 audio export keeps one AAC packet-boundary checkpoint near the selected
source range. Later ranges at or after that boundary restore overlap/noise and
resume at its packet index; earlier ranges reset and preroll normally. The
checkpoint also retains the expected next source timestamp, preserving the
contiguity check. State is local to one source/track and retained memory does
not grow with the edit count. Non-AAC packet decoders keep their existing path.
The nonzero-start synthetic gap/repeat fixture produces identical continuous
PCM slices while requiring 16 decoded packets instead of 20. All 15 AAC export
tests and both player gap/repeat seek/rewind regressions passed. This accelerates
repeated export ranges; player seek still awaits its own checkpoint integration.

### Keep fvid-media; isolate its temporary libav backend

`fvid-media` remains the project's media layer. The migration removes FFmpeg
execution/linkage, not this crate. Its previous implementation is retained
unchanged in `src/legacy.rs`, enabled by the explicit `legacy-ffmpeg` feature.
That feature is currently default to preserve all existing callers and editing
operations; player/cuda-hw explicitly select it while their migration continues.
The build script and optional bindgen dependency run only with this feature.
Without default features, the same crate exports FVid-owned media/control
contracts and builds without any FFmpeg header/library discovery. This is an
incremental dependency boundary, not completion of native operation migration.
Owned codec/filter/export implementations still reside in the root crate and
must move behind the media layer without introducing a dependency cycle.

Validation: standalone `fvid-media --no-default-features` check passed with
`FVID_FFMPEG_PREFIX=/nonexistent`; normal/build dependency tree has no bindgen,
FFmpeg or libav. Root `--no-default-features --features media` check also passed,
confirming compatibility of the retained legacy API. Production `media` still
links libav through its default backend; no overall independence is claimed.

### Owned WAVE headers now live in fvid-media

The existing sample-preserving WAVE extensible f32 header and default speaker
mask implementation moved into public `fvid_media::owned_wav`. Root audio export
now delegates to this media-layer implementation; no duplicate algorithm or
legacy muxer is used. The root depends on `fvid-media` with default features off;
its `media` feature explicitly enables `legacy-ffmpeg`. Player enables only the
media UI feature, which no longer implicitly selects libav (combined media +
player still supplies the prior legacy UI). This keeps the crate in the native
production path while continuing incremental backend replacement.
Validation: all 15 native AAC/WAVE integration tests passed, root player and
legacy-media checks passed. The player's bindgen dependency is from CoreAudio,
not the media backend. Remaining root-native implementations still await transfer.

### AAC synthesis and IMDCT moved into fvid-media

Owned AAC long/short sine/KBD windows, overlap state and the chirp/FFT IMDCT now
live in `fvid_media::owned_aac` (unsafe code forbidden). Root codec module paths
re-export this implementation; packet parsing calls it directly, with typed
media-layer validation errors converted into FVid's existing invalid errors.
The algorithms and mathematical regressions moved intact, without copying or
foreign decoder linkage. Nine transform/window tests passed in standalone media
without default features; seven full native AAC tests and 17 AAC export/960-frame
integration tests passed. Combined player + legacy media check passed.
Linux/Windows CI explicitly runs the standalone owned-media tests so dependency
unit tests are not lost from root-only `cargo test` coverage.
The native dependency guard now examines activated media features rather than
rejecting the project crate's name. Headless/player/camera graphs passed, and a
positive legacy-media check proved `legacy-ffmpeg` detection. Camera bridge lock
metadata was updated offline for the new native media dependency.

### HEVC reconstruction avoids redundant serialized workers

HEVC row workers wait for every preceding row and hold a shared sample-plane
mutex while reconstructing. Multiple consumers therefore cannot reconstruct
concurrently. A single reconstruction worker still overlaps with the parser,
while avoiding many per-picture threads and row notification contention. No
prediction, transforms, filtering or decoded-byte algorithm was changed.
All 25 HEVC playback/multislice/parameter-update integration tests passed,
including saved independent planes, resets and malformed input handling.

The same M4 Max / 886x1920 target / first 600 software frames measured after this
change (sequential raw and CPU RGB runs): raw 153.41 fps, p50 2.65 ms, p95 25.05 ms,
p99 35.55 ms, 122/600 frames over 16.67 ms; CPU RGB 139.83 fps, p50 3.33 ms,
p95 25.53 ms, p99 37.53 ms, 136/600 over budget. Earlier measurements were
107.56/105.83 fps and p95 36.30/33.97 ms. These are local observations in a shared
machine environment, not a fully isolated A/B benchmark or proof of smooth
60 fps GPU presentation. Latency tails/render/queue verification remain pending.

### MP4 AAC player seek restores packet-boundary checkpoints

The audio worker now retains up to 32 opaque decoder states, one per source-clock
second. Streams opt in with a source preroll target and exact packet-boundary
resume; other codecs/containers keep their existing behavior. MP4 AAC opts in
for ordinary edits and complex gap/repeated-range timelines. Seek/reset restores
the nearest earlier retained state and packet cursor while preserving the
presentation floor; complex range-reset steps repeat this restoration after
selecting the new segment. Cache is scoped to the worker's fixed stream and
retains no PCM. Uncached/evicted earlier regions still preroll from source start.

A synthetic two-second AAC/video fixture verifies warm seek to 1.7 seconds:
48 encoded steps versus 95 for full decoding, identical float-PCM tail, then
identical complete PCM on backwards seek to zero. Existing silence/repeat/
nonzero source range seek and rewind tests remain acceptance tests. Full core
653 passed (3 ignored), full player 1189 passed (24 ignored), no failures.
These totals exclude nine transform/synthesis tests moved into standalone
fvid-media CI. ADTS/Matroska checkpoint-aware cursor support remains pending;
no cold-seek acceleration or external-file latency claim is made here.

### AAC checkpoint seek covers ADTS and Matroska

ADTS opts into the worker cache with exact indexed sample timestamps. Matroska
AAC opts in with exact track packet/nanosecond timestamps; other Matroska codecs
retain their existing seek path. Resuming does not erase the presentation floor.
Two new short synthetic fixtures remux the committed two-second AAC/video test
source into ADTS and Matroska, using a separate generation script
`generate_aac_seek_checkpoint_samples.py` (FFmpeg copy-only oracle/remux tool).
No source from private files or test-time FFmpeg is involved. Warm seek to 1.7 s
must decode fewer than 75% of initial encoded steps, produce identical complete
PCM packet tails/timestamps, then reproduce all initial packets on backward seek
to zero. Both container acceptance checks passed, along with all 15 audio worker
regressions. Cold and evicted regions still use source-start preroll; persistent
checkpoint storage is not implemented.

### Owned AAC independent coupling (after IMDCT)

AAC-LC CCE parsing now supports independent coupling declared in PCE, tagged
SCE/CPE targets, shared/left/right/separate channel selection and scalar gains.
CCE spectral tools run in raw-element order to preserve noise history; TNS and
independent synthesis have retained tag-local overlap/window state. Coupling is
added to target PCM after synthesis. Reset/checkpoints include CCE synthesis
state. All state commits remain after validation of the entire packet and all
coupling targets; absent target/tag/configuration changes are explicit errors.
At this stage dependent coupling before/after TNS and height layouts remained
unsupported; see the dependent-coupling update below.

Hand-authored six-packet fixtures cover silent mono SCE receiving nonzero CCE,
and stereo CPE with separate left/right gains (right 2^(-4/8)). Saved independent
FFmpeg PCM is only an oracle, with peak acceptance tolerance 1e-7; all reset and
checkpoint replay samples are bit-identical to continuous owned decode. The
previous PCE implementation was run against this fixture and reproduced the
specific `AAC PCE coupling or height layout is not implemented` refusal. A third
synthetic missing-target fixture tests late failure rollback of overlap/noise.
Generation: `generate_aac_coupling_sample.py`, `--stereo`, `--missing-target`.
Ordinary tests do not invoke FFmpeg. Reference syntax/application ordering was
checked against the primary decoder source (not linked or used as fallback):
https://ffmpeg.org/doxygen/2.7/libavcodec_2aacdec_8c_source.html
Validation: 51 AAC unit tests, 15 export tests, 2 AAC/960 tests and both coupling
acceptance/rollback tests passed. Further dependent, short-window/960 coupling
and additional target selection fixtures remain to be added.

### AAC TNS ownership in fvid-media

The TNS spectral filter and its band-clipping/direction impulse test now live
in `fvid-media::owned_aac::aac_tns`, alongside IMDCT and window synthesis.
The root crate retains side-information parsing and converts media-layer errors
at the decoder boundary. Spectral filtering is unchanged; this migration adds
no codec capability and keeps `fvid-media` as the owned media layer.

Validation: 10 media-layer tests without default features, 3 TNS parser tests,
19 AAC export/960/coupling integration tests, player compilation and native
dependency checks for headless/player/camera bridge passed without FFmpeg.
The full AAC unit suite before relocating the impulse test also passed (51).

### AAC independent coupling target and short-window coverage

Hand-authored stereo fixtures now cover CPE ch_select 0 (shared), 1 (right),
2 (left), and the existing 3 (separate gains). All saved PCM oracles match
within peak error 1e-7; routing assertions additionally require exactly zero
PCM in the unselected channel and identical channels for shared gain.
A separate six-packet stereo fixture follows LongStart, four EightShort
frames (one eight-window group), then LongStop. It verifies coupling spectral
ordering, overlap history and checkpoint/reset replay against saved PCM.
These are acceptance tests of the existing independent coupling path, not
new support for dependent coupling or SBR.

Generate with `scripts/generate_aac_coupling_sample.py --stereo --selection
shared|right|left` or `--stereo --short`. Oracle generation uses FFmpeg;
ordinary tests read checked-in bytes only. Existing mono/stereo/missing-target
fixtures regenerate byte-identically. All three coupling tests passed across
six oracle fixtures, including bit-exact checkpoint tails and full reset replay.

### Owned AAC-LC dependent spectral coupling

CCE parsing now accepts before-TNS and between-TNS/IMDCT coupling points,
common gains and differential per-band gains (including signed gains). The
CCE spectrum is reconstructed and TNS-filtered in raw element order; targets
retain their side information until all elements are validated. Dependent
mixing is applied to the target spectrum before or after its TNS as declared,
then normal channel synthesis runs. Dependent CCE does not create an IMDCT
overlap state. Missing targets, window-sequence mismatch and nonfinite gains
reject the packet without committing noise or overlap history.

Synthetic stereo fixtures cover both points with active target TNS, long and
eight-short windows, common L/R gains and a negative differential band gain.
Saved PCM tolerance is 1e-7. The before/after oracle difference exceeds 1e-5,
so the fixtures exercise the mixing stage rather than an inactive TNS path.
A missing-target dependent fixture checks rollback after a warmed decoder.
The previous PCE implementation reproduces exactly the dependent-coupling
refusal on the new fixture; ordinary tests enable acceptance with this fix.
Generate with `generate_aac_coupling_sample.py --stereo --point before-tns`
or `after-tns`, optionally `--short` or `--band-gain --signed-gain`.
FFmpeg is used only to save generation-time oracle PCM.
Height layouts, HE-AAC/SBR and broader coupling combinations remain unfinished.
Validation: 51 AAC unit tests, 4 coupling acceptance/rollback/fixture-quality
tests, 15 AAC export tests and 2 AAC/960 tests passed; player compilation
and native headless/player/camera dependency checks passed. Thirteen existing
coupling fixture/oracle files regenerate byte-identically.

### AAC multiband coupling and media-layer spectral mixing

Dependent spectral mixing now belongs to `fvid-media::owned_aac::aac_coupling`.
The root syntax layer supplies band offsets, grouping and parsed gains. The
media function checks finite samples/gains and geometry, and commits a mixed
spectrum only after all bins succeed, preserving the destination on overflow
or invalid geometry.

Three new saved-oracle acceptance fixtures exercise active bands separated by
a zero codebook band, signed cumulative gain deltas, and two four-window groups
in EightShort frames. Syntax assertions verify book layout, exact gains and
END placement; full decode/reset/checkpoint replay matches PCM within 1e-7.
Generate with `--stereo --point after-tns --band-gain --signed-gain --multiband`,
optionally `--short` and `--split-groups`. Validation: five coupling tests
covering fourteen oracle fixtures and eleven no-default-feature media-layer
tests passed. All 24 previously tracked coupling fixture/oracle files
regenerated byte-identically. No new codec profile or SBR support is implied.

### HEVC long-term software acceptance (2026-10-06)

The native software decoder now accepts long-term slice RPS entries: retained
POCs are resolved using explicit MSB cycles or unambiguous LSB matching,
long-term pictures are appended to L0/L1, and collocated/spatial motion
prediction carries short/long classification. Slices must agree on both
retained POCs and their classification.

`hevc-long-term-rext8.mp4` is an owned 64×64 I/P/P synthetic fixture with used
explicit long-term entries. Before enabling decoding it reproduced the exact
long-term motion refusal at the second picture. Acceptance checks every YUV
byte against independent HM 18.0 output for all three pictures and after reset.
Ordinary tests use committed bytes and require neither HM nor FFmpeg.

This evidence covers the contained low-delay 8-bit RExt stream. The NVDEC
adapter now fills IsLongTerm, RefPicSetLtCurr, NumPocLtCurr and NumPocTotalCurr;
its owned scheduler retains the resolved long-term slots. Synthetic tests
verify driver fields, classification/missing-slot errors, retained POCs and
abort-without-commit behavior. These tests do not call an NVIDIA driver.

`owned_hevc_long_term_submits_and_maps_on_nvidia` is an explicitly ignored
physical-device test on Linux/Windows. It submits all three pictures and checks
map/unmap and POC/output against software. Physical execution remains unproven;
it is separate from the software's exact HM pixel comparison. Broader mixed/B
compressed long-term fixtures also remain necessary. Codec coverage is not
declared complete, and FVid versus FFmpeg benchmarks remain after acceptance.

### Mixed and LSB-only HEVC long-term acceptance (2026-10-06)

Owned three-picture variants now qualify LSB-only resolution without MSB
cycles and a mixed short-term/long-term current set against independent HM
18.0 pixels, including reset. The mixed picture carries two used entries but
one active L0 entry; it exercises current-set construction and short-term
motion alongside the separate all-long-term motion fixture. VPS/SPS explicitly
admit its three-picture DPB. This does not qualify all-active mixed L0/L1
B-picture motion, ambiguous LSB cycles, or every profile.

NVDEC synthetic tests cover both current-set arrays/counts, classification
flags and live slot retention for these variants. Physical GPU behavior still
requires device qualification. Fixture generation remains explicit and
separate from tests, uses no FFmpeg, and refuses concealed HM reference losses.

### Persistent HEVC DPB classification (2026-10-06)

Fixed a real state bug: after a picture marked a retained reference long-term,
the software DPB previously kept its original short-term flag. A subsequent
short-term RPS lookup could therefore reuse a long-term picture, contrary to
H.265 8.3.2's short-term-only lookup. The stored DPB now preserves the resolved
classification; such invalid requests refuse instead of publishing a picture.
The owned NVDEC scheduler separately commits long-term POCs with its reference
slots and retains the old classification when a pending submission is dropped.

The new owned invalid-long-to-short fixture failed the refusal expectation
before the fix because decoding succeeded. It now verifies the exact error,
reset recovery and NVDEC scheduler rollback. This refusal is not new valid
codec-profile support. Existing valid explicit, LSB-only and mixed long-term
fixtures continue to require exact independent HM pixels and reset replay.

### SPS-selected HEVC long-term acceptance (2026-10-06)

An owned 64×64 I/P/P fixture now selects two different long-term entries from
SPS rather than encoding their POC explicitly in slices. Software acceptance
checks table contents, selected LSBs and exact independent HM 18.0 pixels
before/after reset. NVDEC tests include its table count, current-set translation
and scheduler retention. The ignored Linux/Windows physical-device test covers
all four valid long-term variants; execution still requires an NVIDIA device.
This closes a verification gap for SPS selection, not all HEVC profiles or
all-active mixed/B-picture motion.

### Active mixed L0/L1 HEVC B-picture acceptance (2026-10-06)

An owned HM-encoded Main8 I/B/B/B control and two long-term variants now
exercise two active references in both lists. The nearest picture remains
short-term and the older picture becomes long-term; one variant reverses L1
via PPS/slice list modification. Independent HM pixels and decoder-reset
replay qualify the complete compressed-picture path. Actual motion-map
inspection requires both reference types in L0/L1 for the normal mixed stream
and long-term L1 motion for the permuted stream. The deterministic final
source frame is a blend to elicit bi-prediction rather than merely advertise
active list entries.

This proves the contained low-delay Main8 B cases. Reordered B pictures with
future references, additional profiles/tools and physical NVDEC execution
remain separate requirements; overall codec coverage is not complete.

### Reordered mixed HEVC B-picture acceptance (2026-10-06)

Owned I/P/B fixtures now decode in order 0,2,1. The mixed B uses short-term
future POC two and long-term past POC zero, with two active L0/L1 entries.
Acceptance verifies actual future-reference motion, exact independent HM
pixels in presentation order, decoder reset, and signed composition clocks
DTS 0,1,2 / PTS 0,2,1. Fixture-only MP4 muxing accepts an explicit dense
presentation-order permutation; omitted order preserves existing fixtures.
NVDEC scheduling and the ignored physical-device test include both variants.
This qualifies the contained Main8 reordered cases, not all profiles/tools,
all player seeks or physical GPU decoding.

### HEVC chroma-QP list syntax foundation (2026-10-06)

PPS range-extension parsing now separates cross-component prediction refusal
from chroma-QP lists and reads their bounded group depth and up to six signed
Cb/Cr pairs. Chroma-less/separate-plane lists refuse. Slice headers expose the
CU-adjustment enable flag and maintain correct entropy alignment. The owned
one-picture RExt fixture reproduces the former PPS refusal and now checks
exact depth zero, entry [6,6] and active slice enablement.

Active picture reconstruction still refuses with the precise CU-selection
error; NVDEC configuration explicitly refuses unqualified lists. Bounds and
refusal tests pass, but they are not acceptance evidence. The independent HM
pixel/reset acceptance remains ignored pending CABAC flag/index decoding,
CU-group state/reset and chroma-QP application. This stage does not close the
chroma-QP tool gap or the overall codec objective.

### HEVC CU chroma-QP selection acceptance (2026-10-06)

Software decoding now initializes the flag/index CABAC banks per H.265 tables
9-34/35, decodes bounded truncated-unary indices, resets coding state at the
configured chroma quantization group, and applies the selected Cb/Cr offsets
before inverse scaling. Slice-segment adjustments initialize to zero; bypass
and absent chroma coefficients do not consume selection syntax. Base/slice
and CU offset bounds remain independently enforced. Chroma deblocking retains
PPS-only offsets as required by 8.7.2.5 (CU/slice adjustments are excluded).

The previous active-selection refusal test is now acceptance. Saved HM pixels
match for the original I picture and a new three-picture I/B/B depth-one group
fixture with SAO/deblocking enabled, including decoder reset. Unit tests cover
all list sizes and indices, zero flag, truncated bins and combined offset
bounds. Ordinary tests use committed bytes without external tools. NVDEC
lists still explicitly refuse as unqualified; these contained 4:2:0 RExt8 cases
do not qualify every profile, dependent segment, WPP or physical GPU path.

Local offline validation for this change: codec library 348 passed / one
explicit fixture-generator test ignored; media library without default
features 334 passed / one ignored; CUDA-feature HEVC admission/scheduler tests
15 passed. This is CPU-side CUDA configuration evidence, not device decoding.

### HEVC chroma-QP entropy paths and high-depth qualification (2026-10-06)

Five additional owned HM-oracle streams cover WPP (two entropy substreams),
four independent slice segments, four dependent segments, and RExt10/RExt12.
Each has three I/B/B pictures with depth-one chroma-QP groups and enabled
SAO/deblocking. Metadata tests require the actual configured group depth,
active chroma selection, WPP/dependent PPS flags, segment addresses/counts
and substream counts; the names alone are not qualification evidence.

All samples match independent HM output on both original decode and reset
replay. High-depth tests compare little-endian 16-bit samples without truncating
them to eight bits. All fourteen MP4/YUV files regenerate byte identically
using explicit HM generation. This extends the software acceptance evidence;
NVDEC chroma lists and other HEVC tools/chroma formats remain incomplete.

Offline codec suite after this expansion: 350 passed, no failures, one
explicit fixture-generation test ignored (25.38 seconds).

### Owned HEVC 4:2:0 PCM reconstruction (2026-10-06)

The former picture-level PCM refusal is replaced by per-CU PCM flag decoding,
zero alignment validation, bounded raw Y/Cb/Cr reads and arithmetic-engine
restart preserving probability/Rice contexts. PCM sample depths are scaled to
sequence depths before reconstruction. PCM writes participate in the existing
row-worker command stream, allowing later intra blocks to observe their pixels.
Metadata marks PCM cells (intra mode DC); deblocking/SAO exclude PCM samples
when the SPS requests filter exclusion. Ordinary non-PCM blocks retain their
existing QP, residual and reference paths.

Ten owned one-picture streams cover all-PCM and mixed PCM/normal blocks,
8/10/12-bit raw samples, 8-bit PCM inside 10/12-bit sequences, filter policy,
WPP, dependent segments and 128x96 parallel reconstruction. Saved HM samples
match on reset replay. Test-only PCM sample counts require actual PCM coding
(and actual normal blocks in mixed cases), not merely an SPS enable flag.
CABAC tests cover raw-plane order, depth scaling, context preservation,
all payload truncations and nonzero alignment rejection. Explicit generation
uses HM only; twenty saved MP4/YUV files regenerate identically. PCM fixture
PPS lists are disabled so they isolate PCM from the chroma-QP extension.
Restoring the old PCM gate reproduces its exact picture-tools refusal.

NVDEC tests verify owned PCM configuration depth/block-field translation for
8/10-bit streams. This remains CPU-side SDK evidence, not physical decoding.
These fixtures do not prove every PCM block size, variable-QP boundary,
reference/seek sequence, chroma format or effective nonzero filter operation.
Other HEVC tools and the overall codec objective remain incomplete.

Local offline validation: codec library 353 passed / one fixture-generator
test ignored; media library 334 passed / one ignored; CUDA-feature HEVC tests
16 passed. No FFmpeg or network access is needed by these tests.

### HEVC PCM reference pictures and smaller coding units (2026-10-06)

Owned three-picture I/B/B streams now confirm that an all-PCM I picture is
used by actual motion-compensated B reconstruction, with WPP both disabled
and enabled. Metadata requires actual PCM in the first picture, zero PCM in
the later pictures, a B slice, populated motion and a resolved motion reference
to POC zero in the first B. Every picture matches independent HM samples on
initial decode and reset replay. The WPP case requires two entropy substreams.

Additional owned one-picture fixtures force PCM bounds to 8x8 and 16x16.
Acceptance requires all luma samples to come from PCM and matches every saved
HM pixel after reset. Together with the original 32x32 case this covers all
base-layer PCM coding-unit sizes. CPU-side NVDEC tests check the corresponding
minimum-size fields. Twenty-eight MP4/YUV files regenerate byte identically.
This does not qualify physical GPU decoding or arbitrary seeks, variable-QP
PCM boundaries, other chroma formats or the remaining codec tools.

Offline validation: codec library 355 passed / one explicit generation test
ignored; CUDA-feature HEVC admission/SDK tests 16 passed. No device run is claimed.

### HEVC tile-scan mapping and explicit reproduction (2026-10-06)

The owned 64x64 I/B/B fixture has two vertical tiles, two entropy substreams
per picture, no PCM/chroma-QP list/WPP, and disabled filters. Its tile-scan
CTU raster addresses are 0,2,1,3; this is deliberately non-raster order.
Metadata checks the tile geometry and entry-point structure before reproducing
the existing picture-tools refusal. Independent HM pixels are saved and
regenerate byte identically; their future pixel/reset acceptance is explicitly
ignored until tile entropy/reconstruction/filter-boundary support is connected.
A passing refusal is not playback acceptance.

The new bounded native TileLayout derives inverse raster/tile-scan maps,
raster tile IDs, tile starts and CTU rectangles. It validates nonempty positive
partitions, level tile-count limits, exact extents, CTU arithmetic and storage
budget before construction. An asymmetric 3x3/four-tile test checks every map,
rectangle and inverse address, and rejects insufficient budget/invalid extents.
This is a required decoding foundation, not completed HEVC tile support.

Offline codec suite: 358 passed, zero failures, two explicitly ignored
tests (fixture generation and future tile reconstruction acceptance).

### Native HEVC single-slice tile reconstruction acceptance (2026-10-06)

Software reconstruction now visits CTUs in tile-scan order, restarts CABAC
at tile boundaries and stores SAO parameters in raster order. CU metadata,
intra samples and spatial motion candidates exclude neighbours outside the
current tile. QP/context state initializes at tile starts. Deblocking and SAO
respect the PPS cross-tile filtering flag. The tile map consumes remaining
picture budget rather than an independent second full budget. Tiled pictures
currently reconstruct synchronously; the existing row-worker path remains for
non-tiled pictures.

The former refusal is now enabled playback/pixel/reset acceptance. Six owned
I/B/B streams cover two-column and asymmetric 2x2 grids, filters blocked or
allowed across tile boundaries, and 8/10/12-bit sequences. Every sample matches
independent HM output. Cross-tile filter oracles differ in 1535 samples, so the
flag exercises an actual output distinction. Twelve saved MP4/YUV files
regenerate identically. Tests also reject missing/out-of-range/short entropy
substreams, malformed tile extents and insufficient combined allocation budget.
CPU-side NVDEC tests verify uniform/asymmetric geometry translation only.

Multiple slice segments with tiles remain explicitly unsupported. The HM
encoder refuses tiles+WPP for this 4:2:0 profile (it permits the combination
for a high-throughput 4:4:4 profile); this change does not qualify that path.
No physical GPU, arbitrary tile seek, throughput or universal profile claim is
made. The overall codec objective remains active.

Offline validation after tile acceptance: codec library 360 passed / one
explicit fixture-generator test ignored; media library 334 passed / one
ignored; CUDA-feature HEVC tests 17 passed. No physical GPU run is claimed.

### HEVC tiled segment address admission (2026-10-06)

Owned independent/dependent four-segment I/B/B streams reproduce two former
raster-monotonic checks: generic slice ordering and dependent-segment range
validation. Both now compare tile-scan addresses. A bounded scalar address
conversion shares partition validation with TileLayout and does not allocate
full maps for each slice; tests agree with asymmetric inverse maps and cover
arithmetic at the u32 CTU address limit without allocating a huge picture.

For both files, every picture now parses addresses 0,2,1,3, the expected
independent/dependent flags and one entropy stream per segment. The exact
software multi-slice picture-tools refusal remains tested; future HM pixel/reset
acceptance is explicitly ignored pending tiled segment reconstruction/ownership.
NVDEC submission preparation uses the same tile-scan check, preserves all four
NAL units in order and rejects raster-sorted submission. This is CPU-side
submission validation, not a device decode. Sixteen tiled MP4/YUV files regenerate
byte identically. This stage does not close multi-segment tile playback.

Offline validation for this stage: codec library 362 passed, two explicit
ignored tests (fixture generation and pending tiled-segment pixel acceptance);
CUDA-feature HEVC tests 18 passed. No playback/physical-device completion claim.

### HEVC tiled segment reconstruction (2026-10-06)

The previously staged tiled-segment pixel acceptance test is now enabled.
Independent, dependent and mixed segments reconstruct in tile-scan order, with
CABAC restarts at tile boundaries, reference ownership and filter availability
following independent slice and tile boundaries. Nine committed synthetic
I/B/B streams compare every decoded sample against HM, including reset replay,
8/10/12-bit mixed segments and a dependent segment spanning two complete tiles.
The spanning stream has segment raster starts [0,2] and entropy substream counts
[2,1]. Filtered cases exercise both enabled and disabled cross-tile/slice policy.
This supersedes the pending multi-segment tile acceptance status above.
Physical GPU decoding, combined tiles/WPP and universal HEVC profiles remain
unqualified. Benchmark measurements have not yet been run for this change.

Offline validation: codec library 363 passed / one ignored; owned media library
334 passed / one ignored; CPU HEVC NVDEC preparation 18 passed. After formatting
only the changed functions, tiled-segment pixel acceptance was rerun successfully.

### HEVC extended-precision coefficient primitive (2026-10-06)

Added the limited EGk coefficient remainder primitive from H.265 9.3.3.4 and
9.3.3.11. Tests cover explicit bypass-bin vectors, every truncation of those
vectors, maximum-length escapes without an extra terminator, and invalid depth
or Rice inputs. The SPS flag remains refused: this primitive is not connected
to picture decoding, and extended transform scaling/clipping is still missing.
The current HM 18 oracle explicitly rejects ExtendedPrecision because it was
built without RExt__HIGH_BIT_DEPTH_SUPPORT. A high-bit-depth HM build is needed
to generate the owned pixel-acceptance fixture; no acceptance claim is made.

### HEVC extended-precision transform foundation and fixtures (2026-10-06)

HM 18 was rebuilt from the official HM-18.0 source with HIGH_BITDEPTH=ON
for x86_64/Rosetta. Only warning-as-error build policy was relaxed for newer
Clang diagnostics. Owned 8/12-bit tiled I/B/B streams and decoded HM oracles
now reproduce the exact SPS extended-precision refusal. Generation is explicit
via generate_hevc_tiles_sample.py --extended-precision-only and a high-bit-depth
HM encoder/decoder; normal tests do not invoke external tools.

Inverse scaling and transforms now have an extended-range entry point: derived
coefficient bounds, scaling shift, intermediate clipping and final shift follow
H.265 8.6.2–8.6.4. The independent factored 4x4 reference exercises both ranges
at every supported depth and QP with dense saturated coefficients. The existing
entry point retains base-range behavior. Picture integration remains incomplete:
the passing exact-refusal test and ignored full pixel/reset acceptance test are
separate. The SPS flag is not yet admitted.

Offline codec validation: 365 passed, no failures, two ignored (explicit fixture
generation and pending extended-precision pixel acceptance).

### HEVC extended-precision picture acceptance (2026-10-06)

SPS extended_precision_processing_flag is now admitted and propagated through
coefficient remainder decoding, inverse scaling and transform reconstruction.
Both persistent and ordinary Rice paths select the limited EGk primitive;
queued reconstruction blocks retain their precision mode. The old SPS refusal
expectation is removed and the pixel/reset acceptance test is enabled.

Six owned HM streams qualify every sample of all I/B/B pictures at 8/10/12 bits,
including tiled mixed independent/dependent segments, WPP, and an enabled
transform-skip/persistent-Rice configuration. Tests assert the active SPS mode
and compare again after reset. This supersedes the staged acceptance notes.
NVDEC still explicitly refuses this unqualified tool, with a regression using
the 8-bit fixture so that refusal cannot be attributed to unsupported depth.
No physical GPU or universal HEVC profile/chroma qualification is claimed.

Offline acceptance validation: codec library 365 passed / one ignored; owned
media library 334 passed / one ignored; CPU NVDEC HEVC preparation 19 passed.
All three runs completed without failures and required no FFmpeg or network.

### HEVC aligned bypass foundation (2026-10-06)

The arithmetic engine now implements H.265 9.3.4.3.6 by setting its interval
to 256 without consuming input bits. Tests cover every legal offset, several
initial ranges, shift-register bypass decoding, truncation, terminated engines
and invalid intervals without mutation. This entry point is not yet wired to
residual syntax and does not enable the SPS flag.

An owned 64x64 4:4:4 12-bit I/B/B fixture uses high-throughput RExt with a 14-bit
profile constraint, WPP, extended precision and CABAC alignment. The high-bit-
depth HM encoder validates this configuration and its decoder matches encoder
reconstruction. HM 18's display code erroneously tests constraint 12 rather
than 14, printing an invalid-profile label although its admission code tests
14 and admits the configuration. The fixture reproduces the exact remaining
SPS range-tool refusal. Pixel/reset acceptance is separately ignored pending
4:4:4 geometry and residual alignment integration. The owned fixture muxer
now accepts explicit chroma metadata, preserving the default 4:2:0 behavior.

Offline codec library validation: 368 passed, no failures, two ignored
(explicit fixture generation and pending aligned 4:4:4 pixel acceptance).

### HEVC aligned coefficient syntax integration (2026-10-06)

SPS CABAC alignment metadata is admitted and passed into block decoding. The
coefficient reader derives escapeDataPresent from significant coefficient count
and greater1/greater2 syntax, then aligns the engine before sign and remainder
bypass bins for that group. Tests prove the event precedes signs for escaped
levels and is absent for level-1/level-2 groups. Arithmetic alignment preserves
context banks, Rice statistics and input position; errors poison HEVC entropy
state until reset. The NVDEC adapter continues refusing this unqualified tool.

The high-throughput 4:4:4 fixture now parses SPS successfully and fails at the
exact unsupported picture geometry gate, replacing its former SPS refusal
expectation. Full pixel/reset acceptance remains ignored pending 4:4:4 planes,
transform-tree semantics and motion/filter geometry. No full playback claim
is made for this fixture. This supersedes the unconnected alignment notes.

Offline validation: codec library 370 passed / two ignored; owned media
library 334 passed / one ignored; CPU NVDEC HEVC preparation 19 passed.
No failures; ordinary tests required neither FFmpeg nor network access.
