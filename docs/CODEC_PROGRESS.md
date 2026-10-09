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
separate FrameHeader/TileGroup OBUs,
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

### HEVC 4:4:4 transform-tree and QP foundation (2026-10-06)

Added 4:4:4 intra/inter transform-tree entry points preserving the existing
4:2:0 API. Every 4:4:4 leaf owns same-resolution chroma blocks, including 4x4;
chroma CBF syntax follows each node with parent-flag inference. Scripted
CABAC tests cover four distinct child coordinates, alternating child flags,
false-parent suppression, inter partition-forced splitting and every truncation
of the scripted tree. H.265 7.3.8.8–7.3.8.9 defines these distinctions.

Component QP mapping now has a format-aware entry point: 4:2:2/4:4:4 use linear
mapping capped at 51 (H.265 8.6.1), while 4:2:0 retains Table 8-10. Tests cover
every supported luma QP at 8/10/12 bits, combined slice/CU offset extrema and
the nonlinear 4:2:0 versus linear 4:4:4 distinction. Picture integration is not
yet enabled: allocation, prediction, motion/filter geometry and intra chroma
mode ownership still need conversion. The existing 4:4:4 HM pixel acceptance
remains ignored and its exact picture-tools refusal test remains passing.

Offline codec library validation: 372 passed, no failures, two ignored
(explicit fixture generation and pending aligned 4:4:4 pixel acceptance).

### HEVC 4:4:4 motion interpolation foundation (2026-10-06)

Added a format-aware motion prediction entry point preserving the existing
4:2:0 API. 4:4:4 components use full-resolution reference geometry and quarter-
sample motion; chroma retains the four-tap filter using even eighth-phase table
entries. Both the integer-copy path and separable filtering use this geometry.
Reference component dimensions/depth are validated before indexing.

The independent scalar interpolation test now covers every phase and both
borders at 8/10/12 bits for 4:2:0 and 4:4:4. It also checks rounded/clipped
public prediction output for unidirectional and bidirectional references.
Picture allocation, chroma intra mode ownership and filter integration remain
incomplete; the existing aligned 4:4:4 HM fixture is still an exact refusal,
with its full pixel acceptance ignored. This is not full 4:4:4 playback support.

Offline validation: codec library 372 passed / two ignored; owned media library
334 passed / one ignored. No failures; normal tests used no FFmpeg or network.

### HEVC full-chroma intra syntax and filtering (2026-10-06)

The intra syntax reader now derives chroma modes with format-aware ownership:
4:4:4 reads a mode per prediction block and resolves derived/collision modes
against that block's luma mode; subsampled formats share the first block's mode.
Interleaved scripted CABAC tests prove ordering, local-luma derivation, every
truncation, shared-mode consumption and validation before consuming input.
The picture path now stores all luma/chroma modes and selects the colocated
chroma prediction block when 4:4:4 geometry is eventually admitted.

Full-resolution chroma reference filtering is separate from luma-only boundary
correction: weak smoothing is allowed, strong smoothing remains luma-only, and
DC/horizontal/vertical boundary correction remains luma-only (H.265 8.4.4.2).
Both synchronous and queued plane reconstruction pass this distinction. Tests
compare weak/strong/subsampled outputs and exercise all three boundary modes.
Picture allocation and loop-filter geometry remain incomplete; the existing
4:4:4 picture refusal and ignored HM pixel acceptance are unchanged.

Offline validation: codec library 374 passed / two ignored; owned media library
334 passed / one ignored. No failures; normal tests used no FFmpeg or network.

### HEVC 4:4:4 picture acceptance (2026-10-06)

Base-layer interleaved 4:4:4 at 8/10/12 bits is now admitted into owned picture
reconstruction. Full-resolution plane allocation, CTU/SAO coordinates, PCM
component counts, intra/inter transform trees, 8x8 chroma coefficient scans,
linear chroma QP, motion prediction and queued reconstruction are connected.
Chroma deblocking uses full-resolution edge spacing and linear threshold QP.
The former picture refusal is replaced by metadata/picture acceptance; the
previously ignored aligned 4:4:4 pixel test is enabled.

Eight owned HM I/B/B streams compare every pixel and replay after reset:
high-throughput 14-bit constrained alignment/extended-precision coding at
12-bit sample depth; ordinary filtered 4:4:4 at 8/10/12 bits; QP=40; WPP;
mixed independent/dependent segments in tiles; and 128x96 WPP that activates
queued reconstruction. The QP=40 case exercises the distinction from 4:2:0's
nonlinear QP mapping. Raw PCM entropy tests separately cover three equal
planes, sample depth scaling, restart/context preservation and every truncation.

Generate explicitly with generate_hevc_alignment_sample.py --full-chroma-suite
and HIGH_BITDEPTH HM. Ordinary tests consume only committed owned MP4/YUV.
NVDEC remains restricted to its qualified Main/Main10 4:2:0 tools; no physical
GPU, all profiles, 4:2:2, separate-colour-plane or 14/16-bit sample claim is made.
These results supersede the pending 4:4:4 picture acceptance notes above.

Offline validation: codec library 376 passed / one ignored; owned media library
334 passed / one ignored; CPU NVDEC HEVC preparation 19 passed. The additional
QP=40 acceptance was rerun after that broad pass and also succeeded. No failures
and no FFmpeg/network requirement in normal tests.

### HEVC 4:2:2 filtered base-layer decoding (2026-10-06)

Native 4:2:2 I/P/B reconstruction now admits 8/10/12-bit equal-depth components.
The transform tree reads upper/lower Cb before upper/lower Cr, preserves both
flags through minimum-size luma splitting, and accounts for both in QP syntax.
Reconstruction uses normative 4:2:2 intra angle remapping, independent axis
scales for motion and PCM, rectangular SAO CTUs and direction-dependent chroma
deblocking geometry. Existing 4:2:0/4:4:4 entry points remain supported.

Three owned 64x64 I/B/B streams with SAO/deblocking match every saved independent
HM sample and replay after reset. Software MP4 playback matches the same samples
and replays after rewind. The former picture refusal has been replaced with
acceptance and the staged pixel test is enabled. No external tools are invoked
by these tests. This is evidence for these streams, not qualification of every
4:2:2 tool: WPP, tiled segments and queued reconstruction remain to be covered.

### HEVC 4:2:2 WPP, tiled segments and queued reconstruction (2026-10-06)

Extended the owned 4:2:2 corpus with three 12-bit I/B/B streams: 64x64 WPP,
64x64 mixed independent/dependent slices in two tile columns with filtering
across tiles, and 128x96 WPP to enable the existing reconstruction queue.
Metadata tests assert SAO/deblocking, entropy sync, tile column count, actual
independent/dependent slice headers and the queue-enabling picture geometry.
All six 4:2:2 streams match every saved independent HM pixel after decoder
reset and software-player rewind. These three cases supersede the pending
WPP/tiled/queue qualification note above. They do not claim all HEVC profiles,
separate-colour-plane, multilayer coding or deeper-than-12-bit support.

Offline validation: three integration tests exercise six fixture pairs and
pass; 63 Python policy/generator tests pass. Normal tests require no HM,
FFmpeg or network. Explicit generation verified HM decoded bytes equal its
encoder reconstruction before saving each oracle.

### HEVC cross-component CABAC and residual foundations (2026-10-06)

Owned 4:4:4 8/10/12-bit I/B/B streams with the cross-component PPS flag and
saved HM oracles reproduce the exact PPS refusal. Full pixel/reset acceptance
is explicitly ignored until integration; an enabled flag alone does not prove
nonzero alpha and must not be used as such evidence.

Added the normative signed residual modification (H.265 8.6.6) with independent
integer-formula checks for all alpha values, mixed-depth scaling, negative
rounding and atomic overflow refusal. Added context-coded truncated-unary alpha
syntax (0, +/-1, +/-2, +/-4, +/-8), separate Cb/Cr magnitude/sign banks initialized
to 154 for every slice initialization, and retained banks in entropy snapshots.
Tests cover exact bin/context order, every syntax truncation, initialization,
adaptation and invalid-context poisoning. PPS admission and synchronous/queued
picture residual integration remain outstanding; no cross-component playback
support is claimed yet.

### HEVC cross-component picture acceptance (2026-10-06)

PPS admission, CABAC alpha syntax and residual modification are connected for
interleaved 4:4:4. Intra eligibility uses the signalled derived-chroma mode code;
inter eligibility uses the luma CBF. Alpha is read before each chroma residual,
including zero-CBF chroma that derives residuals entirely from luma. Original
inverse-transformed luma residuals are retained before clipping for both chroma
components. Synchronous and queued reconstruction carry the same alpha and
residual rules. Other chroma formats and separate-colour-plane remain excluded
from this tool as required by its format constraints.

Four owned HM streams compare every sample and replay decoder reset/software
player rewind: 64x64 at 8/10/12 bits and 128x96 12-bit WPP that activates queued
reconstruction. A test-only picture counter proves nonzero alpha is decoded in
every stream. The exact PPS refusal test is replaced with PPS/picture acceptance,
and the formerly ignored pixel test is enabled. These results supersede the
pending cross-component integration notes above; all HEVC profiles/tools remain
an open objective.

Offline validation: codec library 387 passed / one ignored; owned media library
334 passed / one ignored; cross-component integration three passed, 4:2:2 three
passed and RExt smoothing sixteen passed. No ordinary test invokes external
codec tools or network.

### HEVC monochrome filtered picture acceptance (2026-10-06)

Native monochrome reconstruction now admits 8/10/12-bit luma with independent
inferred chroma depth. Chroma planes are absent, no chroma syntax/prediction,
PCM payload or filters are consumed, and QP is derived only for luma.
Three owned filtered I/B/B streams match all HM luma samples and replay after
reset. Software playback/rewind matches luma and supplies neutral chroma for
display, fixing the previous empty-chroma slice panic with a synthetic regression.
The former picture refusal is replaced and luma acceptance is enabled.
This corpus does not independently qualify monochrome WPP/tiled/queued paths.

### HEVC monochrome WPP, tiles and queued acceptance (2026-10-06)

Added owned 12-bit streams for 64x64 WPP, mixed independent/dependent segments
in two tile columns with cross-tile filtering, and 128x96 WPP that activates
queued reconstruction. Metadata tests assert the active entropy-sync/tiles
flags, independent/dependent slice headers and queue-enabling dimensions.
All six monochrome fixture pairs match HM luma after reset and software-player
rewind, with neutral chroma for display. This supersedes the pending monochrome
WPP/tiled/queued note above. Three integration tests pass offline without any
external codec tools. Other HEVC profiles/tools and the full codec-gap objective
remain open.

### HEVC monochrome PCM corpus acceptance (2026-10-06)

Extended the explicit PCM generator with a monochrome mode, preserving default
4:2:0 generation. Fourteen owned streams qualify active 8x8/16x16/32x32 PCM,
mixed PCM/non-PCM CUs, SAO/deblocking with protected/unprotected PCM, 8-bit PCM
scaled to 10/12-bit reconstruction, full-depth PCM, 128x96 queued reconstruction,
WPP, dependent segments and reference-picture reuse. The fixture test asserts
nonzero actual PCM coverage (partial in mixed streams), absent chroma, every
saved HM luma sample and replay after reset. The targeted corpus test passes;
normal tests consume saved bytes without HM, FFmpeg or network.

### HEVC 4:2:2 / 4:4:4 PCM corpus qualification (2026-10-06)

Extended the explicit PCM generator with anisotropic 422 and full-resolution
444 source geometry and distinct fixture prefixes. Twenty-eight streams cover
8x8/16x16/32x32 PCM, mixed coding, both filter policies, input/full PCM depths,
WPP, queued reconstruction, dependent segments and reference reuse.
Every saved HM sample matches after native decoding and reset. Tests assert
actual PCM coverage and chroma dimensions, so these streams qualify PCM rather
than merely PPS admission. The targeted 28-stream acceptance passes, as do 63
Python checks. Normal tests require no external codec tools or network.

### HEVC mixed component depth native acceptance (2026-10-06)

Removed the blanket equal-component-depth refusal from native single/multi-slice
picture admission. Existing reconstruction selects component depth per plane.
Three owned filtered 4:4:4 I/B/B streams at Y/C depths 8/10, 10/8 and 12/10 match
every saved HM sample and replay after reset. The fixture muxer now accepts an
explicit chroma depth and writes correct hvcC metadata while preserving its
equal-depth default. HM generation is separate from ordinary tests.

This is native decoder evidence. Player/export depth transport, mixed-depth
WPP/tiles/queue and cross-component prediction need separate qualification;
no complete mixed-depth production workflow claim is made yet.

### HEVC mixed-depth tool combinations (2026-10-06)

Extended the three Y/C depth pairs with WPP, mixed independent/dependent tile
segments, 128x96 queued WPP reconstruction and cross-component prediction.
All 15 streams match every saved HM sample after reset. Tests assert active
PPS tools, segment types and queue-enabling dimensions. The existing test-only
alpha counter now verifies nonzero alpha in mixed-depth cross-component
streams, qualifying both directions of luma-to-chroma residual scaling.
Generation checks decoded oracle bytes against HM encoder reconstruction;
ordinary tests invoke no external tools. Player/export transport remains open.

### HEVC player component precision and chroma geometry (2026-10-06)

Software MP4 playback now carries explicit packed planar geometry for non-4:2:0
and mixed-depth HEVC instead of treating every picture as equal-depth 4:2:0. Components with different depths
are lifted exactly to the larger depth before display; monochrome pictures
receive neutral chroma. Pending-frame storage accounting includes packed data.
All fifteen mixed-depth fixture variants compare every packed display sample
with the owned HM oracle across rewind. The six 4:2:2 fixtures additionally
check full-height chroma and every packed sample across rewind. These are
software display-transport checks, not GPU or export qualification.

### HEVC mixed-depth owned lossless export (2026-10-06)

Removed the equal-component-depth rejection from the owned MP4 spool path.
Export metadata and packed samples use max(Y depth, C depth), lifting lower-depth
components by an exact left shift. Monochrome continues to use luma depth.
The root MP4 FFV1 writer now accepts explicit packed planar pictures.
`hevc_mixed_depth_export` checks all fifteen owned streams through both public
export entrypoints, with and without negate, against independently scaled HM
samples and scalar negate expectations; every exported frame is replayed after
rewind. This covers native MP4 to FFV1/Matroska transport, not mixed-depth HEVC
encoding or every other export format.

### HEVC 14/16-bit native acceptance (2026-10-06)

Extended native HEVC component admission through 16 bits. Deblocking limits
scale with sample depth; motion interpolation uses minimum two-bit headroom
instead of subtracting depth from 14 without a lower bound. High dynamic-range
inverse transforms accumulate in i64 before normative shifting and clipping,
while the existing factored i32 transform remains for bounded lower ranges.

Six owned HM streams qualify high-throughput 16-bit intra at actual 14/16-bit
precision, each with ordinary and 128x96 WPP reconstruction, plus 14-bit
high-throughput I/B/B at both sizes. Their samples contain information beyond
12 bits. Acceptance checks every decoder sample, software playback, both
FFV1/Matroska export APIs, scalar negate and reset/rewind. Fixture generation
checks HM decoder bytes against encoder reconstruction separately from tests.
This does not qualify every high-depth chroma/profile/tool combination,
16-bit inter profiles, hardware paths or HEVC encoding.

### HEVC deep PCM acceptance (2026-10-06)

Added ten owned three-frame 4:4:4 high-throughput intra PCM streams at 14/16
bits. Each depth covers input-8 PCM, full-depth PCM, mixed PCM/non-PCM with
filtering enabled on PCM, dependent segments, and 128x96 WPP. Full-depth input
is generated directly as little-endian 14/16-bit noise; input-8 uses the original
8-bit source representation. The minimum component QP makes HM actually select
PCM: QP zero can otherwise bypass PCM due to HM's raw-cost precheck.

Core acceptance asserts actual PCM sample coverage (full or partial), explicit
PCM depths, dependent headers and WPP, and compares every saved HM sample
after reset. Player acceptance verifies software playback and FFV1/Matroska
exports through both public APIs across rewind. These qualify the current
high-depth PCM implementation; other chroma/profile/tool combinations remain
separate work. No generator or external codec runs in ordinary tests.

### HEVC SCC base profile admission (2026-10-06)

SPS/PPS now parse SCC extension presence separately from multilayer, 3D and
unknown extensions. Profile 9 streams with SCC coding tools disabled are
admitted, including an explicitly empty PPS palette initializer list. Reserved
motion-resolution value 3 is invalid. Active current-picture prediction, palette
coding, integer-motion resolution, intra-boundary-filter disable and adaptive
colour transform still have explicit unsupported errors; no full SCC claim.

Six owned 4:4:4 8/10-bit I/B/B streams cover ordinary coding, 128x96 WPP and
empty initializers. The explicit generator uses HM for base reconstruction and
changes PTL plus inert SCC SPS/PPS syntax only, preserving entropy bytes.
Acceptance compares every saved pre-rewrite HM pixel in decoding, software
playback and both FFV1 export APIs across reset/rewind. A separate synthetic
reserved-motion fixture checks invalid syntax refusal, not playback acceptance.

Separate colour planes were investigated against ITU-T H.265 V11 A.3.5/A.3.6:
standard base-layer RExt/high-throughput profiles constrain their flag to zero.
The existing refusal has not been removed or described as supported decoding.

### HEVC SCC intra boundary filtering disable (2026-10-06)

SPS retains `intra_boundary_filtering_disabled_flag` and applies it to both
angular and DC boundary correction in synchronous and queued reconstruction.
RDPCM's existing angular-only bypass exclusion is preserved separately from
DC's boundary rule. Reference-sample smoothing remains independent.

Four owned SCC 4:4:4 I/B/B streams at 8/10 bits qualify ordinary and 128x96
WPP paths. A separate HM18 HIGH_BITDEPTH encoder/decoder build uses the saved
`hm_scc_boundary_oracle.patch` to suppress angular and DC boundary correction
for this controlled oracle. This is a patched HM oracle, not unmodified HM SCC
support. The generator checks encoder reconstruction against decoder output,
then signals SCC syntax without changing entropy bytes. Binary, patch and
fixture hashes are saved in `hevc-scc-boundary-oracle.json`. Ordinary tests use
only the saved bytes.

Acceptance checks every pixel after decode/reset, software playback/rewind,
and both FFV1 export APIs. Clearing the SPS boundary flag demonstrably changes
the first-frame samples for each fixture, proving actual feature coverage.
Independent constant-reference tests check unmodified DC/horizontal/vertical
predictors with boundary correction disabled at every admitted even depth.
Other SCC tools (palette, current-picture prediction, integer-motion resolution
and adaptive colour transform) remain incomplete.

HEVC SCC integer motion: parse SPS resolution control (0/1/2), consume adaptive
slice selection, floor MVPs before adding integer MVDs with signed-16 wrapping,
and round selected merge vectors after candidate-list derivation. Eight owned
SCM 8.8 streams cover 8/10-bit forced, adaptive and WPP decoding/playback/export;
this does not qualify SCC palette, IBC or adaptive colour transform.

HEVC SCC adaptive colour transform now parses PPS/slice ACT offsets, decodes the
TU ACT CABAC flag before delta-QP syntax, uses ACT component QPs and applies the
inverse three-component residual transform before clipping reconstructed pixels.
Synchronous and queued WPP reconstruction preserve TU ordering. Bypass with mixed
component depths is rejected as non-conforming. Ten owned official SCM 8.8
streams qualify 8/10-bit intra/inter, WPP, non-default ACT offsets and lossless
bypass, with active-block counters, pixel comparison, reset, playback and export.
SCC palette and current-picture prediction remain incomplete; mixed/deeper ACT
streams and non-zero slice ACT offsets still need dedicated stream qualification.

ACT qualification now additionally covers 18 owned streams with mixed 8/10-bit
and 10/14-bit component depths, profile 11 with WPP, and non-zero slice offsets.
Encoder-only configuration patches select existing slice-offset controls and
correct SCM's SCC14 admission gate; the unmodified decoder supplies the oracle.
Every stream uses ACT in both intra and inter blocks. Decoder reset, common-depth
playback, rewind and both FFV1 exports preserve all samples. This extends the
previous 8/10-bit qualification, not palette/current-picture support.

SCC current-picture prediction now passes twelve owned 8/10-bit fixtures:
ordinary/WPP 4:4:4, weighted prediction and odd-vector 4:2:0. Native samples,
reset, software playback, rewind and both FFV1 exports equal the SCM oracle.
A logging-only reference probe proves actual IBC use; native counters also prove
fractional chroma sampling in the 4:2:0 streams. Current references access the
unfiltered reconstruction without allocating another picture. Source bounds,
readiness, CU exclusion and WPP availability are checked. Current reference
weight flags are inferred rather than consumed from the header.

SPS/PPS capability dependencies and reference-list selection remain tested,
including inert capability fixtures. Mixed ordinary/current references,
B prediction, tiles, dependent segments and additional depth combinations
still need dedicated IBC qualification; palette support remains unfinished.

Latest offline IBC validation: codec library 405 passed / one ignored explicit
fixture-generator test; owned media library 334 passed / one ignored; Python
policy/generator suite 63 passed. Active IBC integration: two passed, none
ignored. Ordinary tests invoked no FFmpeg or reference decoder.

Mixed-reference IBC: four additional 8/10-bit ordinary/WPP streams reproduce
`HEVC current-picture vector violates block availability` before the fix, due
to incorrectly discarding temporal current-reference predictors. H.265 8.5.3.2.9
permits matching long-term classifications and copies those vectors without
POC scaling, including zero POC distance for current references. Native output,
reset, software playback, rewind and both FFV1 exports now equal SCM. Header
checks prove active mixed lists; codec counters require actual current and
completed-picture predictions. B prediction, tiles and dependent segments
remain separately unqualified.

IBC B/partition qualification: eight further 8/10-bit fixtures cover B slices
with/without WPP, tiles and dependent segments. B fixtures require actual
biprediction plus current and completed-reference predictions, not just B
headers. Tiled/dependent fixtures validate tool flags and dependent headers.
All twenty-four streams match every SCM sample through reset, software playback,
rewind and both FFV1 exports. These checks qualify those configurations;
palette and additional depth/combined-tool coverage remain open.

Palette preparation: four actual SCM palette fixtures (8/10-bit ordinary/WPP)
now have independent decoder/probe pixel agreement and recorded active counts.
The exact SPS palette refusal is tested; full pixel/reset acceptance remains
explicitly ignored. Owned predictor state implements equation 8-79 transactionally,
with ordered reuse, unused-tail retention, bounded EG0 reuse/new-entry syntax,
component-major samples and per-component depth checks. Truncation, out-of-range
runs/counts and invalid state updates reject. Index runs, escapes, CABAC mode
flags and CU reconstruction are not connected; palette playback remains open.

Palette map/reconstruction primitives now implement alternating-row traversal,
run expansion and copy-above across row boundaries with full-block coverage
checks. Reconstructed samples use luma-domain transposition and chroma
subsampling (8-69..71), bypass escapes and rounded/clipped lossy escape scaling
(8-72..78), including mixed component depth and bounded 64-bit arithmetic.
Unit tests cover mono/420/422/444, transpose, row crossing, clipping, malformed
indices/runs/escapes and geometry. These are not full stream acceptance:
CABAC index/run syntax, SPS/PPS palette admission and CU integration remain open.

Palette CABAC preparation now includes four independent context banks (mode,
run-prefix, shared copy-above/final-run flag, transpose), initialized from
H.265 tables 9-38/40/41/42. Run-prefix context selection follows table 9-51
including bypass after bin four. Context snapshots/WPP restoration retain all
palette banks for I/P/B initialization. Bounded TR prefix/TB suffix reading
implements equations 7-85/86, with inferred zero-maximum runs and truncation
checks. These helpers are not yet CU-connected; stream acceptance remains
explicitly ignored and the exact palette refusal remains passing.

Palette index/map parsing now implements bypass Rice/EGk index counts and
truncated-binary index IDs (9.3.3.13/14), including the reduced alphabet after
the first index. The map state machine derives explicit/inferred copy-above,
final-run behavior and omitted-reference-index adjustment (7-83/84), expands
runs in alternating-row order and rejects unused/exhausted indices or derived
run bounds. Tests distinguish inferred zero-alphabet paths (no bits consumed),
truncation, oversized counts and an exact two-color/copied-row map. These parts
still await palette header/escape/CU integration; no full stream support claim.

Palette escape parsing and CABAC bridge now implement FL samples for bypass CUs,
EG3 for quantized samples, component-major traversal and chroma location filtering,
including transposed 4:2:2. Overflow/truncation and invalid maps reject before
publication. Header::read and Header::samples connect existing palette helpers
to real HevcCabac decisions/bypass bins, with an explicit shared-QP syntax boundary.
An empty-palette escape block exercises that bridge. SPS/PPS palette parameters,
CU invocation, predictor lifetime across tiles/WPP and full fixture acceptance
are still pending; no stream playback support is claimed.

Palette SPS syntax now parses the enabled flag, bounded palette/predictor maxima,
component-major initializers and their depth-specific sample widths. Initializer
counts are checked against predictor size and the caller's allocation budget;
zero-size palettes cannot signal initializers. Four additional SCM streams carry
actual nonempty SPS initializer tables (8/10-bit ordinary/WPP), independently
verified by unmodified decoder and logging-only palette probe. Metadata tests
accept all eight SPS configurations. Playback refusal remains explicit in
configuration/in-band SPS admission and direct single/multi-slice reconstruction;
full pixel/reset acceptance is still ignored pending CU/PPS integration.

Palette PPS syntax now reads nonempty initializer tables, requires SPS palette
capability, bounds counts against predictor capacity/allocation budget, and
checks monochrome/depth fields against the active SPS. Four additional owned
8/10-bit ordinary/WPP PPS-initializer streams agree with SCM/probe pixels.
Parameter tests reject missing capability and mismatched depth. Predictor
construction now inherits the SPS table when PPS initialization is absent,
uses a supplied PPS table, and preserves an explicit empty-table override.
The preexisting inert zero-entry PPS compatibility case is preserved. Full
palette reconstruction admission stays closed pending CU/state integration.

Palette CU integration supersedes the preparation/refusal notes above. The mode
flag is read before intra partition syntax; header/QP/map/escape reconstruction
and predictor updates are connected for synchronous and queued reconstruction.
Palette samples are excluded from deblocking as required by 8.7.2; the first
pixel comparison exposed and fixed incorrect filtering. Predictor snapshots
follow WPP CABAC snapshots, reset at independent slice/tile starts, and survive
dependent-segment continuity. All twelve ordinary/WPP 444 8/10-bit fixtures,
including SPS/PPS initializer variants, match every SCM sample through reset,
software playback, rewind and both FFV1 exports. Acceptance is enabled and the
old refusal test removed. Additional chroma/depth, escape-heavy, tiled/dependent
and combined-tool fixture qualification remains open; this is not universal
codec/profile completion.

Palette tile/dependent qualification: four additional 8/10-bit owned streams
contain two tile columns, with dependent-segment variants. Unmodified SCM and
logging-only palette probe agree; actual palette use is recorded. Enabled
acceptance compares all sixteen fixture streams, reset, software playback,
rewind and both FFV1 exports. Header checks require actual dependent segments,
not merely PPS flags. Tile reset and dependent continuation are now stream
qualified for these cases; additional chroma/depth and combined tools remain.

Palette chroma qualification adds twelve owned streams: mono/420/422, 8/10-bit,
ordinary/WPP. SCM reconstruction and logging-only probe agree with recorded
actual palette blocks. Acceptance compares original codec samples and reset,
software playback/rewind and both FFV1 exports. Monochrome SPS infers chroma
depth 8; display adds neutral 420 chroma, root lossless export retains this
420 representation while the media library promotes monochrome to 444.
Tests compare all original luma and exact neutral samples at each API's explicit
representation, rather than equating these different geometries. Other depth,
escape-heavy and combined-tool qualification remains open.

2026-10-06: Palette escape qualification expanded to 36 owned streams. Eight new 4:4:4 8/10-bit cases exercise actual lossy EG3 and bypass fixed-width escape decoding, ordinary/WPP. Native pixel/reset/playback/rewind and both FFV1 export checks pass (`hevc_scc_palette`: 2 passed, 0 ignored). This does not qualify every depth/chroma/tool combination or establish complete codec coverage.

2026-10-06: Palette escape qualification now includes 16 additional owned 4:2:0/4:2:2 streams (8/10-bit, lossy/bypass, ordinary/WPP). Reference logging proves 2763 lossy and 6738 bypass luma escape reads across the new streams. All 52 palette streams pass exact native pixels, reset, software playback/rewind and both lossless exports in the offline acceptance test (2 passed, 0 ignored). No additional depth or transpose coverage is inferred from these results.

2026-10-06: Eight owned monochrome palette-escape streams extend qualification to 60 total (8/10-bit, lossy/bypass, ordinary/WPP). Logging-only reference probe observed 1151 lossy and 3330 bypass luma escape reads in these eight. Offline native sample/reset/playback/rewind and both lossless-export acceptance checks pass (2 passed, 0 ignored). Display/export neutral chroma follows the existing monochrome API expectations; no chroma input is invented in the oracle. Other depths and tool combinations remain unqualified.

2026-10-06: Logging-only SCM probe now records actual transposed palette escape reads. All 32 escape fixtures (mono/420/422/444, 8/10-bit, lossy/bypass, ordinary/WPP) have nonzero counts. Generator rejects missing coverage, and an offline provenance regression verifies every saved escape case. All 60 streams again pass exact native pixels/reset/playback/rewind/both exports (3 tests passed, 0 ignored). This supersedes the earlier absence of observed transpose qualification for these specific fixtures; broader depths/tools remain unqualified.

2026-10-06: Added 28 owned 12-bit palette streams, bringing the total to 88. SCC high-throughput uses the legal 14-bit profile constraint with actual 12-bit component depths and mandatory WPP; encoder configuration validation uses the documented scm_scc14_config.patch, reference decoder remains unmodified. Tiles/dependent tile cases are excluded at 12-bit because SCM rejects tiles+WPP for this profile. Mono/420/422/444, predictor initializers, lossy/bypass/transposed escapes all pass exact samples, reset, playback/rewind and both exports (3 offline tests passed, 0 ignored). Input is owned 8-bit pattern promoted by the encoder, so arbitrary full-precision 12-bit input remains a separate qualification.

2026-10-06: Eight additional owned full-precision 12-bit palette/escape streams cover mono/420/422/444 lossy and forced bypass under mandatory WPP. Source uses 16-bit little-endian storage with nonzero low four bits, not promoted 8-bit samples. Generator proves low-bit retention in the oracle and source==oracle for bypass; offline provenance regression verifies retained low bits and actual transposed escape reads. All 96 palette streams pass exact native pixels, reset, playback/rewind and both lossless exports (3 tests passed, 0 ignored). This qualifies these full-precision patterns, not arbitrary 12-bit streams or all codec tools.

2026-10-06: 36 new owned 14-bit SCC palette streams extend acceptance to 132 total. Covers mono/420/422/444, SPS/PPS initializers, lossy/bypass/transposed escapes and eight full-precision input cases. High-throughput SCC uses 14-bit constraint and mandatory WPP; forbidden tiles+WPP combinations remain excluded. Generator confirms low-bit oracle retention and exact bypass input preservation. All 132 pass native sample/reset/playback/rewind/both lossless-export acceptance (3 offline tests passed, 0 ignored). Existing AVC field-reference and display-reordering refusals remain in source; this is not completion of the codec-gap objective.

2026-10-06 AVC audit correction: the earlier statement that display reordering is wholly unimplemented was too broad. AvcDecoder::decode_order accepts coded-order I/P/B pictures, and the MP4 playback reader performs presentation ordering using timestamps. AvcDecoder::decode intentionally requires increasing POC; its error now directs callers to decode_order. The owned avc-multislice-ipb fixture proves this specific API refusal, decode-order acceptance, reset, and exact presentation-order oracle playback/rewind in a dedicated regression. avc_multislice: 12 passed, 0 ignored, offline without FFmpeg. Field reference lists/marking remain unsupported; this audit does not remove those gaps.

2026-10-06 MBAFF gap reproduction: avc-mbaff-cabac.mp4 and avc-mbaff-cavlc.mp4 are owned three-picture 64x64 x264 CLI streams, generated explicitly by scripts/generate_avc_mbaff_sample.py (no FFmpeg). avc_mbaff verifies parsed MBAFF SPS and matching entropy mode, then the exact progressive-only reconstruction refusal, excluding unrelated parse failures. This is a passing refusal/reproduction test, NOT playback acceptance or an implemented interlaced decoder. Future support must replace this expectation with pixel acceptance against an independent oracle. Current offline reproduction: 1 passed, 0 ignored.

2026-10-06 MBAFF implementation foundation: src/codec/avc_mbaff.rs provides checked component sample addressing for progressive blocks and frame/field macroblock pairs (H.264 6.4.1). Supports mono/444 geometry and 422/420 subsampling with field row stride. Tests prove mixed frame/field pairs cover every component sample exactly once and reject out-of-picture/overflow/malformed geometry (2 passed offline). This helper is not yet connected to entropy, prediction or deblocking; the owned MBAFF playback refusal remains unchanged.

2026-10-06 MBAFF addressing continuation: added component sample-to-owner mapping for mixed frame/field pairs. Unknown pair mode or out-of-picture sample returns unavailable instead of guessing. Forward/inverse round trips cover each sample of mono/444, 422 and 420 layouts; progressive mapping is now used by existing intra reconstruction. Three geometry tests and 16 integration tests (MBAFF refusal, AVC multislice pixel/rewind/seek and parameter updates) passed offline. Entropy field flags, MBAFF prediction neighbours and deblocking remain incomplete; interlaced playback acceptance is not claimed.

2026-10-06 MBAFF field syntax foundation: PairMode reads mb_field_decoding_flag on even macroblock addresses or an odd address after skipped top, and otherwise inherits the known pair mode (H.264 7.3.4). Reader callback can supply a CAVLC bit or future CABAC decision; no CABAC context implementation is implied. State commits only after successful flag read; missing top mode is an error. Four addressing/state tests and two owned fixture tests passed offline. Actual CAVLC IDR fixture validates flag precedes I macroblock type. This is syntax-prefix qualification, not full coefficient parsing or playback acceptance; progressive-only reconstruction refusal remains.

2026-10-06 MBAFF oracle preparation: generator now saves exact YUV from the independent unmodified JM 19 decoder for both owned CABAC/CAVLC three-frame streams, with executable and oracle hashes. JM decoded all three frames in each. A separate pixel/rewind acceptance test is present but explicitly ignored until native MBAFF reconstruction/prediction/deblocking is connected. Ordinary offline tests: 2 passed, 1 ignored; the passing refusal test remains distinct from acceptance. Generation: python3 scripts/generate_avc_mbaff_sample.py --x264 /path/to/x264 --jm-decoder /path/to/ldecod.exe. No FFmpeg or reference executable is invoked by ordinary tests.

2026-10-06 MBAFF context foundation: component neighbour_location translates local sample offsets with the current field row stride, then resolves ownership in the neighbour pair mode. cavlc_context derives nC from available neighbour 4x4 blocks; missing slice/block counts remain unavailable and counts above 16 are rejected. Six geometry/flag/context tests pass offline, including frame-to-field and field-to-frame luma/chroma transitions. These context helpers are not yet connected to the production MBAFF entropy reader; JM pixel acceptance remains ignored until reconstruction/prediction/deblocking are implemented.

2026-10-06 CAVLC production integration: IntraCavlcReader now derives luma DC/AC and chroma AC nC through the checked component/macroblock neighbour context path. Existing raster coefficient grids provide decoded/slice availability; the duplicate old nc formula was removed and its availability test redirected to the production context helper. 105 AVC core tests and 17 integration tests passed offline; one MBAFF pixel acceptance test remains ignored. This connects the progressive path only: full MBAFF field flags, count-grid ownership, prediction and deblocking are still pending.

2026-10-06 CAVLC macroblock ownership: coefficient-count and intra-mode contexts now use address-major macroblock storage rather than global raster grids. Production mode prediction and nC resolve geometric neighbours through the shared addressing path; PCM/inter publication and count snapshots use the same ownership. Allocated context sizes remain unchanged. 105 AVC core tests and 17 pixel/rewind/parameter/MBAFF-prefix integration tests passed offline; one MBAFF playback acceptance remains ignored. This enables subsequent pair-mode addressing, but does not yet enable interlaced entropy/reconstruction.

2026-10-06 CAVLC intra MBAFF syntax reader: new_mbaff supports explicit intra frame slices with MBAFF. Counts include both macroblocks per map unit, first_mb is converted to pair address, field flags are read at pair starts and inherited below, coefficient contexts/mode neighbours use pair geometry, and luma/chroma 4x4 plus luma 8x8 inverse scans select field order. Incomplete terminal pairs are rejected. Embedded mixed P/B MBAFF dispatch remains unconnected. Added two owned alternating-row field-coded streams (CABAC/CAVLC), independently decoded by JM; x264 reports 100% field macroblocks, and native CAVLC test confirms every parsed IDR block field flag. Both all-frame and all-field CAVLC IDRs consume 16 blocks and exact RBSP trailer. 105 core AVC + 18 integration tests passed offline; one pixel playback acceptance covering all four MBAFF streams remains ignored. No interlaced pixel/reconstruction acceptance is claimed.

2026-10-06 MBAFF sample output foundation: checked write_samples scatters reconstructed component blocks into progressive or every-other-row field layouts. Complete footprint/sample count validation precedes any mutation; invalid/truncated/out-of-plane writes leave the plane unchanged. Existing AVC PCM and reconstructed block writes now use this common checked writer in progressive mode. 106 AVC core and 18 integration tests passed offline; one MBAFF playback pixel acceptance remains ignored. Field-capable writing does not yet connect MBAFF prediction or deblocking.

2026-10-06 MBAFF prediction-edge foundation: prediction_edges reads top/left/corner samples with frame or field row stride and caller-supplied per-sample availability, validates complete block geometry and avoids out-of-plane reads. Existing progressive AVC reconstruction now uses this checked edge gatherer. Test confirms exact field row samples and absent edges when a sample is unavailable. 107 AVC core and 18 integration tests pass offline, with one MBAFF pixel playback test still ignored. Remaining full MBAFF work includes pair-aware reconstruction readiness, prediction assembly, deblocking, CABAC and inter references.

2026-10-06 MBAFF reconstruction readiness: Readiness420 stores slice-local completed 4x4 masks separately for Y/Cb/Cr and each macroblock, with known pair modes and checked memory budget. Publishing inconsistent pair modes or invalid geometry does not expose samples. Combined prediction-edge test proves completed even field rows cannot make undecoded odd rows available; reset_slice clears all availability. Nine MBAFF geometry/syntax/readiness unit tests passed offline. This readiness map is not yet connected to full picture reconstruction; MBAFF pixel acceptance remains ignored and the codec-gap goal remains incomplete.

2026-10-06 first MBAFF pixel acceptance: avc_mbaff_picture implements CAVLC complete intra slice reconstruction with filtering disabled. Uses component frame/field views, slice-local readiness, existing owned intra transforms/prediction and checked sample scatter back into frame storage. Checked budget includes output planes, temporary view and entropy/readiness contexts. New owned avc-mbaff-field-intra-unfiltered-cavlc three-IDR fixture matches every JM sample both via direct reconstruction and native MP4 playback, including two rewind passes. Production intra dispatch enables this path; filtered CAVLC refusal now reports the filtering/complete-slice limit, while CABAC/inter/multi-slice MBAFF remain incomplete. 108 AVC core + 19 integration tests passed offline; one broader filtered/inter/CABAC acceptance test stays ignored. No complete MBAFF, mixed-pair/depth coverage or throughput claim follows from this fixture.

2026-10-06 MBAFF frame/mixed acceptance: added three owned three-IDR unfiltered CAVLC streams: all-frame pairs, field-right/frame-left pairs and field-left/frame-right pairs. Native syntax test verifies exactly 0/8/8 field macroblocks out of 16 in each frame (existing all-field fixture verifies 16/16), so names alone do not imply coverage. All four unfiltered intra streams match every JM sample via direct reconstruction and native MP4 playback, with rewind repeated. Target MBAFF suite: 4 passed, 1 ignored, offline; ignored case remains filtered/inter/CABAC playback. This establishes these mixed spatial patterns at 8-bit, not every pair topology, depth or tool combination.

2026-10-06 MBAFF multi-slice intra support: unfiltered CAVLC intra reconstruction now accepts ordered multiple slices, validates pair-address coverage and resets entropy/readiness for each slice. Added owned three-IDR two-slice field-coded fixture with independent JM oracle; native playback matches every sample and rewinds identically. Target MBAFF tests: 5 passed, 1 ignored (filtered/inter/CABAC). Deblocking/inter/CABAC remain incomplete; this qualifies the saved 8-bit two-slice pattern.

2026-10-06 MBAFF deblocking groundwork: filter_line applies the existing H.264 filter to a checked eight-sample strided line; horizontal field traversal can use twice plane stride. Invalid footprint/parameters fail before mutation. Existing progressive inter/intra traversal now uses this common primitive. Test verifies exact filtered values and untouched opposite-parity rows. 109 AVC core and 20 integration tests passed offline; broader MBAFF filtered/inter/CABAC acceptance still ignored. MBAFF boundary strengths, mixed frame/field edge topology and traversal order remain to implement; no filtered MBAFF acceptance is claimed.

2026-10-06 MBAFF intra boundary strength: added shared H.264 8.7.2.1 derivation for intra/intra edges: external vertical and external frame/frame horizontal bS=4, field-involving horizontal and internal bS=3. Progressive intra grid uses the shared rule. Tests compare the resulting normal versus strong sample outputs for both field modes and mixed flags. 110 AVC core plus 17 integration tests passed offline, with broader filtered/inter/CABAC MBAFF still ignored. Mixed edge topology and traversal remain incomplete; this does not enable filtered MBAFF playback.

2026-10-06 MBAFF deblocking line geometry: added checked q0 position, perpendicular sample stride and p0 macroblock ownership for progressive and mixed frame/field storage. Tests cover field-to-frame vertical ownership and both parity boundaries of a frame top below a field pair, across 444/422/420 component geometry; picture boundary and unknown pair modes remain unavailable. 111 AVC core and 20 integration tests passed offline; one filtered/inter/CABAC acceptance test remains ignored. This supplies neighbour QP/slice lookup geometry, not a connected MBAFF deblocking traversal or filtered playback acceptance.

2026-10-06 MBAFF intra deblocking traversal: added a checked 4:2:0 component filter in macroblock pair address order, vertical before horizontal, deriving each line's neighbour QP and slice identity. Mixed frame-top/field-above boundaries filter both parities with bS=3. Tests compare field traversal against separately filtered progressive field planes for Y/Cb/Cr, verify different frame-owner QPs on a mixed vertical edge, both horizontal parities and slice-boundary disabling; malformed pair modes reject before mutation. 114 AVC core and 20 integration tests passed offline, one filtered/inter/CABAC acceptance test still ignored. The traversal is not connected to reconstruction yet; owned compressed-stream pixel acceptance is the next gate before enabling filtered playback.

2026-10-06 MBAFF filtered CAVLC intra acceptance: production reconstruction now retains pair-address component QPs, field mode, transform size and slice filter controls, and runs intra deblocking after all slices are reconstructed. Its checked memory budget includes deblocking metadata. Added five owned three-IDR 64x64 filtered streams: frame, field, both horizontal mixed layouts, and two field-coded slices, with independent unmodified JM oracle and generator/tool hashes. All match every sample through native playback and two rewind passes; single-slice cases also match direct reconstruction. A same-packet deblocking-disabled comparison proves all four single-slice fixtures actually exercise filtering. Original filtered CAVLC IP fixtures now accept/reset the first intra picture, and their old refusal test checks the still-unsupported inter picture instead. 114 AVC core and 21 integration tests passed offline (one inter/CABAC MBAFF acceptance test remains ignored). This qualifies the saved 8-bit CAVLC intra patterns; inter/CABAC MBAFF, broader pair topology, high depths and tool combinations remain unqualified. Ordinary tests consume saved fixtures and never invoke the generator, x264, JM or FFmpeg.

2026-10-06 MBAFF vertical mixed topology qualification: added two owned three-IDR filtered CAVLC streams with field pairs below frame pairs and the reverse. Syntax assertions verify the actual top/bottom transition and exactly eight field macroblocks per picture, rather than relying on fixture names. Direct reconstruction and native playback match every unmodified JM sample for all three pictures, with two rewind passes; disabling deblocking on the same first packet changes output. This covers the mixed horizontal boundary's two-parity frame-top case and field-top/frame-above case at 8-bit. MBAFF suite: 6 passed, 1 ignored (inter/CABAC); compatibility: 15 multislice/parameter-update tests passed offline; fixture/oracle manifest hashes verified. Inter/CABAC, higher depths and broader tool combinations remain incomplete.

2026-10-06 High10 MBAFF intra qualification: added seven owned filtered three-IDR CAVLC fixtures at actual 10-bit input/output depth (16-bit little-endian source storage, nonzero low bits), covering frame/field, horizontal and vertical mixed topologies in both directions, and field-coded two-slice pictures. Native playback matches every independent JM sample and repeats after rewind. Tests verify SPS depths, actual per-macroblock field flags/topology, all 16 blocks per picture, filtering enabled, at least one decoded 8x8 transform in the corpus, and retained nonzero low-bit precision. MBAFF suite: 7 passed, 1 ignored (inter/CABAC); compatibility: 15 tests passed offline. Fixture/oracle hashes verified. This qualifies these High10 intra streams; MBAFF inter/CABAC, additional chroma formats/depths and wider tool combinations remain incomplete.

2026-10-06 CABAC MBAFF field-flag syntax foundation: field_decoding_flag reads one regular arithmetic bin at context 70 + condTermFlagA + condTermFlagB (H.264 9.3.3.1.1.2). Neighbour availability and skipped-pair inference remain caller responsibilities. Scripted tests cover all neighbour combinations and both flag values; saved owned CABAC frame/field IDR fixtures validate the first pair's actual arithmetic decision, with PairMode bottom inheritance consuming no additional bin. 115 AVC core and 8 MBAFF integration tests passed offline; one full inter/CABAC MBAFF pixel acceptance test remains ignored. This is prefix syntax qualification, not an enabled CABAC macroblock reader or pixel/playback acceptance. CABAC spatial contexts, pair-aware termination and coefficient field dispatch remain to connect.

2026-10-06 CABAC block-context ownership: luma/chroma coded-block flags and intra prediction modes now use macroblock-address/local-cell storage. Progressive spatial lookups translate component raster coordinates into this storage; inter/skip, PCM, 4x4 and 8x8 publications use the same indexing. Allocation sizes are unchanged. Tests cover distinct cross-macroblock luma/chroma neighbours and intra versus inter unavailable coded-context conditions. 116 AVC core and 23 integration tests passed offline; existing CABAC I/P/B, multi-slice pixel/reset/seek and parameter-update behavior is preserved. MBAFF pair-aware spatial derivation, termination and residual field dispatch remain to connect; one full inter/CABAC MBAFF acceptance test remains ignored.

2026-10-06 Shared CABAC/CAVLC component neighbour geometry: avc_mbaff::block_neighbours resolves left/top 4x4 cells into macroblock-address/local-cell ownership with frame/field row steps and subsampling. CAVLC nC and production progressive CABAC coded/mode lookups now use this common geometry. Tests verify mixed vertical luma/chroma transitions, frame-top over field-pair ownership, unavailable unknown neighbouring pair and invalid component cell rejection. 117 AVC core and 23 integration tests passed offline, one inter/CABAC MBAFF acceptance remains ignored. CABAC macroblock-level spatial contexts, pair flags/termination and field residual dispatch remain to connect; this does not enable CABAC MBAFF reconstruction.

2026-10-06 CABAC macroblock neighbour geometry: shared avc_mbaff::macroblock_neighbours resolves left/top origin ownership for progressive and mixed frame/field storage, including unknown mode and picture-edge unavailability. Production progressive CABAC type, transform-size, chroma mode, coded pattern and DC context queries now use this path with checked geometry. Tests cover both blocks of a field pair next to frame storage, frame below field and field below frame, component subsampling and intra/inter unavailable-context conditions. 119 AVC core and 23 integration tests passed offline; one full inter/CABAC MBAFF pixel acceptance remains ignored. Actual MBAFF reader pair modes, pair-aware termination and field coefficient dispatch are not connected yet. Pattern-dependent mixed-boundary context derivation still needs acceptance qualification.

2026-10-06 CABAC MBAFF pair termination rule: shared end_of_slice_flag consumes a termination bin after progressive macroblocks or MBAFF bottom blocks only; top blocks infer continuation without any arithmetic read (H.264 7.3.4 slice_data). Scripted tests cover multiple top/bottom addresses, both termination values and progressive behavior. Existing production progressive end_mb now uses the shared helper; address advances after a successful decision. 120 AVC core and 23 integration tests passed offline, one full inter/CABAC MBAFF acceptance remains ignored. The MBAFF flag is not yet enabled in the macroblock reader, so this is syntax/control-flow foundation rather than MBAFF CABAC pixel acceptance.

2026-10-06 CABAC intra MBAFF syntax reader: explicit new_mbaff accepts I frame slices with MBAFF, doubles map-unit count, converts first_mb to pair address and tracks slice-local pair field modes. Top blocks decode contexts 70-72 from known left/top pairs, bottom blocks inherit; end_of_slice is consumed only after the bottom. Macroblock/component context neighbours use pair geometry, and 4x4/8x8 residual CABAC contexts and inverse scans select field order. Saved owned CABAC frame/field IDRs each parse exactly 16 blocks with expected field flags and precise final slice termination. 120 AVC core and 24 integration tests passed offline; one full inter/CABAC MBAFF pixel acceptance remains ignored. This enables explicit syntax parsing only; production picture reconstruction still refuses CABAC MBAFF. Mixed pair/pattern contexts, high-depth CABAC streams and exact pixel reconstruction remain to qualify before dispatch is enabled.

2026-10-06 CABAC intra MBAFF reconstruction acceptance: production intra dispatch now selects the explicit CABAC reader within the common MBAFF reconstruction/deblocking pipeline. Added seven owned filtered three-IDR CABAC fixtures (frame, field, horizontal/vertical mixed pairs both directions, field two-slice). Actual syntax tests verify per-macroblock pair topology and complete picture coverage; native playback matches every independent JM sample for all three pictures and two rewind passes. Direct first-picture reconstruction also matches JM; disabling deblocking on the same syntax changes every fixture's output. Original CABAC IP fixtures now accept/reset their first intra picture, while old refusal tests check the subsequent unsupported inter picture. 120 AVC core and 25 integration tests passed offline, one full inter MBAFF acceptance remains ignored. CABAC High10, broader pattern/tool combinations and MBAFF inter prediction remain unqualified/incomplete. Generation is explicit with owned source samples, x264 CLI and unmodified JM; ordinary tests need none of those tools or FFmpeg.

2026-10-06 CABAC High10 MBAFF intra qualification: added seven owned filtered three-IDR 10-bit CABAC streams, matching the frame/field, both horizontal/vertical mixed layouts and field two-slice CAVLC corpus. Shared High10 acceptance now checks both entropy modes, actual per-block topology, full 16-block picture coverage, depth and low-bit precision; separate per-entropy counters require actual decoded 8x8 transforms in both corpora. Native playback matches every unmodified JM sample for all three pictures and repeated rewind. 25 integration tests passed offline, one full MBAFF inter acceptance remains ignored; all 37 MBAFF fixture/oracle manifest hashes verified. No codec changes were needed for these samples. Wider tool/pattern combinations and MBAFF inter prediction remain incomplete; High10 intra acceptance is limited to the saved corpus, not universal profile coverage.

2026-10-06 MBAFF inter motion-storage foundation: MotionField now stores 4x4 cells by macroblock address/local raster cell rather than the picture raster. Publication, neighbour reads and transactional save/restore use the same storage indexing; allocation sizes remain unchanged. Persistent ReferenceMotionField snapshots explicitly export picture raster order, preserving co-located B-picture lookup and per-slice reference identities. New 2x2-macroblock test checks distinct vectors in every cell, address-owned storage boundaries and all snapshot positions/identities. 121 AVC core and 25 integration tests passed offline, including existing progressive I/P/B, multislice seek/reset and MBAFF intra coverage. MBAFF motion-neighbour geometry, field/frame vector/ref-index conversion, reference-list construction and inter reconstruction remain incomplete; full MBAFF inter acceptance stays ignored.

2026-10-06 MBAFF motion-neighbour geometry: shared motion_neighbours resolves A/B/C/D (left/top/top-right/top-left) partition positions through current frame/field layout into address-owned local 4x4 cells. Partition extent/alignment are checked; unknown mode and picture-edge samples stay unavailable. Existing progressive MotionField neighbour queries now use this path while retaining slice/decoded availability. Tests cover a field partition crossing between neighbouring frame blocks, a frame top below field pairs, all four neighbour locations and invalid footprints. 122 AVC core and 25 integration tests passed offline; full MBAFF inter acceptance remains ignored. Field/frame vector and reference-index normalization, pair-aware motion publication, reference lists and inter reconstruction remain incomplete.

2026-10-06 MBAFF spatial motion normalization foundation: avc_mv::normalize_neighbour converts frame/field vertical vector units and reference indices (H.264 8.4.1.3.2): frame-to-field y/2 and ref*2; field-to-frame y*2 and ref/2. Unavailable/NoPrediction remain unchanged; same-mode values retain their units. Tests cover negative odd vector division toward zero, every field reference index 0..63, source-list limits and signed-16-bit doubling boundaries/overflow refusal. 123 AVC core and 25 integration tests passed offline, full MBAFF inter acceptance still ignored. This helper is not yet connected to motion-neighbour publication/prediction. Existing progressive predictors still restrict references to 0..31; field-aware prediction, DPB reference-list expansion and complete inter reconstruction remain to implement.

2026-10-06 Field-aware AVC spatial predictor: predict_for_field accepts already-normalized field reference indices 0..63; ordinary predict and frame mode retain 0..31. Both paths share the existing partition preference, single matching reference, top-right fallback and component-median derivation. Tests connect frame-to-field neighbour normalization to predictor selection, exercise references 62/63, top/right partition preferences, missing top-right fallback, invalid reference refusal and progressive compatibility. 124 AVC core and 25 integration tests passed offline; full MBAFF inter acceptance remains ignored. This predictor is not yet called by an MBAFF inter reader. Pair-aware motion storage/publication, field-aware skip/direct paths, DPB list expansion and inter reconstruction remain incomplete.

2026-10-06 MBAFF motion publication/neighbour API: store_mbaff publishes address-local partitions with stored field mode and permits field reference indices 0..63. It validates pair mode consistency, duplicate publication, local footprint and prevents mixing progressive storage. neighbours_mbaff combines A/B/C/D geometry, decoded/slice availability and frame/field vector/reference normalization; conflicting reader/stored modes are rejected. Tests connect actual stored frame neighbours to a field predictor, verify reverse normalization, unused-list/slice isolation, reference 63, mode-change refusal and invalid local origins. Progressive snapshots/neighbour APIs refuse MBAFF storage instead of exporting incorrect co-located data; Cell sizing is included in the existing checked budget. 125 AVC core and 25 integration tests passed offline. No MBAFF inter reader invokes these APIs yet; skip/direct, pair-aware snapshots, field DPB references and motion compensation remain incomplete.

2026-10-06 MBAFF P-skip motion foundation: p_skip_for_field validates expanded field reference indices and applies the existing zero-neighbour/median rule to normalized candidates; ordinary p_skip remains the frame wrapper. MotionField::decode_p_skip_mbaff combines pair-aware lookup, normalization and predictor, then publishes reference-zero L0 and unused L1 across the complete address-local block. Tests cover zero detection after normalization, field reference 63, invalid reference rejection, nonzero median from three stored frame neighbours, all 16 published cells, duplicate publication and unknown pair mode without mutation. 127 AVC core and 25 integration tests passed offline. Slice-reader skipped-pair inference, inter entropy/residual dispatch, field reference lists, compensation and B-direct remain incomplete; full MBAFF inter acceptance stays ignored.

2026-10-06 MBAFF explicit inter motion transactions: decode_mbaff now combines pair-aware neighbour lookup, frame/field normalization, field-aware partition prediction and checked motion differences for both lists before publication. decode_macroblock_mbaff processes explicit L0/L1/Bi partitions in syntax order and restores all address-local cells plus the storage mode on late failure or incomplete coverage. Progressive data cannot be queried as MBAFF. Tests cover mixed frame neighbours, two-list prediction, second-list overflow without publication, references 62/63, unknown modes, invalid geometry, duplicate writes, late partition overflow, incomplete coverage and refusal of unconnected B-direct without disturbing prior blocks. 129 AVC core and 25 integration tests passed offline; one full inter MBAFF acceptance remains ignored. These APIs are not connected to inter entropy dispatch, expanded DPB lists or compensation yet; full MBAFF inter acceptance remains ignored. No production playback capability claim is made by these storage/predictor tests.

2026-10-06 MBAFF CAVLC inter coefficient foundation: read_inter_coefficients_field applies the selected frame/field inverse scan to luma4, interleaved luma8 and chroma AC; progressive entrypoints remain wrappers selecting frame scan. CoefficientField::neighbours_mbaff resolves each external luma/chroma block boundary through actual pair geometry with slice/decoded availability, including boundaries spanning both frame macroblocks. read_inter_mbaff joins those contexts with explicit field-aware prediction/header parsing and commits cursor plus counts only after complete success. Expanded active reference counts are caller-supplied, with a field-only limit of 64 and unchanged progressive limit 32. Unit tests cover mixed-pair nC owners, expanded index63, invalid counts/indices, nonzero 4x4/8x8/chroma scans, truncated residual cursor rollback, duplicate publication and atomic header/count publication. 134 AVC core tests passed offline. Pair flag/skip-run dispatch, embedded intra in mixed MBAFF slices, DPB field-list expansion and reconstruction remain to connect; this is not full inter playback acceptance. No new media failure class was discovered in this step; the existing owned IP MBAFF fixtures and ignored acceptance remain the integration target.

2026-10-06 Embedded intra MBAFF CAVLC context: new_context_mbaff permits I/P/B frame slices using address-owned pair-aware coefficient/mode grids, while new_mbaff retains its I-only contract. record_pair_mode accepts dispatcher-owned modes for intra/inter/skip blocks and rejects changing an established pair. read_embedded_mbaff parses an already-dispatched intra body with that mode, preserves the external cursor on failure and requires the caller to discard failed internal context. Existing ordinary embedded entrypoint still refuses MBAFF use. Saved synthetic fixture tests compare every intra mode, residual level, count, QP and cursor between standalone and embedded readers across eight complete pictures: frame, field, both horizontal/vertical mixed orientations and two mixed High10 layouts. Unit tests exercise P/B context construction, pair consistency, malformed body cursor preservation, invalid addresses and field-picture refusal. 135 AVC core and 25 integration tests passed offline; full MBAFF inter acceptance remains ignored. These tests qualify embedded intra syntax, not mixed P/B playback. Skip-run/pair-mode inference, complete inter dispatch, DPB field lists, compensation and B-direct remain incomplete.

2026-10-06 MBAFF CAVLC mixed slice dispatcher: InterCavlcSlice::new_mbaff now allocates pair-address coefficient/intra contexts and iterates skip, coded inter and embedded intra syntax. For a skipped top with a coded bottom, the top mode is obtained by non-consuming lookahead at the bottom field flag; fully skipped pairs inherit an available same-slice left pair, then above, otherwise frame (H.264 7.4.4). Coded bottom after skipped top consumes its explicit flag; other bottoms inherit the established pair. Field blocks use twice the frame active-reference counts and field residual scans. Pair grids for both the dispatcher and intra context are included in checked allocation accounting. Incomplete pairs, missing coded bottoms, mode mismatch and failed parsing poison the reader. Two original owned IP CAVLC fixtures now parse both P pictures completely, all 16 addresses and RBSP end, twice; frame and field coded syntax are exercised. Synthetic bit tests verify lookahead cursor ownership and both left/top skip inference. 137 AVC core and 27 integration tests passed offline, full inter MBAFF pixel acceptance remains ignored. This qualifies syntax dispatch only; production inter reconstruction still refuses MBAFF pending field DPB lists/snapshots, compensation, inter deblocking and B-direct. Normative source: https://www.itu.int/rec/dologin_pub.asp?id=T-REC-H.264-201106-S%21%21PDF-E&lang=e&type=items .

2026-10-06 MBAFF reference selection and compensation: ReferenceLists::select_mbaff resolves the already modified frame list by field index/2 and selects same parity for even indices, opposite for odd; it returns frame identity, parity and explicit-weight frame index without manufacturing field IDs. ReferencePlane/Reference420::field_view borrow alternating rows with doubled checked stride and half height, with no plane copy or heap allocation. predict_macroblock_mbaff derives frame/field logical origin from pair geometry, resolves per-list frame indices, uses the selected field view and applies H.264 Table8-10 chroma vertical motion adjustment (+2 top reference/bottom current, -2 bottom reference/top current); luma motion remains unchanged. Existing frame predictor is a shared wrapper. Tests exercise all 64 field indices on both lists/parities, missing/invalid lists, padded-stride 8/10bit views versus independently packed fields for fractional vectors and edge extension, actual macroblock parity/chroma sample results and frame-mode compatibility. 141 AVC core and 27 integration tests passed offline; full inter MBAFF pixel acceptance stays ignored. Whole-picture inter reconstruction, mixed frame/field inter deblocking, field-aware motion snapshots/direct and CABAC inter dispatch remain unconnected. PAFF field-picture DPB construction/marking remains a separate gap; MBAFF continues to use frame marking. Normative reference selection: H.264 8.4.2.1, https://www.itu.int/rec/dologin_pub.asp?id=T-REC-H.264-202408-I%21%21PDF-E&lang=e&type=items ; chroma motion: H.264 Table8-10, https://www.itu.int/rec/dologin_pub.asp?id=T-REC-H.264-200903-S%21%21PDF-E&lang=e&type=items .

2026-10-06 MBAFF inter macroblock residual publication: reconstruct_inter_macroblock validates picture geometry, complete plane extents, prediction dimensions/depth/sample range and all target layouts before applying 4x4/8x8 inter residuals or transform bypass. It then scatters the reconstructed macroblock to its own frame rows or field parity. No picture plane changes until validation/residual reconstruction succeeds. Tests cover every address in a 2x2-MB picture, both pair modes, 8/10bit, 4x4/8x8 bypass luma/chroma residuals, all untouched samples, truncated chroma, mismatched depth, invalid address and invalid QP without partial publication. 143 AVC core and 27 integration tests passed offline. This helper still needs the complete mixed-slice picture assembler and inter deblocking/metadata path; full MBAFF inter playback acceptance remains ignored. Existing owned IP fixtures remain the target; no new media failure class was found in this step.

2026-10-06 Shared MBAFF intra macroblock reconstruction: extracted reconstruct_intra_macroblock from the accepted intra picture pipeline for reuse by the mixed P/B assembler. It preserves pair-local sample layout, field logical prediction views, slice-local readiness, scaling/coefficients and component scattering; it validates picture geometry/depth/plane extents before reading plane data. Existing intra picture reconstruction now calls the common helper, avoiding a second intra implementation in the planned mixed decoder. 143 AVC core and 26 integration tests passed offline, including all stored intra MBAFF/JM pixel cases. No new codec tool is admitted by this refactor; complete inter assembly, readiness publication for inter blocks, inter deblocking, B-direct and CABAC inter dispatch remain incomplete.

2026-10-06 Complete MBAFF reconstruction readiness: Readiness420 now validates complete macroblock geometry, established pair mode and absence of previously published component blocks without mutation, then can mark Y/Cb/Cr together after successful sample publication. reconstruct_inter_macroblock_ready joins this transaction with residual reconstruction and checked frame/field scattering. Shared intra reconstruction also validates readiness before work and publishes all components only after scattering. Expanded residual tests verify actual availability for every sample in all three components across all addresses, frame/field mode and 8/10bit; duplicate, invalid-plane/depth/sample/QP/address and mismatched-readiness geometry failures leave the picture/readiness unchanged. 143 AVC core and 26 integration tests passed offline, including saved JM intra pixel acceptance. Whole-picture mixed inter assembly/deblocking, B-direct and CABAC inter remain incomplete; no full MBAFF inter playback acceptance is claimed.

2026-10-06 Preserved intermediate CAVLC MBAFF P assembler: decode_p_slices_unfiltered combines pair-aware CAVLC dispatch, motion prediction, explicit reference weights, inter residuals and shared intra reconstruction with slice-local readiness. This explicit pre-deblocking API is not connected to production playback. Full-stream independent pixel acceptance, picture-identity validation and inter deblocking remain pending; passing existing tests does not qualify this new assembler as complete playback support.

2026-10-06 CAVLC MBAFF unfiltered P-picture pixel acceptance: two owned 64x64 three-picture I-P-P streams, frame-unfiltered-cavlc and field-unfiltered-cavlc, use one frame reference and disabled deblocking. Explicit pre-deblocking assembly matches every independent JM Y/Cb/Cr sample across all three pictures and two fresh decode passes. Syntax assertions require 16 macroblocks per P picture, exactly 0 or 16 field blocks respectively, and actual coded inter blocks. Multi-slice picture identity checks reject mismatched frame number, PPS, POC and reference status before assembly. Focused generation --only preserves other fixture manifest records; all 39 stream/oracle hashes verified. This acceptance does not enable production inter playback: inter deblocking, mixed topology/high-depth/multi-slice qualification, B-direct and CABAC inter remain incomplete. Ordinary tests use saved bytes without generator tools or FFmpeg.

2026-10-06 CAVLC MBAFF inter deblocking acceptance: the common pair-address component walker now accepts inter/intra metadata; strength_mbaff implements H.264 8.7.2.1 mixed-mode priority and field vertical-vector threshold. P assembly retains per-cell residual flags and resolved picture/field identities, component QPs and slice filtering controls within its checked allocation budget. decode_p_slices applies this filter to Y/Cb/Cr; its unfiltered counterpart remains explicit. Two new owned 64x64 frame/field three-picture I-P-P single-reference filtered fixtures match every independent JM sample and fresh restart, and same-header pre-filter comparisons prove each P picture exercises filtering. Existing 37 intra/original IP streams remain unchanged; all 41 manifest stream/oracle hashes verified. Walker tests preserve intra output for frame/field/mixed pair modes across all three components at 8/10/12/14-bit and reject missing inter motion before any sample mutation. 145 AVC core tests passed offline. Production decoder still refuses MBAFF inter: persistent MBAFF motion snapshots/DPB dispatch, mixed/high-depth/multi-slice inter qualification, B-direct and CABAC inter remain incomplete. Full inter playback acceptance remains ignored. Normative derivation: https://www.itu.int/rec/dologin_pub.asp?id=T-REC-H.264-201602-S%21%21PDF-E&lang=e&type=items (8.7.2.1).

2026-10-06 CAVLC MBAFF native inter playback admission: persistent ReferenceMotionField now has explicit address-local MBAFF storage, checked pair-mode allocation and selected reference-field parity in ReferenceMotion. snapshot_mbaff_slices resolves each slice's frame-list indices into stable DPB identities, retains expanded field indices/vectors, rejects incomplete data, duplicate/missing mappings and insufficient budget. Physical-sample at_mbaff lookup retains source field mode; ordinary progressive at/colocated explicitly refuses MBAFF data, so incomplete B-direct conversions cannot silently treat field vectors as frame vectors. Unit tests cover every sample of all-field/mixed frame/field storage, both address parities, reference index 63, identity/vector preservation and invalid inputs. Native resolved inter dispatch enables CAVLC MBAFF P assembly/deblocking and stores its snapshot in the DPB. All six saved CAVLC I-P-P streams (original frame/field with default references plus new filtered/unfiltered single-reference frame/field cases) match every JM sample through native playback and rewind. Old CAVLC refusal expectations were replaced by enabled pixel acceptance; only CABAC inter acceptance remains ignored. Full codec unit suite: 467 passed, 1 ignored, offline; selected MBAFF/multislice/parameter-update integration coverage is retained. MBAFF B/direct, CABAC inter, mixed/high-depth/multi-slice inter corpus qualification and broader AVC/HEVC/AAC gaps remain incomplete. This is CAVLC P playback admission, not complete MBAFF/all-codec coverage.

2026-10-06 CAVLC MBAFF mixed/High10/multislice inter qualification: added 12 owned filtered 64x64 three-picture I-P-P single-reference streams: four mixed pair layouts (horizontal and vertical transitions in both directions) and field two-slice at 8-bit, plus frame/field, the same four mixed layouts and field two-slice at 10-bit. Automated syntax checks require the actual pair flags at every macroblock address in all three pictures, 0/8/16 field blocks as appropriate, actual coded P blocks, ordered complete slice coverage and deblocking enabled. High10 coverage requires real 8x8 transforms in the corpus and retained low-bit output precision. Every native playback Y/Cb/Cr sample matches independent JM for all three pictures and two rewind passes. No decoder change was needed for these cases. All 53 stored stream/oracle manifest hashes verified. Ordinary tests use saved bytes and no generator tools or FFmpeg. This qualifies the saved CAVLC patterns; changing pair topology across pictures, broader partition/reference combinations, CABAC inter and B-direct remain unqualified/incomplete.

2026-10-06 CABAC inter context storage foundation: CabacMotionContexts now stores reference-index/MVD condition cells by macroblock address and local 4x4 position; progressive origin, neighbour and partition update paths translate coordinates at the storage boundary. Allocation sizes and progressive syntax decisions remain unchanged. New 2x2-macroblock test assigns distinct context magnitudes to all 64 cells, checks contiguous macroblock ownership and actual cross-block left/top lookups, and confirms slice isolation. 147 AVC core tests passed offline. Pair-aware neighbour derivation/normalization, CABAC skipped-pair flags and the mixed MBAFF reader are still pending; CABAC inter playback acceptance remains ignored.

2026-10-06 CABAC MBAFF P motion context derivation: CabacMotionContexts::new_mbaff, store_non_inter_mbaff and read_prediction_mbaff use pair geometry for A/B neighbours with address-local cells, slice isolation and consistent stored pair modes. Reference-index conditions apply H.264 9.3.3.1.1.6's special field-neighbour ref 0/1 treatment for a current frame block; vertical absolute MVD magnitudes double field-to-frame and halve frame-to-field per 9.3.3.1.1.7. Explicit reference_index_for_field allows expanded lists up to 64 while the existing progressive entry point retains 32. Tests cover both mixed directions, same modes, condition/magnitude context decisions, index 63, actual MBAFF slice isolation, invalid mode/geometry/budget and poisoned entropy failure. The existing owned CABAC frame/field IP fixtures also validate each P picture's first coded skip/field/type/motion prefix with two fresh arithmetic/context readers. These are prefix syntax tests, not full P-picture pixel acceptance. 149 AVC core and 30 selected MBAFF/multislice/parameter-update integration tests passed offline; CABAC inter playback acceptance remains ignored. Complete skipped-pair dispatch, CABAC inter residual/context qualification and assembler admission are still pending; B/direct MBAFF and broader codec gaps remain incomplete. No new media failure class was discovered in this foundation step; saved owned CABAC IP reproducer streams remain the target. Normative source: https://www.itu.int/rec/dologin_pub.asp?id=T-REC-H.264-201602-S%21%21PDF-E&lang=e&type=items (9.3.3.1.1.6 and 9.3.3.1.1.7).

2026-10-06 CABAC MBAFF P playback admission: IntraCabacReader::new_context_mbaff exposes shared mixed arithmetic/residual state; InterCabacSlice::new_mbaff dispatches complete P slices with pair-aware skip/type neighbours, field flags, expanded reference/motion syntax, embedded intra and field residual scans. For a skipped top, it probes the bottom skip/field prefix on a cloned arithmetic bank before publishing top reconstruction mode; the actual bottom later consumes the original bins and checks agreement. Fully skipped pairs infer same-slice left/above mode, defaulting to frame. Pair-field metadata is borrowed alongside arithmetic without per-macroblock grid copies. Existing macroblock pair-aware termination and poisoned-error behavior remain active. Common MBAFF P reconstruction now selects CABAC/CAVLC and budgets CABAC motion/context storage; native decoder dispatch admits both entropy modes and retains their DPB snapshots. Original CABAC frame/field three-picture IP streams now match every JM pixel through native playback and rewind. Their old refusal expectation became reset acceptance, and the previously ignored full CABAC playback test is enabled.

Added three owned 64x64 three-picture CABAC fixtures: frame-inter-skipped, field-inter-skipped and field-inter-topskip. Syntax gates verify actual fully skipped pairs and their left/above/default mode inference, plus skipped top preceding coded field bottom; native output matches every JM Y/Cb/Cr sample across all three pictures and two rewind passes. Static source patterns can still produce isolated coded blocks due to lossy encoder reconstruction, so tests require the intended skipped-pair class rather than claiming all macroblocks are skipped. The topskip generator disables scene cuts to preserve the intended P pictures. Bounded payload truncations prove a failed mixed CABAC dispatcher cannot resume. All 56 fixture/oracle manifest hashes verified. Full codec unit suite: 470 passed, 1 ignored; selected MBAFF/multislice/parameter-update integration tests: 34 passed, none ignored, offline without FFmpeg. CABAC mixed/High10/multislice inter corpus qualification, MBAFF B/direct and wider codec gaps remain incomplete. Normative skipped-pair handling: H.264 7.3.4, 7.4.4 and 9.3.3.1.1.1, https://www.itu.int/rec/dologin_pub.asp?id=T-REC-H.264-201602-S%21%21PDF-E&lang=e&type=items.

2026-10-06 Mixed CABAC MBAFF coded-block-pattern regression and inter qualification: new owned mixed-inter-filtered-cabac I-P-P fixture reproduced a CABAC context failure on the first P picture (after 16 blocks, end_of_slice was not recognized and the reader reported macroblock exceeds picture). JM independently decoded all three pictures. The luma CBP context previously reused one macroblock neighbour pattern for both 8x8 rows; mixed field/frame geometry can change the owning macroblock or select the same source 8x8 block for both rows. read_pattern now resolves each external 8x8 neighbour's actual address/local block via pair-aware sample geometry before selecting its CBP bit, preserving the progressive path. Synthetic context vectors independently verify both mixed directions, the current frame bottom and unavailable neighbours. The previously failing owned fixture now matches every JM sample through native playback and rewind, proving acceptance of the specific failure class rather than a replacement refusal.

Added 14 owned three-picture filtered CABAC I-P-P cases at 8/10-bit: frame, field, four horizontal/vertical mixed transitions in both directions, and field two-slice. Tests require actual field flags and complete address coverage in all three pictures, expected 0/8/16 field block counts, two slice headers when requested, real coded inter blocks and enabled filtering. High10 corpus assertions require real 8x8 transforms and retained low-bit output precision. Every native Y/Cb/Cr sample matches independent JM and repeats after rewind. All 70 stream/oracle manifest hashes verified; generation remains explicit and ordinary tests offline without FFmpeg or generator tools. 150 AVC core and 35 selected MBAFF/multislice/parameter-update integration tests passed. CABAC/CAVLC P support is qualified for this stored corpus; MBAFF B/direct, additional temporal topology/reference/partition combinations and wider AVC/HEVC/AAC gaps remain incomplete. Normative CBP derivation: H.264 6.4.11.2 and 9.3.3.1.1.4, https://www.itu.int/rec/dologin_pub.asp?id=T-REC-H.264-201602-S%21%21PDF-E&lang=e&type=items.

2026-10-06 MBAFF B-direct co-located foundation: ReferenceMotionField::colocated_mbaff implements the AFRM/AFRM rows of H.264 Table 8-8 in address-local storage. Frame-to-field references select the nearest source field by full-range signed POC distance (ties choose bottom), then map the current macroblock half to the source field's 4x4 row. Field-to-frame references select the source macroblock half from local y and double its local row. Same-mode mappings preserve the address and row. L0 preference, L1 fallback and intra cells retain raw motion/reference metadata. ColocatedScale exposes checked temporal-only vertical conversion; map_reference_mbaff maps decoded-picture identity and retains field parity where required, including expanded index 63. spatial_direct_for_field accepts normalized current field lists while validating the co-located list in its own mode; spatial colZeroFlag deliberately uses the raw vector, never the temporal conversion. The progressive wrapper retains its limits and behavior.

Synthetic unit vectors exhaust all four current/source mode combinations, every 4x4 position in an eight-macroblock mixed snapshot, both macroblock parities, nearest-top/bottom/tie/extreme POCs, source L0/L1/intra selection, all 32 frame identities and both reference field parities, negative division and checked doubling boundaries. 157 AVC core tests passed offline. This is normative helper qualification, not compressed MBAFF B acceptance: decoder dispatch remains P-only. Per-field POC persistence/DPB plumbing, direct motion publication, entropy B dispatch and owned spatial/temporal B-picture fixtures remain to connect. No new compressed-media failure was discovered in this helper step. Normative source: H.264 8.4.1.2.1–8.4.1.2.3, https://www.itu.int/rec/dologin_pub.asp?id=T-REC-H.264-201602-S%21%21PDF-E&lang=e&type=items.

2026-10-06 MBAFF direct field-POC context: DecodedReferencePicture retains the complete post-marking FieldOrder beside its image/motion, so both field POCs survive DPB retention and MMCO 5 adjustment rather than being replaced by the frame minimum. MbaffDirectPrediction composes the previous Table 8-8 selector, raw spatial colZero rules, reference identity/parity mapping and checked temporal vector conversion. It requires complete field orders aligned with each active frame list and verifies their minima against frame POC metadata. Temporal mode selects current/L1 fields by macroblock parity and L0 fields by that parity XOR the expanded reference index; frame mode uses frame minima. Direct 8x8 inference selects the outer 4x4 corners before co-located mapping. No heap allocation is needed for identity mapping.

Independent scalar expectations exercise distinct top/bottom POC distances, opposite-parity temporal references, all frame/field conversion combinations, intra-only co-located references, inference enabled/disabled, expanded reference index 63, missing identity, incomplete/misaligned field orders, invalid coordinates and POC mismatches. The decoder's existing retention/eviction test now checks that retained field timing matches its DPB entry. 159 AVC core tests passed offline. This context is not yet invoked by MBAFF compressed B-picture reconstruction: motion publication/rollback, entropy B dispatch, two-list reconstruction/weights and saved synthetic spatial/temporal B acceptance still need to be connected. Full codec coverage remains incomplete.

2026-10-06 MBAFF B-direct motion publication: decode_macroblock_mbaff_with_direct combines explicit L0/L1/Bi and inferred direct 4x4 partitions in one address-local transaction. It captures normalized macroblock-partition-0 external neighbours before publishing any partition; earlier explicit blocks cannot contaminate spatial direct. Direct syntax, context geometry and complete coverage are validated. Late syntax/coverage/publication failures clear only the current macroblock and restore the prior progressive/MBAFF storage mode, preserving completed neighbours. The existing P-only wrapper delegates without a direct context. CABAC read_prediction_mbaff_for_slice exposes the existing P/B syntax engine with pair-aware context lookup; the P wrapper remains unchanged, both B lists consume their own ref/MVD syntax, and direct publishes zero CABAC conditions without consuming motion bins.

Synthetic motion transactions cover mixed explicit/direct publication in both frame and field modes, missing context, late invalid direct syntax, incomplete coverage, mode rollback before the first publication, neighbour preservation after MBAFF storage exists, slice isolation and complete snapshots. A temporal direct transaction independently verifies derived top/bottom vectors and reference indices/parities after snapshot selection in physical frame coordinates. Scripted CABAC vectors prove two-list zero-MVD consumption and no-bin direct behavior in both modes. 162 AVC core and 35 selected MBAFF/multislice/parameter-update integration tests passed offline. These APIs are not yet called for compressed MBAFF B pictures: assembler two-list compensation/weights, field-aware entropy slice dispatch, native decoder wiring and saved synthetic spatial/temporal B pixel acceptance remain incomplete. No new compressed-media failure class was discovered in this foundation step; existing owned MBAFF P regressions remain passing.

2026-10-06 MBAFF B weighting/reconstruction foundation: mbaff_weights resolves partition/list/component weights for P and B motion. Explicit tables use ref_idx >> 1 for field macroblocks, independently for both lists. Implicit B weighting uses current macroblock parity for current POC and current parity XOR each list's expanded reference parity for reference POCs; complete field metadata and its frame minima are validated. Frame blocks retain frame POCs, long-term references retain default equal weights, and a single-list implicit predictor remains unweighted. The existing MBAFF P assembler now uses this shared resolver in place of duplicated explicit-P weight construction.

Synthetic two-list compensation tests verify every Y/Cb/Cr sample for distinct top/bottom field POCs, same-parity top/bottom references, opposite-parity references with negative weights, explicit frame-index selection for expanded field ref_idx=2, and independent component offsets/denominators. Explicit offset scaling and pixel results are checked at 8/10/12/14-bit. Missing weight tables/context, misaligned field metadata and long-term default weights are exercised. 163 AVC core and 35 selected MBAFF/multislice/parameter-update integration tests passed offline, including all saved CABAC/CAVLC P fixtures. This qualifies two-list weighted predictors, not compressed MBAFF B playback: entropy B slice admission, assembler direct wiring, native DPB context plumbing and owned spatial/temporal B pixel-oracle fixtures remain incomplete. Normative source: H.264 8.4.2.3 and 8.4.3, https://www.itu.int/rec/dologin_pub.asp?id=T-REC-H.264-201602-S%21%21PDF-E&lang=e&type=items.

2026-10-06 Native MBAFF B-picture playback admission: the common MBAFF assembler now accepts P/B slices, derives B-skip as direct 4x4 partitions, invokes transactional explicit/direct motion publication, resolves shared P/B weights, reconstructs residual/intra/inter blocks and deblocks both lists. CABAC MBAFF slice admission accepts P/B and passes the actual slice type to pair-aware motion syntax; CAVLC's existing MBAFF B syntax is reused. AvcDecoder obtains complete stored FieldOrder metadata for each modified DPB list, builds per-slice MbaffDirectPrediction contexts and dispatches adaptive P/B frame pictures to the assembler. Progressive dispatch remains separate. An owned two-macroblock CAVLC B-skip payload checks whole-picture spatial/temporal, ordinary/implicit-weighted samples and retained snapshots.

Added eight owned 64x64 three-picture I-B-P display-order streams: frame/field, spatial/temporal, CAVLC/CABAC. Explicit generation uses x264 and independent JM, never FFmpeg/private media. Each saved stream's syntax gate requires adaptive-frame SPS, exactly one B picture, the intended direct mode, real direct partitions, complete coverage and field-coded blocks for field cases. Native software playback matches every independent JM byte in all three output pictures and repeats after rewind. All 78 stream/oracle hashes verified. This is acceptance for the saved 8-bit frame/field B corpus; mixed B layouts, High10/multislice, broader reference/partition combinations, changing pair topology, separate field pictures and wider AVC/HEVC/AAC gaps remain unqualified/incomplete. Ordinary tests use stored bytes without external codecs or network.

Admission verification: complete codec unit suite 481 passed, 1 ignored; selected MBAFF/multislice/parameter-update integration suite 36 passed, none ignored; media/player all-targets check passed. Changed Rust files match edition-2024 formatting, Python generator compiles, and git diff --check passes. These checks do not prove the remaining codec profiles/tools or platform-specific playback.

2026-10-06 Extended MBAFF B qualification: added 48 owned moving 64x64 three-picture streams spanning four mixed horizontal/vertical pair layouts in both directions, field multislice and the complete frame/field/mixed/multislice High10 matrix, each with spatial/temporal direct and CAVLC/CABAC. Qualification exposed a fixture-coverage issue, not a decoding failure: the moving High10 temporal field-multislice stream selected explicit biprediction for every B block. That stream was preserved, and four additional owned static temporal field-multislice cases at 8/10-bit ensure real B-skip/direct acceptance for those categories. Direct coverage is required per topology/mode/depth/entropy key across moving/static streams rather than assuming every slice or a signalled direct mode uses direct partitions.

All 60 B streams match every independent JM display-order sample across three pictures and two rewind passes. Syntax gates verify actual MBAFF SPS, implicit B weighting in PPS, exactly one B picture, selected direct mode, direct syntax per category, exact pair flags/0-8-16 field block coverage and ordered complete slices in all I/P/B coded pictures, one/two slice counts, genuine High10 B 8x8 transforms in the corpus and retained low-bit precision. All 130 stored stream/oracle manifest hashes verified. No decoder changes were required for this extended corpus. The current selected MBAFF/multislice/parameter-update suite passed 36 tests offline; generator Python compilation, changed Rust formatting and git diff --check passed. Multiple references, reference B pictures/B pyramid, changing pair topology across pictures, explicit B-weighted compressed fixtures, separate field pictures, broader AVC profiles/chroma/FMO/ASO and remaining HEVC/AAC tools remain incomplete or unqualified. This corpus qualification does not establish universal codec coverage.

2026-10-06 MBAFF multiple-reference/B-pyramid qualification: added 24 owned nine-picture moving streams for frame/field/mixed geometry, 8/10-bit, spatial/temporal and CAVLC/CABAC, generated with three configured references and fixed three-B groups/normal B-pyramid. Gates require exactly one I, two P and six B pictures, exactly two retained reference B pictures, multiple active references, actual nonzero frame reference indices in coded B partitions (field parity alone does not count), complete geometry/slices, and a retained B selected as L1[0]. A parallel metadata-only PocDecoder/ReferenceBuffer traversal applies once-per-picture marking and verifies that direct partitions actually use a co-located B picture in the intended mode for every category; this proves the retained-B motion path rather than merely counting reference B headers.

Fixture qualification exposed encoder choices, not new decoder failures: temporal-requested pyramid streams can signal spatial for some pictures, and field High10 CABAC plus mixed 8/10-bit CAVLC moving cases did not exercise the intended direct-from-B path. Those moving streams were retained. Three additional mostly-static temporal fixtures preserve moving anchor pairs and provide the missing direct path for the same categories. Actual header modes are inspected; temporal category acceptance requires real temporal direct from B motion, not just the encoder option or other spatial partitions. All 27 new streams match every saved independent JM output sample in nine displayed pictures, repeated after rewind. Playback gates additionally check exact consecutive presentation ticks and unit durations against each MP4 track timescale. No decoder changes were needed for this corpus. All 157 stored stream/oracle pairs verified; current selected integration suite passed 36 tests offline, generator Python compilation/format/diff checks passed. Changing pair topology across pictures, broader reference/partition combinations, explicit B-weighted compressed fixtures, separate field pictures and remaining AVC/HEVC/AAC capabilities remain incomplete/unqualified.

2026-10-06 Changing MBAFF topology and owned temporal-direct qualification: eight nine-picture x264/JM streams switch pair modes across pictures at 8/10-bit with CAVLC/CABAC and spatial/temporal encoder requests. Every saved pixel, presentation timestamp and rewind is checked. Actual direct-from-B coverage is required for the changing spatial categories; temporal requests alone do not qualify the changing temporal categories.

Eight additional owned 16x32 five-picture CAVLC streams force nonzero temporal direct from retained B motion across both field-to-frame and frame-to-field transitions, at 8/10-bit with implicit or explicit B weights. The new generate_avc_mbaff_direct_samples.py writes SPS/PPS, PCM and P/B syntax itself; JM independently supplies saved pixel oracles. Tests verify reference identities, pair modes, motion syntax, actual direct partitions, weight tables, clipping, every output sample and rewind. Ordinary tests use saved bytes only. The combined manifests contain 173 stream/oracle pairs. Changing-mode CABAC temporal direct, separate field pictures and broader AVC/HEVC/AAC capabilities remain unqualified or incomplete.

Delivery verification: 37 selected MBAFF/multislice/parameter-update integration tests passed offline (22/12/3), none ignored. The complete codec unit suite previously passed 481 tests with one ignored on this production implementation; 173 manifest pairs verified.

2026-10-06 Owned CABAC cross-mode temporal direct qualification: the independent PCM/header generator now implements a short integer CABAC interval writer using normative probability/transition/context initialization tables. It keeps the previously qualified CAVLC I/P/reference-B anchors and selects a second CABAC PPS for the non-reference temporal-direct B target. Thirty-two new 16x32 five-picture streams cover frame-to-field/field-to-frame, 8/10-bit, implicit/explicit B weighting, cabac_init_idc 0/1/2, and a skipped top macroblock whose pair mode is signalled by the coded bottom. JM independently decodes every generated stream; ordinary acceptance uses saved bytes and no external codecs/network.

The CABAC syntax gate requires actual temporal B-direct partitions, exact target pair mode, complete pair termination and the expected skipped-top variant; metadata-only POC/DPB traversal requires retained B picture 2 as L1[0] and I picture 0 as L0[0]. Native playback matches every JM Y/Cb/Cr byte across all five display-order pictures, exact presentation ticks and rewind. All 205 combined stream/oracle manifest pairs verified. Generator debugging corrected its table parser and CABAC terminal bit placement; no new production decoder failure was found. This closes the previously unqualified owned cross-mode CABAC temporal-direct case, including bottom-signalled mode probing, but not universal AVC support: separate field pictures, FMO/ASO, mixed slice types, wider chroma/profiles and HEVC/AAC gaps remain.

Verification for the CABAC qualification: 38 selected integration tests passed offline (23 MBAFF, 12 multislice, 3 parameter updates), none ignored. Python compilation, changed Rust formatting and git diff --check passed. Production decoder sources were unchanged in this qualification step.

2026-10-06 CABAC retained-B motion and higher-depth cross-mode qualification: the owned interval writer now emits B_L0_16x16 reference pictures with nonzero MVD (8,4), field reference index 1, pair-mode context selection, unary motion bins and bypass signs. Thirty-two new 8/10-bit streams retain that CABAC B motion before the CABAC temporal-direct target, covering all three cabac_init_idc banks, explicit/implicit weights and bottom-signalled pair mode after a skipped top. Source syntax gates now inspect actual entropy, L0 prediction/reference index, both MVDs, pair mode, exact coverage/termination and retained B identity, in addition to target direct and full pixel/rewind acceptance.

The same corpus was expanded to 12/14-bit equal-depth 4:2:0 using profile 244 (High 4:4:4 Predictive), with owned PCM gradients retaining lower-bit precision. These are 4:2:0 prediction/PCM qualifications under that profile, not 4:4:4-chroma or residual-tool acceptance. Combined owned direct manifest: 144 streams (16 CAVLC targets, 128 CABAC targets), 104 newly added since the previous 40-case step. All generated streams decoded independently with JM; every FVid output byte and rewind matches. Native tests also require profile/depth/chroma identity, in-range oracle samples and nonzero low-bit precision. All 309 combined stream/oracle manifest pairs verified. No production decoder change was required; separate field pictures, mixed slices, FMO/ASO, wider chroma/residual/profile tools and HEVC/AAC gaps remain.

Verification for CABAC source/high-depth qualification: 38 selected integration tests passed offline (23 MBAFF, 12 multislice, 3 parameter updates), none ignored. Python compilation, Rust formatting and git diff --check passed. Production decoder sources were unchanged.

2026-10-06 Native mixed MBAFF I/P slice admission: a new owned 32x32 three-picture reproducer first failed on picture 1 with "invalid MBAFF P reconstruction configuration" because the common assembler excluded I slices. After intra dispatch was connected, the same stream exposed a second refusal, "MBAFF P slices belong to different pictures": I/P slices had nal_ref_idc 2/3, which are both reference slices in one picture. H.264 7.4.1 and 7.4.1.2.4 distinguish reference identity only when one value is zero; the assembler now uses that same zero/nonzero rule already applied by access-unit preparation.

The common MBAFF reader now dispatches I slices through owned intra CAVLC/CABAC readers and publishes them through existing shared intra reconstruction, readiness, no-prediction motion and deblocking metadata alongside inter slices. The public resolved reconstruction facade routes MBAFF I/P slices to this common assembler. The new generate_avc_mbaff_mixed_samples.py writes owned PCM, nonzero P motion and slice headers; its explicit JM generation is separate from ordinary tests. CABAC generation supports I_PCM arithmetic restarts retaining adapted contexts and owned P_L0_16x16 motion. No private media/parameters or FFmpeg are used.

Forty-eight saved acceptance streams cover I/P and P/I order, horizontal frame/field and field/frame pairs, 8/10-bit, CAVLC/CABAC and deblocking idc 0/1/2. Syntax gates require actual I_PCM/P motion, pair modes, MVD/reference indices, complete slice address/termination and the intended filter mode; CAVLC cases require distinct nonzero NAL reference priorities. The mixed picture is retained and used by a subsequent P picture. Every JM output byte matches native display-order playback and rewind; the mixed output also matches through the public reconstruction API. All 357 combined manifest pairs verified. This is mixed I/P PCM/prediction acceptance, not mixed B/intra residual qualification or universal AVC coverage. Separate field pictures, FMO/ASO, additional chroma/profiles/residual tools and HEVC/AAC gaps remain.
Normative reference: https://www.itu.int/rec/dologin_pub.asp?id=T-REC-H.264-201602-S%21%21PDF-E&lang=e&type=items.

Mixed I/P filtering qualification uses P slice QP 50 (I/IDR headers retain QP 26). Each of the 16 topology/order/depth/entropy categories requires identical PCM anchor bytes but different mixed-picture bytes between deblocking idc 0 and 2. This isolates actual cross-slice I/P filtering instead of counting differences that arise only in the following P picture. The QP-26 initial generator did not activate filtering in the mixed picture; QP 50 does, and independent JM/FVid output agrees.

Final mixed-slice verification: 481 fvid-codecs unit tests passed, 1 previously ignored; 235 frontend codec/audio tests passed; 39 selected integration tests passed (24 MBAFF, 12 multislice, 3 parameter updates), none ignored. Native codec and final integration tests ran offline. The media/player all-targets check passed; changed Rust formatting, Python generator compilation and git diff --check passed. The updated generator reproduces all 144 direct streams byte-for-byte; all 357 manifest pairs verify after the active mixed-filter fixture update.


Mixed MBAFF I/B, B/I, P/B and B/P qualification adds 192 owned streams (240 mixed streams total), including retained motion, temporal direct, 8/10-bit, CAVLC/CABAC, explicit/implicit weighting and active cross-slice filtering. Every saved JM output byte matches native playback and rewind.

Frame-number gap work remains a foundation: DPB stores non-existing reference slots without allocating pixels, POC types 1/2 infer history, and type 0 excludes unknown-POC gaps from B lists. Nine new unit tests pass. The native decoder still explicitly refuses actual gaps until reference availability is connected; 24 owned gap fixtures reproduce that specific refusal, not playback acceptance. These fixtures and generators contain no private media and ordinary tests require neither FFmpeg nor JM.


Native frame-number gap admission now connects the owned DPB/POC foundations. SPS permission is required; every missing frame number occupies a non-existing sliding-window slot with no image allocation, receives a unique reference ID, and updates POC types 1/2. All 24 saved frame/field-MBAFF, 8/10-bit, CAVLC/CABAC and POC 0/1/2 streams now pass byte-exact JM acceptance for all three pictures over two decoder resets. Syntax gates require frame_num 0/3/4. The former refusal regression is replaced by this acceptance test.

Remaining gap limitation: active reference lists containing unused non-existing slots are still refused during view resolution. Prediction from non-existing pictures remains invalid; supporting unused slots requires optional reference availability in reconstruction and direct-motion paths. This change does not claim complete gap admission or separate field-picture support.


Unavailable reference sampling foundation: Reference420 now carries optional planes. Non-existing entries allocate no samples, propagate through field views, and refuse every sampling request before writing output. A regression exercises 8/10/12/14-bit and both parities, preserving sentinel output samples on failure. This is not yet connected to optional active-list picture views; unused gap entries therefore remain refused by the decoder.


Unused gap reference admission: optional picture views preserve non-existing active slots through progressive/MBAFF reconstruction. Twenty-four additional owned streams retain two unused missing references alongside the real IDR (48 gap streams total). Syntax gates require three active L0 entries; every output pixel matches JM across two resets, 8/10-bit, CAVLC/CABAC, frame/field MBAFF and POC 0/1/2. A separate owned two-picture CAVLC fixture deliberately selects a missing frame and must fail specifically at prediction, then require reset. No synthetic pixels are substituted for missing references.

MBAFF B lists with missing entries still need stored inferred per-field POCs; co-located missing references are rejected instead of being treated as intra pictures. Separate field pictures, FMO/ASO and broader profile/tool gaps remain.


MBAFF B gap admission now retains exact inferred top/bottom field POCs in the DPB. Thirty-two owned I/B/P streams cover explicit L0 and temporal direct, 8/10-bit, CAVLC/CABAC, frame/field pairs and POC types 1/2. Both active lists include the real IDR and two non-existing slots; output matches every JM pixel over two resets. Direct/explicit output differs in all 16 paired categories. Missing co-located pictures are explicitly rejected; absent images are never treated as intra. A DPB test preserves asymmetric field POCs, rejects partial-frame metadata atomically and verifies eviction.

The new non-reference B transition also exposed and fixed PrevRefFrameNum history: inferred gap frames advance the previous reference number even when the current picture is non-reference, preventing duplicate inference before the subsequent P with the same frame_num. These fixtures reproduce that exact transition. Remaining codec coverage is not complete: separate field pictures, FMO/ASO and further profiles/tools still require implementation and acceptance.


ASO admission for single-group AVC: access-unit preparation validates all picture identities, normalizes slices by first_mb, rejects duplicate starts and missing macroblock zero, then computes reconstruction ranges before updating POC/DPB. Wire order is no longer required to be raster order. The former reversed-slice refusal test now verifies normalized ranges and byte-identical progressive I/P/B reconstruction.

Seventy-two new owned reversed-order MBAFF fixtures cover I/P, P/I, I/B, B/I, P/B, B/P, 8/10-bit, CAVLC/CABAC and deblocking idc 0/1/2. Every packet must contain addresses [1,0] on wire and normalize to [0,1]. All JM display-order pixels match native decode over two resets, including retained mixed pictures and temporal direct. Their JM outputs equal the corresponding independently saved raster-order outputs. The generator explicitly invokes JM separately; ordinary tests remain offline with no FFmpeg/JM. This does not implement FMO address maps or separate field pictures.


FMO address foundation: the owned avc_slice_group_map module implements map types 0–6 (interleaved, dispersed, foreground precedence, box-out, raster, wipe and explicit), bounded cycle/rate handling, frame/field/MBAFF macroblock conversion and NextMbAddress. Five tests cover fixed spatial examples, all cycles/rates/directions on 1×1 through 9×9 maps, asymmetric foreground overlap, address skipping and malformed dimensions/maps/budgets. Implementation follows H.264 8.2.2.1–8. This is a map/address foundation only: FMO slice readers and reconstruction are still explicitly unsupported, and no video acceptance is claimed before synthetic fixtures and decoder integration.


FMO CAVLC reader integration begins with an explicit syntax-only new_fmo constructor. It derives and expands the real PPS map and uses NextMbAddress after PCM and coded intra blocks, retaining coefficient/mode availability at physical addresses. Existing playback constructors still refuse FMO until reconstruction is connected. Two compact owned progressive explicit-map I_PCM fixtures (ordinary and reversed slice order) use groups [0,1,0,1]; readers must emit [0,2] and [1,3], terminate each slice, cover all four addresses exactly once and reproduce every saved JM PCM pixel. This is syntax acceptance plus a separate passing playback refusal, not FMO playback acceptance. Generation is separate and invokes JM explicitly; tests use only saved fixtures, no external decoder or private data.


Native progressive FMO I-picture reconstruction now tracks coverage and slice ownership by physical macroblock address, preserves per-slice prediction availability, and invokes the FMO reader. Deblocking receives explicit owners instead of inferring boundaries from first_mb ranges; idc 2 compares real slice ownership. Owner/map scratch storage is included in picture memory accounting. Slice identity validation now includes slice_group_change_cycle.

Twenty owned I_PCM fixtures cover map types 0–6, both changing-map directions and both slice wire orders. Every saved JM pixel matches AvcDecoder over two resets. The old native refusal in the explicit-map test is replaced with pixel acceptance; the legacy syntax constructor remains separately restricted. A missing-group regression verifies that no partial picture is returned, errors require reset and a valid frame succeeds after reset. This qualifies progressive 8-bit I_PCM FMO, not all intra prediction/residual tools, filtered FMO pixel cases, MBAFF FMO, or P/B FMO. These broader paths remain open and require their own fixtures.


Progressive FMO I16 DC/residual/filter qualification: 120 owned one-picture PCM/I16 streams cover all map types 0–6, both changing-map directions, forward/reversed slice order, zero/one positive luma DC coefficient and deblocking idc 0/1/2. Syntax gates require two PCM and two actual I16 DC blocks, exact DC coefficient count/sum, QP 50 and active +12 offsets. Every JM output pixel matches the native decoder over two resets. All 40 idc 0/2 paired categories require equal parsed PCM anchors but distinct saved output pixels, proving active cross-slice filtering. No production code changes were needed: the owned FMO reconstruction/filter-owner implementation passes this expanded acceptance.

This extends the former PCM-only qualification; it does not yet qualify arbitrary intra modes, AC/chroma residual combinations, higher depths, MBAFF FMO or P/B FMO. Ordinary tests consume saved fixtures only; the explicit JM generator uses no private media or FFmpeg.


FMO P/B CAVLC reader foundation: new_fmo derives the real map, skips foreign-group addresses after coded/intra/skip blocks, and validates skip run against a precomputed same-group suffix count. Remaining-group lookup is O(1), and added map/count context is charged to the reader budget. FMO mixed intra context uses the same physical address map. Ordinary constructors retain their previous behavior.

Two compact owned Extended-profile I/P/B fixtures (forward/reversed slice order) retain two references, include nonzero P/B MVDs and per-group skip runs; JM decodes all three pictures. Syntax acceptance requires correct picture types, coded MVD [8,4]/[4,0], zero coefficients, addresses [0,2]/[1,3], complete coverage and termination. A third owned malformed fixture requires the exact skip-run-out-of-group refusal and reader poison. A separate native refusal test documents that FMO P/B reconstruction remains unsupported; saved JM pixels are not yet counted as native playback acceptance. All 20 earlier PCM streams remain byte-identical after optional profile/POC/reference configuration was added to the generator.


Native progressive FMO P/B reconstruction now tracks unique coverage at physical addresses, writes loop-filter metadata into physical row/column slots and uses the FMO inter reader. FMO rows are published to reconstruction workers only after successful complete parsing; ordinary raster row pipelining remains enabled. Motion/prediction availability still follows each slice's identity, and replay preserves intra/inter decode order. The old playback refusal is replaced by full acceptance.

Six owned Extended-profile I/P/B streams cover forward/reversed order and deblocking idc 0/1/2. Every JM display-order pixel matches native decode over two resets. Filtered streams use QP 50/+12 offsets and distinct group MVDs; both idc 0/2 pairs require identical IDR pixels but different B and P top-row luma pixels. Initial identical-motion fixtures had zero boundary strength; final fixtures deliberately activate cross-slice filtering instead of weakening the gate. A malformed skip-run fixture must fail at group bounds through syntax and native decode, poison state, and allow a valid IDR after reset. This qualifies explicit-map progressive 8-bit motion/skip paths; other map types, residual/intra combinations, depths and MBAFF FMO still need acceptance.


FMO P/B map-type qualification now includes all map types 0–6 and both changing-map directions: 60 owned three-picture I/P/B streams cover both slice orders and filter idc 0/1/2. Groups may contain one, two or three macroblocks; skip runs derive from group length instead of assuming adjacent raster addresses. Syntax gates verify SPS Extended profile/two references, actual P/B types, each group's coded MVD, remaining skip addresses, unique complete four-macroblock coverage and termination. Every saved JM pixel matches native decode across two resets. All 20 filtered pairs require equal IDR but distinct B/P top-row luma output, proving active cross-slice filtering for every map topology.

Scope correction from normative H.264 A.2: Extended FMO uses 8-bit 4:2:0 and CAVLC; High-family profiles constrain num_slice_groups_minus1 to zero. Higher-depth/CABAC FMO combinations are not standard High-family bitstreams and should not be presented as missing FMO conformance. Valid MBAFF/field FMO, additional group counts, intra/residual tools and the broader non-FMO profile gaps still remain. Qualification here covers two-group progressive motion/skip FMO only.


Three-to-eight-group FMO qualification adds 144 owned 64×64 three-picture Extended-profile I/P/B fixtures for map types 0/1/2/6, every group count 3–8, forward/reversed group wire order and filter idc 0/1/2. Actual coded motion and all remaining skipped physical addresses are gated against each PPS map; every picture must cover all 16 addresses exactly once and terminate every slice. Every JM pixel matches native playback over two resets. All 48 filtered pairs require identical IDR pixels and distinct B/P luma planes. This exercises the highest group ID, non-power-of-two explicit maps, single-MB foreground groups and long skips in the remainder group. Changing map types 3–5 are defined only for two groups and retain their prior two-group acceptance.

No production fixes were required for these additional group counts. FMO residual/intra combinations and MBAFF/field pictures remain open, alongside broader codec/profile tools. Ordinary tests consume saved synthetic clips/oracles without FFmpeg, JM or network access.


Progressive mixed FMO inter/intra residual acceptance adds 60 owned Extended-profile I/P/B streams across map types 0–6, both changing-map directions, both slice orders and filter idc 0/1/2. P/B slices contain a coded motion block followed by an embedded I16 DC block with one positive luma DC coefficient (and remaining group skips where present). Syntax gates require actual P/B types, two coded inter blocks, nonzero I16 DC coefficients and unique complete physical coverage. All display-order JM pixels match native decode over two resets. All 20 filter pairs require identical PCM IDR anchors and distinct B/P luma output between idc 0 and 2.

Generation explicitly invokes restored official JM 19.0 outside ordinary tests; saved fixtures are synthetic and have verified hashes. No production fix was needed. This qualifies the specific mixed motion/I16 DC combination; arbitrary AC/chroma residuals, additional intra modes, changing-map rates/cycles, MBAFF/field FMO and broader codec/profile tools remain unqualified. Ordinary tests use saved fixtures without FFmpeg, JM or network.


Mixed progressive FMO AC/chroma acceptance adds 180 owned Extended-profile I/P/B streams covering all map types 0–6, changing-map directions, forward/reversed slice order and deblocking idc 0/1/2. Three residual modes exercise embedded I16 luma AC (16 nonzero coefficients), chroma DC (two nonzero coefficients), and combined luma/chroma AC (16 luma, two chroma DC and eight chroma AC coefficients). Luma/chroma AC signs alternate. Syntax checks require the exact coefficient counts, signed luma AC, real P/B slice types, coded inter blocks and complete unique physical coverage. Every display-order pixel matches independent saved JM output over two resets. All 60 filter pairs preserve the IDR anchor and change both B/P luma planes between idc 0 and 2.

No production fix was needed. The generator is owned and invokes JM explicitly only during generation; tests consume saved synthetic fixtures and require no FFmpeg, JM or network. This covers single nonzero first-AC levels within embedded I16 blocks, not arbitrary levels/scan runs, every intra prediction mode, residual-bearing inter blocks, changing-map rates/cycles, MBAFF/field FMO, or complete codec/profile conformance. Those remain open.


Progressive FMO inter residual acceptance adds 180 owned Extended-profile I/P/B streams with real motion and signed residuals in coded inter macroblocks, followed by group skip runs. Modes cover 16 nonzero luma 4x4 coefficients, two nonzero chroma DC coefficients, or their combination with eight nonzero chroma AC coefficients. Every map type 0–6, changing-map direction, slice wire order and filter idc 0/1/2 is exercised. Syntax gates require coded inter dispatch (no embedded intra), nonzero MVD, exact residual counts/signs and unique complete coverage. Every JM display-order sample matches native decoding across two resets. All 60 idc 0/2 pairs retain equal IDR anchors and change both B/P luma planes.

No production fixes were required. These use one nonzero first coefficient per selected block with alternating signs; arbitrary levels, scan positions/runs, additional prediction/partition modes, changing-map rates/cycles, MBAFF/field FMO and broad codec/profile conformance remain open. Generation invokes JM separately; ordinary saved-fixture tests require no FFmpeg, JM or network.


MBAFF FMO intra admission: an owned 32x64 explicit-map Extended-profile PCM stream reproduced the exact native refusal "intra CAVLC reader requires progressive 4:2:0 I slices without FMO". The MBAFF intra assembler now selects the FMO reader, verifies unique complete physical coverage and stores filter metadata by physical macroblock address instead of parse order. Coverage storage is charged to the picture budget. Ordinary raster range checks remain enabled outside FMO. The CAVLC reader records the last emitted address so field_decoding reports that pair's mode after a non-raster group jump rather than inferring it from next_address minus one.

Eighteen synthetic PCM streams exercise four pairs with map [0,1,0,1], frame/field/mixed pair modes, both wire orders and filter idc 0/1/2. Syntax gates require exact physical addresses [0,1,4,5]/[2,3,6,7], actual PCM, each pair's field mode, complete unique coverage and termination. Every JM output sample matches native decode over two resets. PCM-only streams qualify filter metadata/configuration, not active residual filtering. MBAFF FMO predicted/residual intra, P/B, other maps and separate field-picture admission still require acceptance. Generation explicitly invokes JM separately; ordinary tests require no external decoder, FFmpeg or network.


MBAFF FMO predicted intra qualification adds 144 owned 32x64 Extended-profile explicit-map streams. The first two pairs contain PCM anchors; the later two pairs use I16 DC/vertical luma and DC/vertical chroma prediction with zero or one positive luma DC coefficient at QP 50. Frame, field and mixed pair layouts, both wire orders and filter idc 0/1/2 are covered. Syntax checks require exact physical pair addresses/modes, PCM versus I16 dispatch, luma/chroma prediction modes, exact DC count/sum, QP and complete termination. Every saved JM sample matches native decoding over two resets. All 48 idc 0/2 pairs change output; 12 unfiltered pairs independently require luma and chroma prediction-mode changes to affect their respective planes.

No additional production fixes were required beyond the preceding physical-address MBAFF FMO admission. This qualifies DC/vertical prediction with first-DC residuals, not arbitrary intra modes/AC/chroma residuals, other maps, P/B FMO, or separate field pictures. Fixtures and saved references are synthetic; explicit JM generation remains separate from ordinary offline tests without FFmpeg/JM/network.


MBAFF FMO inter admission connects the common intra/inter assembler to FMO CAVLC readers. Owned three-picture streams first reproduced the common reader's FMO refusal. The assembler now stores deblocking metadata by physical address, validates unique complete coverage, retains ordinary raster checks outside FMO and includes additional FMO reader/coverage storage in its budget. InterCavlcSlice records its last emitted address for field_decoding across group jumps, matching the earlier intra-reader correction.

Eighteen Extended-profile I/P/B streams cover four explicit-map pairs [0,1,0,1], frame/field/mixed modes, both wire orders and filter idc 0/1/2. They retain PCM IDR and motion P references; B pictures use explicit L0 motion. Syntax gates require P/B dispatch, every physical address, MVDs, pair modes, zero coefficients, unique coverage and termination. Every JM display-order sample matches native decode across two resets. Six idc 0/2 pairs require equal IDR and distinct B/P luma output. This qualifies explicit motion without residual/direct/skip mixtures; remaining map types, MBAFF FMO residual/intra/direct combinations, separate field pictures and broader codec tools remain open. Ordinary tests use saved owned fixtures without FFmpeg/JM/network.


MBAFF FMO skip/direct qualification adds 18 owned Extended-profile I/P/B streams with coded nonzero-motion top macroblocks and P-skip/B-skip bottom macroblocks in each pair. Explicit map [0,1,0,1], frame/field/mixed pair modes, both wire orders and filter idc 0/1/2 are covered. Syntax gates require exact coded/skip parity and physical addresses, actual P/B slices, top MVDs, pair modes, complete coverage and termination. Every JM display-order sample matches native decoding over two resets. Six filter pairs preserve IDR anchors and change B/P luma; every B luma output differs from its corresponding previously saved explicit-motion stream, isolating active temporal direct behavior.

The initial writer incorrectly emitted a second mb_skip_run before the coded block following a skip. It was corrected before fixture acceptance; the final JM generation completes all pictures. No production fixes were needed beyond the preceding physical-address common-assembler and field-mode corrections. Top-skip/whole-pair skips, explicit/spatial direct, residual/intra mixes, other FMO maps and separate field pictures remain open. Ordinary saved-fixture tests require no FFmpeg/JM/network.


MBAFF FMO inter residual qualification adds 54 owned Extended-profile I/P/B streams for luma 4x4 residuals, chroma DC, and combined luma/chroma DC+AC. Every coded macroblock has the intended counts (16/0/0, 0/2/0 or 16/2/8), alternating residual signs, explicit motion and exact physical pair modes. Frame/field/mixed layouts, both slice wire orders and filter idc 0/1/2 are covered. Every saved JM display-order sample matches native decode over two resets, including B prediction from the retained residual-bearing P frame. Eighteen filter pairs require equal IDR anchors and distinct B/P luma planes.

No new production fixes were needed. This qualifies single nonzero first coefficients in each selected block and opposite signs across top/bottom blocks; arbitrary levels/runs, additional partitions, residual-bearing direct/skip/intra combinations, remaining maps and separate field-picture admission remain open. Generation invokes JM explicitly outside ordinary offline tests, which require no FFmpeg/JM/network.


Separate-field PCM reconstruction foundation: four owned 32x32 streams contain two complementary field pictures, 8/10-bit and top-/bottom-first order. The first field is IDR with POC zero; the second is a non-IDR I field of the same frame_num with POC one, preserving the pair rather than flushing the DPB with a second IDR. Native stateful playback specifically refuses them at the progressive-only intra reader.

The new owned avc_field_picture module reconstructs single-group CAVLC PCM into compact field planes and weaves complementary fields into a full frame with parameter/frame/parity/dimension validation and explicit output/coverage memory accounting. Every independent JM pixel matches the reconstructed/weaved fields in either argument order. Tests require actual field/parity/IDR syntax, exact memory boundaries, rejection of equal-parity pairs and explicit non-PCM refusal. This is reconstruction acceptance only: the test separately retains the specific passing AvcDecoder refusal. Field reference-list construction, DPB marking, pairing/display scheduling, non-PCM intra/inter reconstruction and full playback acceptance remain unfinished. No FFmpeg/private media are used; explicit JM generation is separate from offline tests.


Field reference-list foundation: the owned avc_field_references module builds P/B lists from complete or unpaired frame stores, sorting stores first and alternating current/opposite parity with independent cursors. P uses FrameNumWrap; B uses the store's minimum field POC, including the equal-current-POC partition, and swaps the first two L1 fields when both initial lists are equal. Field PicNum/LongTermPicNum use same-parity +1, doubled wrap/modification ranges and prefix-preserving list changes. Configuration is bounded to 16 stores/32 fields, validates IDs, available fields, frame numbers and long-term indices, and refuses missing selections.

Four tests cover asymmetric unpaired stores, frame-number wrap, frame POC versus individual-field order, equal B lists, same/opposite parity, long-term changes, intentional modified-prefix duplicates, modulo cycles and invalid/missing references. The four existing complementary PCM field fixtures also exercise parity list selection from their actual parameter/frame metadata. This is a list algorithm foundation, not field playback acceptance: DPB field marking and list integration remain disconnected. The API currently represents one shared short/long-term status per frame store; mixed marking within a pair must be added before complete conformance. Separate-field non-PCM reconstruction/display integration and broad codec gaps remain open.


Independent field marking is now represented in the field-list foundation: each parity has its own optional long-term index. Short-list expansion selects only short fields while preserving frame-store POC sorting; long lists independently sort each parity by its long-term index and alternate them. Duplicate long-term indices are rejected within the same parity, while opposite-parity reuse is retained. Modification lookup uses the selected field's own status/index. Two new tests cover mixed pairs, distinct long indices in one pair, asymmetric parity order, shared store POC, short/long changes, opposite-parity index reuse and absent marked fields.

Four additional owned complementary-field streams (8/10-bit, both first-field orders) place actual MMCO 3 in the second field header, converting the first opposite-parity field to long-term index 2 while retaining the current field as short-term. Syntax gates require that exact operation; reconstruction/weaving matches every JM sample and the list tests select the intended parity for short and long references. Eight saved field streams now qualify reconstruction. This does not implement DPB MMCO execution or full field playback: AvcDecoder's progressive-only reconstruction refusal remains separately asserted. The prior shared-marking-only restriction of the list module is superseded, but DPB integration, field prediction, non-PCM reconstruction and display pairing remain unfinished.


Field DPB foundation: avc_field_dpb stores reference fields independently, resolves field selections and executes MMCO 1–6 transactionally before committing references/limits. Complementary reference fields share a stable store ID; only the immediately pending store can receive its other parity, avoiding accidental pairing with an older wrapped frame_num. Short/long marking is per field. Sliding-window qualification covers complete short-term pairs and unpaired fields; full field/frame mixed storage and all mixed-store eviction cases still need conformance acceptance.

AvcDecoder now accepts the eight owned complementary PCM field streams, returning no picture for the first field and one woven picture for the second. Every output sample matches JM after reset. This is decoder acceptance only: MP4 player/export timing still needs to associate the pair with the first field timestamp and combined duration, and incomplete pairs at EOF need explicit handling. Non-PCM/inter fields, field frame-number gaps and mixed frame/field storage remain unsupported.

The eight PCM field fixtures now drive actual field DPB marking and list lookup. Their mixed-long headers were corrected: SPS max_num_ref_frames is three and MMCO 4 sets MaxLongTermFrameIdx before MMCO 3 converts the opposite field to index two. The previous JM-decodable sequence lacked that limit establishment and was not sufficient conformance evidence. Exact syntax gates now require MMCO 4+3. Tests exercise conversion, current-long replacement, parity-specific long/short forgetting, reset, pair completion without sliding eviction, eviction of an older complete short pair, refusal when only long references remain and preservation of all references after invalid operations. PCM reconstruction/weaving still matches JM.

This is field DPB/list execution acceptance, not AvcDecoder playback acceptance. Native field reconstruction admission, sample/motion reference integration, non-PCM prediction, mixed frame/field DPB streams, POC/display pairing and memory integration remain unfinished. Ordinary tests are offline without FFmpeg/JM/network; fixture generation invokes JM separately.

PCM field presentation admission: MP4 software playback and owned media decode now retain the first field sample/PTS and combine both nominal durations. The eight owned field streams match JM through playback and rewind; media visitor checks include exact pixels and a 40 ms interval for both 8-bit and High10 pairs. A separately generated single-field MP4 refuses at EOF with the specific unpaired-field error, including after rewind. This closes the previously documented pair-timing/EOF gap; non-PCM/inter fields, gaps and mixed frame/field DPB remain open.

Separate-field P skip admission: native AvcDecoder resolves L0 through the field DPB and reconstructs a complete same-parity CAVLC P-skip field. Four owned I/I/P/P streams cover 8/10-bit and both first-field parities, with exact syntax gates and every sample matched against separately generated JM output after reset. Software playback/rewind and owned media visitor timing/pixels also pass. Output allocation accepts exactly 1536 bytes and refuses 1535 for 32x16 compact fields; native fixture replay passes at the conservative decoder budget of 15362 bytes. Coded P macroblocks, opposite-parity interpolation, weighted fields, CABAC/FMO fields and B fields still require general field reconstruction. This is a specific inter-field acceptance milestone, not full AVC field conformance. Core: 503 passed, one ignored; AVC regression suites: 64 passed; owned media field test passed, without FFmpeg/JM/network during tests.

Separate-field explicit motion admission: CAVLC P fields now reconstruct explicit 16x16 L0 macroblocks with no residual, same-parity reference zero and deblocking disabled. Motion prediction shares MotionField; interpolation runs in compact field coordinates. Sixteen owned streams cover both parity orders, 8/10-bit and signed fractional vectors [1,1], [-1,-1], [5,-3], [-7,6]. Exact syntax gates require two coded P16x16 blocks with the intended first MVD and propagated second predictor; every sample differs from the stationary first frame and matches separately generated JM output after reset and playback rewind. Media visitor pixel/timing checks include fractional 8/10-bit examples. Conservative native storage succeeds at 17408 bytes; 17407 refuses the P field and requires reset. Core: 503 passed, one ignored; AVC regressions: 65 passed; owned media field acceptance passed offline. Residual, partitioned/weighted/multiple-reference/opposite-parity P fields, filtering of coded fields, CABAC/FMO and B fields remain open.

Separate-field partition admission: native CAVLC P fields now share the existing partition motion predictor and reconstruct rectangular luma/chroma blocks for P16x8, P8x16 and P8x8/P8x8ref0 with 8x8, 8x4, 4x8 or 4x4 subdivisions. Forty owned I/I/P/P streams cover both field orders and 8/10-bit precision, distinct signed MVDs per partition and no residual/deblocking. Exact syntax gates check partition dimensions/counts and each MVD; changed output and every JM sample match after reset and software playback rewind. Owned media visitor checks include 4x4 and 4x8 examples. Residual, weighted/multiple-reference/opposite-parity fields, coded-field filtering, CABAC/FMO and B fields remain open.

Separate-field residual admission: CAVLC P field reconstruction now reads field-scanned inter coefficients with raster nC neighbours, stores zero counts for skips, propagates macroblock QP and applies existing component-QP/scaling-list reconstruction (including the existing 4x4/8x8/bypass dispatch). Qualification covers signed 4x4 luma AC, chroma DC and chroma AC in twelve owned 8/10-bit streams with both field orders. Syntax gates require nonzero coefficients at field-scan positions (raster index four rather than frame index one), exact signs/counts and the following skipped macroblock. Four independently generated zero-residual controls preserve identical motion: every residual stream must change the moving output against its control. Native reset, playback rewind and media visitor pixel/timing acceptance match separately generated JM output. Transform8, bypass, varying QP/scaling matrices and more neighbour layouts still need field-specific qualification; filtering, multiple-reference/weighted/opposite-parity prediction, CABAC/FMO and B fields remain open.

Separate-field transform/QP/scaling qualification: forty-eight owned streams now qualify the existing P-field dispatch for 4x4/8x8 transforms, slice QPs 18/26/40, parity-dependent mb_qp_delta -3/+3, 8/10-bit and both field orders. High/High10 PPS explicitly enables transform8; scaled variants carry all eight explicit scaling lists (intra eight, inter twenty-four), while controls retain default flat sixteen. Syntax gates check transform flag, QP, coefficient counts and resolved inter matrices. Every sample matches separately generated JM after native reset and playback rewind; QP40 controls prove that transform size and scaling separately change reconstructed residual pixels, without changing PCM reference output. Media visitor includes scaled 4x4/8x8 examples. This adds qualification to previously connected native reconstruction, not a new external backend. Transform bypass, longer QP/context transitions and broader scaling-list layouts remain to qualify; field filtering, multiple-reference/weighted/opposite-parity prediction, CABAC/FMO and B fields remain open.

Separate-field transform bypass qualification: thirty-two owned High 4:4:4 Predictive-profile (4:2:0 chroma) streams qualify native P-field QP-prime-zero dispatch for 8/10-bit, 4x4/8x8 and both field orders. SPS explicitly toggles qpprime_y_zero_transform_bypass_flag, slice/MB syntax gates prove QP-prime zero (QP zero at depth eight, minus twelve at depth ten), signed luma/chroma coefficients and transform selection. Both flat-sixteen and explicit inter-twenty-four PPS matrices are used. Enabled-bypass oracles must equal each other across scaling matrices and differ from quantized controls; native reset, playback rewind and every JM pixel match. Media visitor includes scaled 4x4/8x8 bypass examples. This qualifies the previously connected inter bypass dispatch; separate-field intra bypass/RDPCM, broader profiles/chroma and field filtering remain unqualified, and multiple-reference/weighted/opposite-parity prediction, CABAC/FMO and B-field paths remain open.

Separate-field filtering admission: CAVLC P field reconstruction now retains per-4x4 motion/nonzero metadata and derives field row edges through row_edges_field. Field vertical motion uses threshold two; frame row_edges retains its threshold four. Compact luma/chroma planes use the existing inter filter, per-component QPs, slice offsets and transform8 edge suppression. Thirty-six owned streams cover filter0/1/2, both field orders, 8/10-bit and motion-only/luma-AC/all-AC cases at QP50. Every enabled oracle differs from its filter1 control; same-slice filter2 equals filter0. The motion-only stream uses a vertical MVD difference two, specifically distinguishing field from frame strengths. All samples match JM after reset/rewind, with media visitor checks. Fields with multiple slices, mixed intra/inter blocks and wider row/edge layouts still need qualification.

P-field storage accounting was tightened: coded prediction reserves an additional 16384 bytes for bounded partition/prediction scratch, and enabled filtering reserves 2048 rather than 1024 bytes per macroblock for retained edge grids/count/motion state. This supersedes the previous 17408-byte coded-motion fixture boundary: the unfiltered fixtures now accept 33792/refuse 33791; filtered fixtures accept 35840/refuse 35839. Complete skip-only fields retain their earlier compact-copy admission. Multiple-reference/weighted/opposite-parity P prediction, CABAC/FMO and B fields remain open, alongside other codec gaps.

Separate-field multislice admission: native CAVLC P fields now sort contiguous slices by first_mb, validate picture/reference context identity, enforce complete coverage and maintain independent motion/count/QP contexts per slice. Field-edge metadata carries slice IDs, making filter2 suppress cross-slice boundaries. Forty-eight owned two-slice streams cover both parity orders, 8/10-bit, motion/all-AC, filters0/1/2 and forward/reverse NAL order. Different nonzero nal_ref_idc values two/three are admitted while reference/non-reference identity remains consistent. Exact syntax gates require two one-MB slices in the intended wire order; controls prove cross-slice suppression and ASO invariance. Every JM sample matches after reset and playback rewind; media visitor covers ASO cases. Different per-slice reference-list contexts remain explicitly unsupported, as do mixed intra/P field slices, CABAC/FMO and B fields; multi-row field filtering still needs qualification.

Multi-row separate-field qualification: one hundred twenty owned 32x64-frame streams (32x32 compact fields, four macroblocks) now cover one slice, two row slices and four macroblock slices, motion/all-AC, filters0/1/2, 8/10-bit and both field orders. Multi-slice variants include reverse NAL order. Syntax gates check the actual dimensions, slice address ranges, every MVD and slice-local coefficient counts, including left/top propagation across two rows. Every saved JM sample matches after reset and playback rewind; seek restarts from the IDR pair and reproduces the second frame exactly. Filter controls prove actual enabled changes and row/slice suppression; ASO preserves each layout output. Media visitor includes one-slice residual and reversed row-slice motion examples. This qualifies previously connected compact row handling; it does not establish universal field filtering. Per-slice different reference lists, multiple-reference/weighted/opposite-parity prediction, mixed intra/P fields, CABAC/FMO and B-field paths remain open, as do other codec gaps.

Per-slice P-field reference admission: AvcDecoder now resolves each slice L0 through the field DPB, preserving the selected field and stable frame-store identity into decode_p_field_resolved. Reconstruction selects the corresponding compact reference planes after address sorting; deblocking uses each actual reference identity rather than a shared zero. Different per-slice list modifications are now admitted for one active same-parity reference. Ninety-six owned six-field streams retain distinct frame-zero/frame-one PCM pairs, then choose old/new references independently in two P slices. Normal/swapped lists, skip/coded, filters0/1/2, both field orders, 8/10-bit and reversed NAL order are qualified. Syntax gates prove Subtract(3)/Subtract(1) selections; oracle controls require visible swapped-reference changes and filtering from unequal picture identities despite zero MVD. Every JM pixel matches after reset/rewind/seek; media visitor verifies three-frame pixel/timing delivery. Multiple active references, weighted/opposite-parity prediction, mixed intra/P, CABAC/FMO and B fields remain open, alongside broader codec gaps.

Opposite-parity P-field admission: compact prediction now permits the selected reference field to have opposite parity. Luma remains in field coordinates; chroma applies the established +/-2 eighth-sample vector adjustment (bottom from top plus two, top from bottom minus two), consistent with the existing MBAFF primitive and JM field prediction. Stable deblocking identity now encodes frame-store ID and parity, distinguishing two fields of one store. Complete opposite-parity skip fields use the general prediction path rather than the same-parity copy fast path. Ninety-six owned streams qualify old/new opposite-field selections using Subtract(4)/Subtract(2), skip/coded, normal/swapped choices, filters0/1/2, ASO, 8/10-bit and both field orders. Every JM pixel matches after reset/rewind/seek, with media visitor pixel/timing checks. Multiple active references, weighted prediction, mixed intra/P, CABAC/FMO and B fields remain open; this does not prove universal AVC field conformance.

Multiple active P-field references: native decode_p_field_lists now receives the complete resolved L0 for each slice (up to the parsed 32-entry limit), validates active-list length and selects each partition reference index. Skip retains index zero; coded partitions carry their actual index through motion prediction, parity-adjusted chroma sampling and stable deblocking identity. Ninety-six earlier opposite-field streams remain accepted; two hundred eighty-eight new streams specifically qualify four simultaneously active entries for both retained frame pairs and both parities. Explicit field list modifications produce old-same/old-opposite/new-same/new-opposite entries; rotated ref_idx choices, 16x8/8x16/subdivisions down to 4x4, filters0/1/2, ASO and 8/10-bit compare every JM sample after reset/rewind/seek. Media visitor pixel/timing checks include mixed parity/reference partitions. Additional list storage is included conservatively (32 bytes per entry beyond the first per slice); fixture decoder budgets accept 33984 unfiltered/36032 filtered and refuse one byte less at the P field. Four-active-entry qualification does not prove every count/list layout. Weighted fields, mixed intra/P reconstruction, CABAC/FMO and B fields remain open, with broader codec gaps unchanged.

Separate-field weighted P prediction: CAVLC P fields apply the selected L0 luma/chroma weights after interpolation and before residual reconstruction. Weighted skip bypasses the unweighted copy shortcut. One hundred sixty-eight owned streams qualify four-entry weights, implicit defaults, signed weights, clipping, High10 offset scaling, whole-field skip, 16x8/8x8 partitions, signed residual, filters0/1/2, ASO and both field orders. Every saved JM pixel matches after native reset, playback rewind and seek; media visitor checks preserve pair timing. No-filter controls isolate residual effects. Core: 503 passed, one ignored; AVC and owned-media regression checks: 77 passed offline without FFmpeg/JM/network. Mixed intra/P reconstruction, separate-field CABAC/FMO/B and broader codec gaps remain open.

Mixed PCM/inter P-field admission: native CAVLC P fields dispatch mb_type30 to owned I_PCM reconstruction, publish intra motion neighbours and sixteen-coefficient contexts, and use intra deblocking metadata with PCM QP zero while retaining prior syntax QP for subsequent inter blocks. Forty-eight owned streams qualify PCM before/after coded inter or skip, 8/10-bit, both field orders and filter0/1/2. Every JM sample matches after reset, playback rewind and seek; media visitor checks include PCM/inter pairs. Replaying the fixture with the previous implementation fails specifically at the P-field PCM macroblock with “expected AVC inter macroblock”, rather than container/header parsing. Non-PCM intra prediction inside P fields remains open, as do separate-field CABAC/FMO/B and broader codec gaps.

Mixed non-PCM intra P-field admission: the owned IntraCavlcReader now accepts compact separate fields and selects field coefficient scan independently of MBAFF. P-field dispatch maps intra mb_type to the shared reader and reconstruction, publishes counts/modes across inter neighbours, applies constrained-intra availability, updates syntax QP only for non-PCM blocks and emits intra filter metadata. I_PCM is retained through this shared path. Forty-eight new Intra4x4/DC zero-residual streams match every saved JM sample after reset/rewind/seek, covering placement before/after coded inter or skip, filters0/1/2, both field orders and 8/10-bit. Media visitor checks include both placements. Other intra modes, nonzero intra residual, transform8 and constrained-intra variants still require field-specific qualification; I-field non-PCM admission, CABAC/FMO/B remain open. This is not full field conformance.

Field intra residual qualification: 240 owned P-field streams now qualify Intra4x4 luma AC at field-scan index one (raster four) with alternating signs, and Intra16x16 signed DC with zero AC. Matched zero-coefficient controls retain each mode and motion; deblocking-disabled comparisons require visible residual effects. Syntax-only shared-reader gates verify field scan, every AC sign/count placement and DC sign. PCM references, intra before/after coded inter or skip, filters0/1/2, both field orders and 8/10-bit match every saved JM sample after reset/rewind/seek. Media visitor pixel/timing checks include AC and negative High10 DC. This qualifies existing native field reconstruction; chroma intra residual, other prediction modes, transform8 and broader coefficient contexts remain to qualify.

Non-PCM I-field admission: native AvcDecoder dispatches single-group CAVLC I fields to the shared scaling-aware intra reconstruction in compact half-height coordinates, with halved vertical crop, field coefficient scans and sorted slice coverage. The field wrapper validates picture identity/MMCO/parity before reconstruction; completed pictures retain existing field DPB and presentation pairing. One hundred twenty owned Intra4x4 AC and Intra16x16 positive/negative DC streams include zero controls, two slices, ASO, filters0/1/2, 8/10-bit and both field orders. Every JM pixel matches after reset/rewind/seek; media visitor preserves first-sample timing and combined duration. The old PCM-only dispatch refuses the fixture specifically at the non-PCM macroblock. CABAC/FMO I fields remain explicitly unsupported; other intra modes/chroma residual/transform8 and broader layouts still require field qualification.

PCM-only I fields with deblocking disabled retain the compact reconstruction path and its earlier memory admission boundary; non-PCM syntax falls through to the general intra path. The existing skip-field fixture caught the increased temporary-storage requirement before this compatibility dispatch was restored.

I-field chroma residual qualification: 120 additional owned streams activate both chroma DC and AC in Intra4x4/Intra16x16 fields. Cb/Cr DC signs differ, chroma AC signs alternate and field scan places AC at raster index four. Identical luma-only streams prove chroma isolation without filtering: luma bytes must match, nonzero chroma bytes must differ, and zero controls match entirely. Both depths/parity orders, two slices, ASO and filters0/1/2 match every saved JM pixel after reset/rewind/seek. Media visitor examples qualify pixels and pair timing. This qualifies the existing owned chroma reconstruction and does not claim other prediction modes, transform8, broader nC or CABAC/FMO field coverage.

Intra8x8 I-field qualification: 96 owned High/High10 streams explicitly enable PPS transform8, select Intra8x8 DC modes and interleave sixteen CAVLC coefficient streams into four 8x8 transforms. Syntax gates require exact signed coefficients at field-scan raster positions 9/24/32/17, default intra matrix sixteen or explicit matrix eight, and zero controls. Both field orders, two slices, ASO, filters0/1/2 and depths8/10 match every JM pixel after reset/rewind/seek; unfiltered controls separately require a matrix-dependent residual effect and scaling independence for zero residual. Media visitor preserves exact pixels/pair timing. This qualifies connected native transform8 reconstruction, not every prediction mode, coefficient layout or bypass/profile.

I-field transform bypass qualification: 224 owned profile244/4:2:0 streams qualify QP-prime zero at depths8/10 (syntax QP0/-12), Intra4x4/Intra8x8/Intra16x16 DC, signed luma/chroma residual or matched zero coefficients, both field orders, two slices and ASO. SPS bypass enable/disable controls preserve syntax and matrices8/16; enabled output must ignore matrix changes and differ from ordinary reconstruction for nonzero coefficients. Every saved JM pixel matches after reset/rewind/seek; media visitor pixel/pair timing examples cover enabled4/8 and disabled16. Deblocking is disabled at this qualification boundary. Directional intra residual accumulation, other modes, broader nC/profile/chroma layouts and field CABAC/FMO/B remain open.

FMO I-field admission: the shared intra CAVLC reader accepts field geometry for FMO and keeps map-unit addressing compact, independently of MBAFF pair mode. Native I-field reconstruction accepts all seven map types, with both change directions for dynamic maps. 480 owned PCM/I16 DC streams cover depths8/10, both field orders, signed/nonzero and zero DC, all filter modes and ASO. Syntax gates require exact group address sequences; every JM sample matches after reset/rewind/seek and media visitor timing checks. This exposed a horizontal field-boundary filtering defect: intra_plane_owned used frame-only intra strength4 rather than field strength3. Selecting field parity in the existing strength helper fixes the specific pixel mismatch (first difference at byte835 in type0/dc0/filter0); filtered and suppression controls require visible effects. FMO P/B field reconstruction and CABAC fields remain open.

CABAC I-field admission: the owned IntraCabacReader now separates compact field-picture mode from MBAFF pair flags. Raster field neighbours, geometry, significance/last contexts and inverse scans use the actual field flag; frame/MBAFF logic remains separate. Native I-field reconstruction no longer refuses CABAC geometry. Seventy-two owned High/High10 Intra16x16/DC streams encode positive/negative DC at scan index one (raster four) or zero coefficients, two slices, ASO, filters0/1/2 and both field orders. Every saved JM sample matches after reset/rewind/seek; syntax gates require the field scan position and sign, while unfiltered zero controls require a pixel difference. The old context admission refuses specifically at field CABAC geometry. Media visitor checks preserve pair timing/pixels. Other CABAC intra modes/chroma/coefficient layouts and native CABAC P/B fields remain to qualify/connect.

CABAC P-field admission: native field reconstruction now consumes the owned InterCabacSlice macroblock iterator and shares the existing intra/inter prediction, weighted interpolation, residual reconstruction and filtering paths with CAVLC. Compact CABAC motion contexts use field units without MBAFF pair addressing; active reference counts are already expanded by the parsed field header and are not doubled again. CABAC temporary contexts reserve a conservative 8192 bytes per macroblock in the field admission calculation; CAVLC fixture memory boundaries remain unchanged. 144 owned I/I/P/P streams specifically qualify skip or fractional P16x16 with MVD[1,-1], no P residual, all three CABAC init banks, ASO, filters0/1/2, both orders and8/10-bit. Nonuniform signed-DC CABAC reference fields make interpolation observable; coded/skip controls require different pixels without filtering. Every JM sample matches after reset/rewind/seek, with media visitor pair timing checks. The old CABAC refusal reproduces specifically at the first P sample. Partitioned/weighted/multiple-reference/mixed-intra/residual CABAC P fields still require dedicated field qualification; B fields remain unconnected.

CABAC P-field signed luma residual qualification: 144 owned streams retain fractional P16x16 motion and CBP15 while toggling zero versus alternating-sign luma AC at scan index one. The owned arithmetic writer explicitly encodes field significance/last contexts and coded-block neighbour flags; syntax gates require CBP15, counts1/0, raster position four rather than one, exact signs and MVD[1,-1]. Independent zero controls isolate residual effects without filtering. All CABAC init banks, ASO, filters0/1/2, both orders and8/10-bit match every saved JM sample after reset/rewind/seek; media visitor checks exact pixels and pair timing. Chroma/transform8/partition/mixed-intra/multiple-reference/weighted CABAC P field variants remain to qualify, with B fields unconnected.

CABAC P-field chroma residual qualification: 144 owned CBP47 streams now qualify signed Cb/Cr DC and alternating chroma AC alongside the already qualified luma residual. Field significance/last contexts and coded-block flags use actual colour components; syntax gates require counts1/0, opposing DC signs and field AC at raster index four. Identical luma-only controls prove unchanged reference frame/final Y and visible Cb/Cr effects without filtering; all-zero controls match fully. Init0/1/2, filters0/1/2, ASO, both orders and8/10-bit match every saved JM sample after reset/rewind/seek, including owned media visitor pair timing. Transform8/partition/mixed-intra/reference/weight variants and B fields remain open.

CABAC P-field transform8 qualification and filtering fix: 288 owned streams qualify one signed coefficient per 8x8 block at field-scan raster position eight, CABAC field significance/last contexts, inter matrices16/24 and CBP0 no-residual controls. Intra matrices remain16 so reference pixels must match across matrix variants; unfiltered P pixels must change with inter scaling, while no-residual controls remain equal. All init banks, ASO, filters0/1/2, both orders and8/10-bit match JM after reset/rewind/seek and media visitor timing checks. The first filtered 8-bit/AC/init0/scale16 fixture exposed a field deblocking bug: CABAC8 coefficients are stored in luma8, but field boundary metadata examined CAVLC4x4 counts. The fix derives nonzero status from the actual 8x8 residual, as the frame path already does. This fixture now accepts exact pixels; CAVLC transform8 regressions remain enabled. Partitions, mixed intra, references/weights, bypass and B fields remain open.

CABAC P-field partition qualification: 432 owned streams cover 16x8, 8x16, 8x8, 8x4, 4x8 and 4x4 partitions with alternating signed MVD. Acceptance gates exact partition geometry and motion, observable differences from single-partition controls, every saved JM pixel, reset, rewind and seek across all init banks, filters, ASO, depths and field orders. Mixed intra, references/weights, bypass and B fields remain open.

CABAC P-field mixed intra qualification: 432 owned single-slice streams place I4 DC or signed I16 DC before/after a skipped or fractional-motion inter macroblock. Syntax gates require both addresses in one P slice, correct intra/inter dispatch and field-scanned signed DC at raster position four. Unfiltered matched controls require observable changes from both DC sign and inter motion, with invariant reference frames. Init0/1/2, filters0/1/2, both field orders and8/10-bit match every saved JM sample after reset, rewind and seek; representative cases also pass owned media visitor pair timing. This qualifies these already connected mixed branches, not PCM, directional intra, constrained prediction, references/weights, bypass or separate B fields, which remain open.

CABAC P-field constrained-intra qualification: 432 constrained streams and432 matched unconstrained controls share biased signed-DC reference fields with row variation. Initial reference frames match exactly; intra-first pairs match fully, while every unfiltered intra-last pair must exhibit the effect of refusing the preceding inter neighbour. Both controls and constrained native decode match saved JM samples; constrained streams also repeat after reset/rewind/seek and pass representative owned visitor pair timing. Syntax gates retain I4/I16 dispatch, field-scan signs and actual skip/coded neighbours. The original unbiased reference had zero mean and failed to expose I16 DC neighbour use; adding mean and row variation makes the test causal. Existing DC mixed and partition coverage remains enabled. PCM, directional intra, multiple references/weights, bypass and separate B fields remain unqualified.

CABAC P-field multireference qualification: 1728 owned six-sample streams cover four active field references in 16x8,8x16,8x8,8x4,4x8 and4x4 partitions, default and explicitly permuted L0, two index rotations, all init banks, ASO, filters0/1/2, both field orders and8/10-bit. Reference pictures distinguish frame age and parity through biased/nonbiased signed DC and row variation. Syntax gates exact four-entry counts, modification sequence, partition reference indices and MVD. Matched unfiltered controls preserve the first two frames and require visible final-frame effects from reference rotation and list permutation. Every saved JM sample matches native reset, player rewind/seek and representative media visitor timing. This qualifies reference-index CABAC context propagation within these partitions and opposite-parity field interpolation; mixed cross-macroblock context variants, weighting, bypass and separate B fields remain open.

CABAC P-field weighted prediction qualification: 576 owned six-sample streams combine four explicitly permuted references with skip,16x8,8x8 or signed Y/Cb/Cr residual, identity/explicit weight tables, all init banks, ASO, filters0/1/2, field orders and8/10-bit. Tables contain separate luma/chroma denominators, negative/zero weights, signed offsets and a default entry. Syntax acceptance gates the exact table, selected partition indices, MVD and field-scanned signed residual. Matched identity controls require visible weight effects without changing either reference frame; residual/no-residual controls require visible effects in every Y/Cb/Cr plane on the same prediction. All saved JM pixels match native reset, player rewind/seek and representative owned visitor timing. This qualifies the existing weight-before-residual field path for these streams; additional denominator boundaries, cross-macroblock contexts, bypass and separate B fields remain open.

Separate explicit B-field reconstruction is connected: the compact inter-field path now resolves both L0/L1 per slice, derives independent list vectors, interpolates each field with parity-correct chroma offsets, averages bidirectional samples, and retains both reference identities for deblocking before shared residual reconstruction. The previous L0-only P API delegates to the same path with an empty L1 and unchanged memory accounting. 288 owned six-sample streams qualify L0,L1 and Bi16x16, CAVLC and CABAC init0/1/2, ASO, filters0/1/2, both field orders and8/10-bit. The saved filtered Bi CABAC fixture first reproduced the exact sample4 `AVC inter field reconstruction is not connected` refusal; its refusal test is replaced by pixel acceptance with the fix. All JM samples, reset, rewind, seek and representative media visitor pair timing are gated. These B pairs use already displayed reference fields; future-reference display reordering is not qualified here. Direct/skip B prediction, implicit B weighting, retained field reference motion and broader partition/residual/weight combinations remain incomplete.

Future-reference B-field presentation qualification: explicit `--future` generation supplies288 streams with decode POC/PTS0,1,4,5,2,3. Native decode-order output must match JM display frames permuted0,2,1; software MP4 and owned media visitor output must match JM display pixels with first-field sample identities0,4,2, PTS0,2,4 and duration2. Syntax gates every field POC and stored signed composition timestamps, plus actual L0/L1/Bi indices/MVD. All init banks, CAVLC, ASO, filtering, field orders and8/10-bit repeat after reset/rewind and seek to the reordered B pair. Existing reorder/pair-timing paths already satisfy these fixtures; no production fallback or queue change was needed. A separate contract test confirms ordered `decode` refuses decreasing pair POC, stays poisoned until reset and recovers through `decode_order`; that refusal is not the presentation acceptance test. Direct/skip, implicit weighting, retained reference motion and broader B tools remain open.

Implicit B-field weighting is connected: field DPB now exposes selected-parity POC and long-term status; the decoder passes validated per-slice L0/L1 order tables and the current field POC to compact reconstruction. Bidirectional predictions derive normative distance-scale weights, zero offsets and denominator5; unilateral predictions stay unweighted. Existing public explicit/P entrypoints retain their contract, and the context-aware entrypoint rejects missing/misaligned order tables. 288 owned implicit future-reference streams cover L0/L1/Bi, CAVLC/all CABAC init banks, ASO, filters, both orders and8/10-bit. The initial native sample4 missing-POC-context refusal was reproduced and replaced by acceptance. Matched unweighted controls require identical reference frames, fully identical unilateral output and observable unfiltered Bi effects. Every saved JM sample, reset/rewind/seek and owned visitor pair timing is gated; existing DPB PCM/MMCO fixtures also gate parity POC and long-term status changes. Direct/skip, retained reference motion, weighted long-term end-to-end cases and broader B tools remain open.

Separate-field motion retention foundation: the reconstruction API can return its completed motion grid, and field snapshots resolve each slice reference into stable picture identity, selected parity and both motion lists. A regression uses six owned mixed-intra, multireference and explicit B streams, checks snapshot budget refusals and metadata, and repeats exact saved JM pixels. Existing picture-only entrypoints preserve their fast skip path. This does not connect retained snapshots to the decoder DPB or enable direct/skip B fields yet.

Separate-field DPB motion retention is now connected: reference inter fields store their compact motion snapshot alongside the owned pixel Arc; per-slice ASO mappings resolve stable store IDs and selected parity before reference marking. Intra references carry no motion. The decoder reserves snapshots for both parities of each reference store plus the current snapshot before eviction; tight field budget tests add that explicit storage. Whole-slice CAVLC P skip preserves its copy path while publishing zero-vector L0[0] metadata and accounting for the working grid. An owned multireference/skip DPB regression checks stored identities/parities, exact JM output, repeated decoding and release on reset. Direct/skip B derivation is still not connected, and mixed frame/field co-located conversion remains unsupported.

Separate-field B direct/skip is connected: a field-specific direct context carries selected-parity POC, long-term status, stable identity/parity, and the motion snapshot of L1[0]. Temporal mapping matches both identity and parity; spatial derivation uses original field-unit neighbours and co-located zero flags. The shared transactional macroblock dispatcher publishes inferred 4x4 partitions and rolls back failures; progressive direct contexts refuse separate-field geometry. Native field reconstruction dispatches both skipped B macroblocks and coded direct partitions through this context. 768 owned eight-sample streams use two I pairs, an inter-coded future P pair and a reordered B pair, covering CAVLC/CABAC init0/1/2, 8/10-bit, both field orders, ASO, filters0/1/2 and direct8 inference on/off. P references are explicitly permuted to older I fields so temporal direct has valid references in B L0; the first unpermuted draft was rejected by JM for an unavailable co-located reference and was corrected before acceptance. Exact JM decode/display pixels, reset, rewind, seek, and first-field sample/PTS/duration are gated. Skip/coded controls match; unfiltered spatial/temporal controls and CABAC inference controls must differ only in B output. A legacy-context refusal test reproduces the specific missing-motion-context error, separately from native acceptance. Selected owned media visitor cases gate reordered timing and JM pixels. Long-term direct end-to-end cases, mixed direct/explicit/intra partitions, direct residual/transform/bypass, mixed frame/field co-located conversion and other codec gaps remain open.

Separate-field CAVLC mixed B qualification: 2976 owned streams place L0/L1/Bi16 adjacent to direct in either macroblock order, embed I_PCM before/after direct, and place direct in each of four B8x8 groups with every explicit B subtype1..12 in the remaining groups. Both B macroblocks share one slice; syntax gates exact references, MVD order, direct groups and PCM dispatch. All depths8/10, field orders, spatial/temporal modes, inference flags and filters match saved JM pixels after reset, rewind, presentation and seek. Matched unfiltered vector controls prove that the direct-right macroblock uses the explicit-left vector only for spatial direct; temporal direct pixels remain independent of that neighbour. Existing reconstruction satisfies these streams without a new production change. CABAC mixed branches, directional intra and richer residual/transform combinations remain unqualified.

Separate-field CAVLC B-direct residual qualification: 624 owned streams cover zero controls, signed luma AC, chroma DC and combined luma/chroma AC in either macroblock. Tests gate CBP0/15/16/47, 16 direct4 partitions, field-scan raster position4, signed chroma DC and actual propagated neighbour counts before checking every JM pixel. Unfiltered controls require changes only on the signalled planes and require both signs to change each affected plane; reference frames stay identical. Reset, rewind, reordered sample/PTS/duration and seek pass across both depths, field orders, direct modes, inference flags and filters. Representative mixed/residual media visitors gate exact display pixels and timings. This qualifies existing CAVLC4 reconstruction, not CABAC residuals, transform8/bypass, long-term direct or mixed frame/field conversion.

Separate-field CABAC B-direct residual qualification: 2304 owned streams cover CBP0, CBP47 with empty coded-block flags, signed luma4 AC, chroma DC and combined chroma AC. Native syntax gates all16 inferred direct partitions, field raster position4, signs, CBP0/15/16/47, init0/1/2 and completed entropy termination. Every JM pixel, reset, rewind, reordered sample/PTS/duration and seek are accepted across both depths8/10, field orders, direct modes, inference flags, filters and ASO. Empty-CBP controls must equal CBP0 including filtered output. Unfiltered residual/sign controls require visible effects only on signalled planes, with invariant I/P references. This qualifies already connected residual4 and filtering paths; no production decoder change was needed. CABAC mixed intra/explicit/direct and bypass remain unqualified.

Separate-field CABAC B-direct transform8 qualification: 864 owned streams combine enabled direct8 inference and PPS transform8 with one signed AC coefficient per8x8 block at field-scan raster position8. Intra scaling stays16; inter scaling varies16/24. Syntax gates actual transform8 dispatch, coefficients, signs and matrices, plus no-residual controls. Exact saved JM output repeats after reset/rewind/presentation/seek for all init banks, depths, field orders, direct modes, filters and ASO. Matched unfiltered controls require B-luma changes from residual sign and inter scaling, identical reference frames and chroma, and invariant no-residual output. Representative CABAC residual/transform8 media visitors gate reordered timing and exact display pixels. This does not qualify CAVLC transform8, mixed transforms, bypass, long-term direct or mixed frame/field conversion.

Separate-field single-slice CABAC mixed B qualification: 5184 owned eight-sample streams place L0/L1/Bi16 or I4 DC / signed I16 DC before/after direct-coded or skipped B macroblocks. Both B macroblocks share one slice, exercising non-skip/non-direct contexts, entropy termination between blocks, MVD/ref syntax, intra suffix32 and field-scanned DC. All init banks, depths8/10, field orders, direct modes, inference flags and filters match saved JM after reset, rewind, presentation and seek. Syntax gates exact explicit references/MVD, direct partitions, embedded intra types and signed DC at raster position4. Matched controls require coded/skip equality including filtering, spatial-only effects from explicit-left MVD, invariant temporal direct-right pixels, and intra-sign changes isolated to the intra macroblock when unfiltered. Reference frames are invariant across these controls. Representative owned media visitor cases gate reordered first-field sample/PTS/duration and exact JM pixels. Existing connected paths satisfy this matrix without a production decoder change. Directional/constrained intra B combinations, CABAC B8x8 mixed subtypes, mixed residual/transform/bypass, long-term direct and mixed frame/field conversion remain unqualified.

Separate-field long-term B direct qualification: 768 owned source/co-located long-term and matched short-term streams gate reference marking, parity-aware list selection, syntax, JM pixels, presentation, reset, rewind and seek. Temporal source-long-term controls and spatial co-located-long-term controls show the intended direct-region effects. A further 768 mixed-parity streams are accepted: only the first decoded parity is long-term, while its complementary parity remains short-term. Explicit list syntax and selected-parity DPB status are gated; causal controls require changes only in marked-parity direct rows, with unchanged complementary rows. Both matrices pass exact JM pixels, reset, rewind, seek, presentation and representative media visitors. Mixed streams declare four reference stores after JM rejected the initial three-store draft; this was invalid generated input, not a decoder regression.

Separate-field CABAC B8x8 qualification: 6912 owned CABAC separate B-field streams mix direct in each 8x8 group with every explicit B subtype1..12. Both fields contain two independent slices; syntax gates partition geometry, prediction modes, zero references, signed MVD and entropy termination. All init banks, depths8/10, field orders, spatial/temporal modes, inference flags and filters match saved JM pixels after reset, rewind, presentation and seek. Selected media visitors gate exact pixels and reordered first-field timing. Generation uses explicitly supplied JM only; ordinary tests consume saved synthetic fixtures offline, without JM/FFmpeg/network/private media. This qualifies existing reconstruction; no production decoder fix was needed. Cross-slice-independent B8x8 acceptance does not qualify cross-macroblock CABAC contexts, mixed residual/transform/bypass or directional intra.

Separate-field joined CABAC B8x8 qualification: 6912 additional owned CABAC B8x8 streams put both macroblocks in one slice. The writer retains per-list 4x4 MVD magnitudes across the boundary and emits neighbour-dependent skip, mb_type and CBP contexts. Tests decode both macroblocks, gate every partition/reference/MVD and entropy termination, then accept all saved JM decode/display pixels, reset, rewind and seek across subtype1..12, direct positions, depths, field orders, init banks, inference flags, direct modes and filters. Unfiltered independent-slice controls require invariant reference frames and left macroblocks, while at least one right macroblock must change through neighbour prediction. Selected media visitors accept pixels and timing; every fixture/oracle hash is verified. No production decoder change was needed. Positive reference-index context transitions, richer mixed residuals, transform bypass and directional intra remain unqualified.

Separate-field CABAC B8x8 multireference qualification: 6912 additional joined CABAC B8x8 streams cycle reference indices0..3 by group, list and macroblock. The owned writer maintains per-list reference grids and emits contexts54..57 with unary continuation58/59; direct groups remain unavailable for positive-index context derivation. Syntax gates exact selected indices, partition geometry, signed MVD and termination for both macroblocks. All saved JM pixels, reset, rewind, presentation and seek pass across all B subtype1..12, direct positions, init banks, depth/order, inference, spatial/temporal and filters. Zero-index controls require unchanged reference frames and observable B prediction changes. Selected media visitors and all video/oracle hashes pass. Generation alone uses explicit JM; ordinary tests need no JM/FFmpeg/network/private files. Existing production reconstruction satisfies the matrix. Transform bypass, richer mixed residuals, directional intra and mixed frame/field or frame-number-gap storage remain open.

Separate-field CABAC B-direct transform8 bypass qualification: 1728 owned CABAC B-direct 8x8 streams use High444Predictive with 4:2:0 geometry and minimum QP_Y (0 at8-bit, -12 at10-bit), paired transform_bypass enabled/disabled controls, inter matrices16/24, signed field-scanned AC and zero residuals. CABAC initialization clamps the negative QP to0 while reconstruction retains the actual QP. Syntax gates minimum slice QP, bypass flag, transform8, CBP and signed raster-position8 coefficients. Every saved JM decode/display pixel, reset, rewind and seek passes across field orders, spatial/temporal, init banks, ASO and filters. Enabled bypass must ignore scaling matrices exactly, preserve reference frames relative to controls and visibly change B reconstruction. Selected media visitors accept timing/pixels. All new video/oracle hashes were checked; regenerated existing864 transform8 streams remain unchanged. Generation alone requires explicit JM; ordinary acceptance is offline without JM/FFmpeg/network/private media. Existing production decoding satisfies these cases; B4x4/chroma bypass and mixed residual combinations remain unqualified.

Separate-field frame-number gap reconstruction is connected: The native decoder now inserts pixel-free non-existing field stores for allowed frame_num gaps, advances shared POC/frame history and applies field sliding-window eviction. Field storage separates pixel ownership from reference metadata; unknown type0 POC slots participate in P PicNum ordering but are excluded from B POC ordering. Non-existing fields cannot be promoted to long-term, and failed inferred insertion/MMCO preserves the buffer. 48 owned CAVLC I0–P2–P3 streams reorder prediction to real retained fields, with both depths8/10, field orders, skip/coded, filters and ASO. They pass exact saved JM pixels, reset, rewind, sample/PTS/duration and seek; a separate gaps-forbidden SPS fixture refuses sample2 with the exact SPS error and requires reset. Metadata regression gates pixel-free slots, missing POC exclusion, valid inferred POC and transactional MMCO rejection. Representative media visitors pass. Ordinary tests consume saved synthetic files offline without JM/FFmpeg/network/private media. CABAC, POC1/2 end-to-end gap qualification, wraparound and long-term gap eviction controls remain to be expanded; mixed frame/field storage remains unsupported.

Separate-field wrap gap and retained long-term identity fix: 48 owned wrap streams use I0–I14–P0 with inferred frames1..13 and15, exercising eviction across MaxFrameNum16 while P0 explicitly selects real frame14 fields. A further48 streams retain I0 as long-term index0 and declare four reference stores. The initial three-store long-term generator draft was rejected by JM for a missing selected reference and corrected before decoder reproduction. The valid long-term fixture reproduced native sample4 failure "AVC field DPB capacity or identity conflict": old validation incorrectly conflated reused frame_num across short/long-term marking. Field insertion and pixel-free gap insertion now reject duplicate short-term frame numbers while permitting coexistence with long-term stores; stable IDs and long-term-index uniqueness remain separately validated. Both matrices accept saved JM pixels, reset, rewind, presentation and seek, and matched short/long oracle pixels are identical. An automated metadata test cycles inferred frame numbers through0/1, retains both long-term parities and proves coexistence with pixel-free frame0. Media visitor checks cover ordinary and long-term wraps; all96 video/oracle hashes pass. Ordinary tests remain offline without JM/FFmpeg/network/private files. CABAC and POC1/2 gap end-to-end qualification, B gaps, MMCO-reset gap combinations and mixed frame/field storage remain open.

Separate-field CABAC gap qualification: 432 owned CABAC I/P gap streams extend ordinary I0–P2–P3, wrapped I0–I14–P0 and retained-long-term wrapped sequences (144 each). All init banks0/1/2, depths8/10, field orders, skip/P16, filters and ASO pass saved JM pixels, reset, rewind, presentation and seek. Syntax gates CABAC init, single active L0, exact skip/P16 dispatch, zero MVD and entropy termination. Matched skip/P16 controls and short/long marking controls require identical pixels. Selected media visitors pass all three timing contracts. Every video/oracle hash is verified; regenerated existing144 CAVLC gap streams remain unchanged. Existing production gap handling satisfies the new matrix with no further decoder change. Generation uses explicitly supplied JM only; ordinary tests are offline with no JM/FFmpeg/network/private samples. POC1/2 gap playback, B gaps and MMCO-reset gap combinations remain unqualified.

Separate-field POC1/2 gap qualification: 1152 owned POC1/2 I/P gap streams extend ordinary, wrapped and retained-long-term cases with CAVLC and every CABAC init bank, depths8/10, field orders, skip/P16, filters and ASO. POC1 uses explicit per-field delta0/1, zero top-bottom offset, a single cycle offset2 and non-reference offset-1; POC2 uses inferred decode order with no POC syntax. Acceptance independently gates each inferred frame order, absence of POC LSB syntax,14 inferred frames for wraps (one for ordinary gaps) and FrameNumOffset0→16 before accepting every saved JM pixel, reset, rewind, presentation and seek. Matched CABAC skip/P16 and short/long controls stay identical. Selected media visitors pass; all1152 video/oracle hashes and the separate refusal hash are checked. Existing576 POC0 gap streams regenerate unchanged. The initial POC1 bottom-first draft used top-bottom offset1 and zero delta, producing invalid nonzero IDR bottom POC; JM accepted that draft but native normative validation correctly refused it. A compact two-field owned reproducer is generated separately as avc-field-poc1-invalid-idr-bottom.mp4 and tested only for the exact IDR refusal/reset contract. The corrected matrix uses zero IDR POC and explicit complementary-field delta1; no production validation was weakened. Generation alone needs explicitly supplied JM; ordinary tests are offline with no JM/FFmpeg/network/private media. More complex POC cycles/deltas, B gaps, MMCO-reset gap combinations and mixed frame/field storage remain unqualified. The zero-IDR rule follows H.264 section8.2.1.

Separate-field B-gap POC1/2 acceptance: 3072 owned eight-sample streams extend complete and mixed-parity long-term source/co-located cases, CAVLC/CABAC init0/1/2, 8/10-bit fields, both field orders, spatial/temporal direct, skip/coded and filters. POC1 uses an inferred missing frame at POC2/2 and explicit deltas for reordered B fields; POC2 uses no POC syntax, P3 reference fields at POC6/6 and non-reference B3 fields at POC5/5 with the same frame_num as the reference. Acceptance gates frame-number sequences, exact inferred/decoded POCs, modulo32 PicNum reordering, selected reference identities/parities, pixel-free gap storage and MMCO. Every saved JM pixel passes native decode, reset, presentation, rewind and seek; matched skip/coded and causal complete/mixed-parity long-term controls pass. All four 768-stream manifest/video/oracle hash sets are verified. Eight selected media visitors pass reordered frame timing. No decoder change was needed. Generation requires explicit JM only; ordinary tests remain offline without JM/FFmpeg/network/private media. General B gap wraps/MMCO resets, richer POC1 cycles/deltas and mixed frame/field storage remain unqualified.

Separate-field B-gap frame-number wrap acceptance: 1536 owned POC0 streams decode I0, I14, future reference P0 and reordered non-reference B1 (complete and mixed-parity long-term source/co-located families, 768 each). Thirteen pixel-free frames are inferred before I14 and frame15 before P0; selected L0 is real I14 and selected L1 is real P0. Modulo32 modifications select I14 with wrapped PicNum-3/-4 and P0 with PicNum1/0. Explicit MMCO1 releases gap9/10 before I14 and gap11/12 before P0, leaving room for both complementary fields and mixed short/long marking. Source-long cases initially mark the complete I0 pair long-term, then MMCO2/6 replace it with complete or mixed I14; this avoids filling a gap sequence while the initial pair is mixed. Initial drafts exceeded JM reference capacity at gap15 or mixed P0 and were corrected in fixture marking without weakening decoder validation. All 1536 saved JM outputs pass exact native/display pixels, reset, rewind, seek, skip/coded equivalence and causal long-term/parity-row controls; four representative media visitors pass reordered timing. Both 768-record video/oracle manifest hash sets are verified. Existing decoder handles the accepted matrix without production changes. Shared intra generators gain optional explicit short/long forgetting, with unchanged defaults. Generator command is generate_avc_field_b_longterm_samples.py --gap --wrap with optional --mixed-parity and explicit --jm-decoder; ordinary tests require no JM, FFmpeg, network or private media. POC1/2 B wraps, B MMCO5/reset-gap combinations and mixed frame/field storage remain unqualified.

Separate-field B-gap POC1/2 wrap acceptance: 3072 owned eight-sample streams extend the wrap complete/mixed long-term source/co-located matrix across CAVLC/CABAC init0/1/2, depths8/10, field orders, spatial/temporal direct, skip/coded and filters. POC1 I14 fields use POC28/29, reference P0 uses POC36/37 and reordered non-reference B1 uses POC34/35. POC2 uses no explicit POC syntax: I14 POC28/28, reference P0 POC32/32 and same-frame_num non-reference B0 POC31/31. Acceptance independently checks all14 inferred frame POCs (2*frame_num), FrameNumOffset0→16, exact decoded POCs, active wrapped PicNum references/parities and MMCO1/2/4/6. Every saved JM pixel passes native and display decode, reset, rewind and seek, matched skip/coded equivalence and causal complete/mixed long-term controls. All four768-record video/oracle manifest hash sets and eight selected media timing visitors pass. The existing production decoder satisfies these streams without changes; ordinary acceptance is offline without JM/FFmpeg/network/private media. Generation alone requires explicit JM and --gap --wrap --poc-type1|2 plus optional --mixed-parity. B MMCO5/reset-gap combinations, general POC1 cycles/deltas and mixed frame/field storage remain unqualified.

Separate-field MMCO5 reset-gap/B acceptance: 1152 owned ten-sample streams (384 each POC0/1/2) decode an initial I0 pair, infer frames1..14 before I15, then wrap to a new I0 whose first field carries MMCO5. Its complementary second field has no reset. After MMCO5 a fresh inferred frame1 precedes reference P2 and reordered B3 (POC0/1) or same-frame_num B2 (POC2). The matrix covers 8/10-bit, both field orders, CAVLC/CABAC init0/1/2, spatial/temporal direct, skip/coded, filters and initially short/long I0 controls. Acceptance gates15 inferred records, exact pre/post-marking POCs and FrameNumOffset16 on the reset field then0, MMCO5 syntax on every slice of only the first reset field, cleared old I0/I15 identities and selection of only the new reset I0/P2 references. Every JM output passes exact native decode-order/display pixels, reset, rewind, timestamps and seek to the reordered B frame. Initial short/long status cannot change any output; skip/coded outputs are identical. All three384-record video/oracle hash sets and six representative media visitor timing checks pass. Existing production decoder satisfies this matrix without changes. Shared intra writers gain an optional MMCO5 command; 96 PCM and576 CABAC default encodings remain byte-identical to HEAD. Generation uses generate_avc_field_b_reset_gap_samples.py with optional --poc-type1|2 and explicit --jm-decoder; ordinary acceptance needs no JM, FFmpeg, network or private media. This qualifies reset on the first field of a coded frame_num0 pair after wrap, not arbitrary non-paired-field/reset arrangements, combined residual/FMO/reset tools, general POC1 cycles or mixed frame/field reference storage. Those remain open. Fixture constraints were checked against H.264 (02/2016), clauses3.36,7.4.1.2.4,7.4.3 and8.2.1; a reset field's FrameNumOffset is calculated before marking resets the state for later fields.

Mixed-reference storage first integration: ReferenceBuffer can transfer initialized state, reference IDs/frame numbers, long-term index limit, exact field POCs and pixel-free gap records into FieldBuffer through an owned conversion callback. AvcDecoder uses this transfer before the first non-IDR separate field, after checking field storage/scratch budget. Complete intra-coded frame references are split into top/bottom compact planes; any empty MBAFF intra motion snapshot is recognized as intra, while non-empty co-located motion conversion remains an explicit unsupported case. This enables MBAFF PCM frames (all macroblocks coded as frames) followed by separate CAVLC P fields without an intervening IDR. Forty-eight owned streams cover 8/10-bit, both field orders, skip/coded, filters and short/long IDR frame references; all JM pixels, output shape, native reset, presentation timing, rewind and seek pass. Long/short controls require identical output and syntax gates require preserved long-term vs short-term list selection. Four representative media visitors pass. Migration unit coverage checks stable identities, exact parity POCs, unknown/known pixel-free gap POCs, preserved long-term limit, initialized marking and atomic failure. All48 video/oracle hashes pass. Production frame-to-field pixel conversion retains original frame_num and takes a checked per-field allocation budget.

The initial PAFF PCM frame fixture failed earlier in IntraCavlcReader, so it could not demonstrate the mixed-storage failure. generate_avc_frame_field_samples.py --paff preserves one owned valid JM two-frame reproducer separately; its passing refusal test gates PAFF frame syntax, the exact reader error and reset/poison behavior. It is not PAFF acceptance. The corrected MBAFF fixture reproduced the precise mixed-storage refusal at sample1 before the fix and now passes acceptance. Inter-coded frame co-located motion conversion, the reverse field-to-frame path, broader PAFF frame geometry and arbitrary mixed/non-paired reference arrangements remain open. Fixture generation alone needs explicit JM; ordinary tests are offline without JM/FFmpeg/network/private media. A stale shared-target codec artifact initially reported the removed refusal; cargo clean -p fvid-codecs forced a current-source rebuild before acceptance and broad validation.

Mixed-reference follow-up verification: the owned --inter-source reproducer adds an accepted full-frame skipped P picture before the separate field. The passing refusal test decodes both initial frame pictures first, then gates the precise unconnected frame-to-field co-located motion error, poisoning and reset. This is not inter-motion acceptance; its one video/oracle hash pair is verified. After a clean current-source codec rebuild, broad AVC validation passed29 MBAFF+95 multislice+3 parameter tests (127 total); full core passed506 with1 ignored. Added long-term/migration checks separately pass three PCM/refusal tests, one marking migration unit and the media visitor. During verification another edit in owned_yuv_rgb.rs introduced an E0308 return-type error; only the LUT getter return was corrected to select its matrix entry, retaining the other draft changes. A fresh current-source inter-motion refusal test then compiled and passed. Full arbitrary mixed-reference support remains open.

PAFF CAVLC frame geometry integration: IntraCavlcReader now admits non-MBAFF frame pictures in an interlaced SPS, reserves width_mbs*height_map_units*2 contexts for frame pictures (one map-unit height for separate fields), and uses the same full frame height for raster neighbour lookup. Calling the ordinary reader for an actual MBAFF frame still refuses; MBAFF dispatch remains required. The existing owned PAFF PCM reproducer no longer fails at frame sample0: the old exact-reader-refusal test was removed and replaced by playback acceptance. Forty-eight PCM frame-to-field streams (24 each short/long IDR marking) cover 8/10-bit, both field orders, skip/coded and filters, with exact JM native/display pixels, reset, rewind, seek and list syntax gates.

PAFF intra context qualification adds72 four-slice frame streams (I4 zero/symmetric AC/biased AC and I16 zero/positive/negative DC, depths8/10, filters, normal/ASO wire order), plus36 joined single-slice frame streams spanning all four raster macroblocks. Syntax gates full four-MB context budget, exact addresses in both rows, frame rather than field scan (AC position1, position4 zero), signed I16 DC, four-block joined parsing and termination. All saved JM pixels, native reset, presentation, rewind and seek pass; ASO stays pixel-identical. Joined vs split controls stay equal for neutral/symmetric patterns and require a prediction difference for biased I4 and nonzero I16 at disabled filtering. The initial symmetric-I4 inequality expectation was incorrect; a separate biased owned pattern provides causal neighbour evidence without changing decoder output. All156 PAFF video/oracle hash pairs are verified. Ordinary tests remain offline without JM/FFmpeg/network/private media; generators alone require explicit JM. Core AVC190 tests, MBAFF29, parameter updates3 and existing FMO18 regressions pass after the geometry change. Wider PAFF CABAC/inter-frame/residual/FMO/tool combinations and inter-motion or reverse field-to-frame reference conversion remain open; these fixtures do not claim universal PAFF support.

### 2026-10-06: PAFF CABAC frame context geometry

Owned `avc-paff-cabac-*` fixtures reproduce the pre-fix CABAC context refusal on a non-MBAFF PAFF frame. The reader now allocates both raster rows for a frame in an interlaced sequence, retains one row factor for separate fields/progressive frames, and still requires the explicit MBAFF reader for adaptive frames. The inter slice context/motion geometry uses the same frame/field height rule; PAFF inter playback qualification remains open.

The 36 checked-in 8/10-bit I16 cases cover zero/positive/negative DC, all deblocking modes and normal/ASO slice order. All video/oracle hashes are checked. Acceptance verifies actual frame syntax, four macroblocks (including addresses 2/3), the allocation limit, signed DC, exact JM pixels, decoder reset, software playback, rewind and seek. Fixture generation explicitly invokes local JM; ordinary tests are offline and do not invoke JM or FFmpeg. The before-fix run failed with the specific CABAC context refusal; after-fix acceptance and 190 core AVC tests passed. Wider PAFF CABAC joined slices, inter pictures, residual tools/FMO and mixed-reference motion remain unqualified.

### 2026-10-06: PAFF CABAC P frames

The 72 owned `avc-paff-inter-*` fixtures reached actual P-picture reconstruction and reproduced `unsupported inter-picture reconstruction tools`: the frame raster reconstructor rejected all interlaced-sequence SPSs. It now distinguishes PAFF full frames from adaptive MBAFF frames, preserving the existing explicit MBAFF dispatch. Together with the CABAC context height correction, PAFF P frames now reconstruct both raster rows.

Acceptance covers 8/10-bit, CABAC init 0/1/2, skip versus explicit nonzero horizontal/vertical MVD, filtering 0/1/2 and ASO. Syntax gates assert P frame/frame_num, four macroblocks including the lower row and MVD [4,4]. Every decoded pixel matches JM across reset, software playback, rewind and seek; skip equals the signed I16 reference and motion differs, providing a causal control. All 72 video/oracle hash pairs match the manifest. This qualifies those P-frame paths, not joined-neighbour motion, B/direct frame paths, richer residual/FMO/weighted/multireference PAFF tools or mixed frame/field reference motion.

Verification for PAFF P-frame admission: 190 core AVC tests and 29 MBAFF + 3 parameter-update regressions passed. Media visitor acceptance covers CABAC PAFF intra and P examples with exact pixels/sample IDs/PTS/durations; the final P-frame sample duration is one container tick (20 ms), not a field-pair duration. This is a corrected test expectation, not a production timing change.

### 2026-10-06: PAFF CABAC B-direct qualification

144 owned `avc-paff-b-*` streams now exercise reordered full-frame B prediction in a PAFF sequence: 8/10-bit, all three CABAC initialization banks, temporal/spatial direct, B-skip/direct, filters 0/1/2 and ASO. They use the previously fixed full-frame context/reconstruction path, without a new production shortcut. Source P frames retain nonzero horizontal/vertical motion. Syntax acceptance verifies actual B frames, direct mode, init bank and all four macroblocks including the lower raster row. Exact JM comparison covers native decode order I/P/B versus display order I/B/P, reset, playback, rewind and seek. Temporal versus spatial direct produces different B pixels; skip/direct and ASO controls are equal. All 144 video/oracle hash pairs were verified. Generation explicitly uses local JM; ordinary tests read checked-in owned files only.

This closes the qualified PAFF B-direct paths for these independent slices. Joined-neighbour motion/direct, explicit B partitions, richer residual/transform/weighted/multireference PAFF tools and mixed frame/field motion remain open; it does not establish universal PAFF or codec completeness.

### 2026-10-06: joined PAFF CABAC motion/direct contexts

The owned generators now support separate `--joined` namespaces: 36 PAFF P streams and 72 PAFF B-direct/skip streams. P/B slices contain four raster macroblocks in one slice, exercising available left/top entropy and motion neighbours across both rows. P block 0 carries MVD [4,4]; later blocks carry [0,0] and must recover the same motion through prediction. All native pixels match JM across reset, software playback, reorder, rewind and seek. Filter-disabled output equals independent-slice controls despite different coded MVD syntax. All 108 final video/oracle hash pairs were verified.

The first fixture draft encoded block 3's MVD context from neighbouring final vectors rather than neighbouring coded differences. Syntax acceptance caught a block-3 entropy error; the generator was corrected and fixtures regenerated before acceptance. This was a fixture defect, not evidence of a decoder defect or accepted refusal. Existing independent P fixture bytes remained identical. Tests read checked-in artifacts without external codecs. Joined CABAC P/B direct neighbour paths are qualified for these fixtures; richer explicit partitions, residual/weighted/FMO PAFF tools and mixed frame/field reference motion remain open.

### 2026-10-06: PAFF CAVLC P/B frame context geometry

108 owned `avc-paff-cavlc-*` streams reproduce the pre-fix `AVC first macroblock outside picture` on a lower-row PAFF P frame slice. The mixed CAVLC reader now sizes the macroblock/context raster from frame-versus-field geometry rather than the MBAFF flag. MBAFF's first-macroblock address multiplier remains separate; PAFF frames use ordinary raster addresses.

Acceptance covers P motion/skip and temporal/spatial B direct/skip, 8/10-bit, filter 0/1/2, independent/ASO/joined slices, actual four-block syntax (including lower rows), first/nonfirst joined MVD, exact JM pixels in decode and presentation order, reset/rewind/seek and independent-vector equivalence with filtering disabled. All 108 video/oracle hash pairs were verified; 190 core AVC tests passed. Generation uses explicit local JM; ordinary tests require neither JM nor FFmpeg/network. PAFF FMO acceptance, explicit partitions, residual/weighted tools and mixed frame/field reference motion remain open.

Post-fix checks: all 9 PAFF acceptance tests, 29 MBAFF + 3 parameter updates, 18 existing FMO regressions and the media pixel/sample/PTS/duration visitor passed offline. The media visitor includes CAVLC PAFF P and reordered B examples. These existing FMO regressions protect neighbouring paths; they are not PAFF FMO acceptance. A concurrently edited player-controls draft is outside this codec qualification.

### 2026-10-06: PAFF full-frame FMO acceptance

480 owned `avc-paff-fmo-*` streams qualify the full-frame FMO path after the PAFF raster/context fixes: map types 0..6, changing-map directions and cycle 2, 8/10-bit, filter 0/1/2, ASO, PCM I frames, nonzero P motion, reordered temporal/spatial B direct/skip. The syntax gate checks the actual PPS map type/parameters and independently expected map units, their PAFF eight-macroblock expansion, group-specific address traversal and complete picture coverage. Exact JM native pixels match in decode order I/P/B, with reset, software display I/B/P, rewind and seek. ASO and skip/direct controls are equal. All 480 video/oracle hash pairs match the manifest. The complete acceptance run passed; the media visitor also passed for selected FMO map/depth/mode extremes with exact samples/PTS/durations.

No production shortcut was added for this qualification. Fixture generation explicitly uses local JM, while ordinary tests remain offline and FFmpeg/JM-free. This qualifies those PAFF FMO motion/direct paths, not residual/transform, weighted/multireference partitions or mixed frame/field reference motion. Codec completeness is still unproven.

### 2026-10-06: PAFF FMO signed residual acceptance

480 owned PAFF FMO P/B streams now qualify signed luma AC (frame-scan position 1), chroma DC and chroma AC, plus no-residual controls, across all map types 0..6/changing directions, 8/10-bit, filters and ASO. Each group starts with an explicit motion/residual macroblock then skipped addresses. Syntax acceptance checks the exact coded pattern, coefficient positions/signs, motion, every group address and eight-block coverage. Configurations match the independently qualified FMO fixtures. Every pixel matches JM through native decode reset and software reorder/rewind/seek; filter-disabled residual controls change P pixels while I pixels remain identical. All 480 video/oracle hash pairs match the manifest. The full residual acceptance test passed.

The shared generator's default output was independently reconstructed and all 480 prior FMO MP4s remained byte-identical. Generation is explicit with local JM; ordinary tests remain offline without JM/FFmpeg. This extends qualification without a production bypass. Transform8/bypass, richer partitions/weighted tools, all-slice neighbour residual combinations and mixed frame/field reference motion remain open.

### 2026-10-07: PAFF CAVLC weighted multireference partitions

192 owned PAFF P/B streams qualify explicit luma/chroma weighting, positive/negative/zero weight values and offsets, active two-reference B lists, selected reference indices 0/1, and 16x16/16x8/8x16/8x8 partitions across 8/10-bit, filters and ASO. Syntax gates verify actual PPS weighted flags, slice denominators/tables, active references, partition prediction modes/indices and motion. All native pixels match JM through reset, software reordered presentation, rewind and seek. Identity controls preserve I pixels and change P pixels; alternate reference selection changes B pixels. ASO controls are equal. All 192 video/oracle hash pairs match the manifest; complete acceptance passed.

This qualifies existing owned reconstruction paths without adding a production shortcut. Fixture generation explicitly uses local JM; ordinary tests are offline without external codecs. PAFF CABAC weighted paths, implicit weighting, joined/residual/FMO combinations, richer profiles/tools and mixed frame/field reference motion remain open. Full codec completeness is still unproven.

### 2026-10-07: asymmetric PAFF implicit bipred acceptance

384 owned streams now qualify PAFF implicit bipred against ordinary averaging with asymmetric POC distances: references at 0/8, B at 2/6, selected reference index 0/1, 16x16/16x8/8x16/8x8 Bi partitions, 8/10-bit, filters and ASO. Syntax gates verify actual PPS bipred 2 versus 0, POCs, active references, partition/motion values and absence of an explicit B weight table. All pixels match JM across reset and software reordered playback/rewind/seek with nonuniform PTS. Causal controls keep I/P identical and change B pixels. All 384 video/oracle hash pairs match the manifest; full acceptance passed.

The shared explicit-weighted generator retained byte-identical defaults for all 192 previous MP4 fixtures. Generation explicitly uses local JM; ordinary tests are offline without FFmpeg/JM. CABAC implicit/explicit weighted paths, long-term/distance-edge implicit cases, joined/residual/FMO combinations, richer codec profiles/tools and mixed frame/field reference motion remain open. No universal codec-completeness claim is made.

### 2026-10-07: PAFF CABAC explicit weighted partitions

576 owned PAFF CABAC streams qualify explicit weighted P and two-reference Bi B prediction: 8/10-bit, init 0/1/2, selected reference 0/1, 16x16/16x8/8x16/8x8 partitions, identity/weighted tables, filtering and ASO. Syntax gates verify actual PPS entropy/weight flags, slice tables/denominators/init, active references, partition modes/indices and MVDs. Every native pixel matches JM across reset and software reordered playback/rewind/seek; weights/reference selection provide causal pixel differences and ASO controls remain equal. All 576 video/oracle hash pairs match the manifest. Complete acceptance passed, and selected media visitor cases passed with exact pixels/samples/PTS/durations.

All 576 CABAC oracles also match their corresponding independently qualified CAVLC oracles; this equivalence is retained in acceptance. Source PCM and all parameters are owned synthetic content. Generation explicitly invokes local JM; ordinary tests are offline without FFmpeg/JM. No production shortcut was introduced. Implicit CABAC, joined/residual/FMO weighted combinations, richer profiles/tools and mixed frame/field reference motion remain open; universal codec completeness is unproven.

### 2026-10-07: PAFF CABAC implicit bipred acceptance

1,152 owned PAFF CABAC implicit/average streams now qualify asymmetric POC weighting (references 0/8, B at 2/6) across 8/10-bit, init 0/1/2, reference indices 0/1, four Bi partition shapes, filtering and ASO. Syntax gates verify the actual PPS bipred mode, POC/init, active references, partition motion and absence of explicit B weights. Every native pixel matches JM and the independently qualified CAVLC oracle. Causal controls keep I/P pixels identical and change B relative to averaging. Decoder reset, reordered software playback, rewind and seek pass with nonuniform PTS. All 1,152 video/oracle hashes were verified. The media visitor passed exact pixels/samples/PTS/durations for CABAC implicit/average extremes.

All 576 existing explicit CABAC MP4s were independently reconstructed using default generator arguments and remained byte-identical. Generators invoke local JM only explicitly; ordinary tests are offline without FFmpeg/JM. Qualification uses the existing owned decoder without production shortcuts. Long-term/distance-edge implicit cases, joined/residual/FMO weighted combinations, richer profiles/tools and mixed frame/field reference motion remain open; the overall codec-completeness goal is not achieved.

### 2026-10-07: retained frame motion to separate direct B fields

Frame-to-field DPB migration now retains the source motion map, shared by both parity views through one Arc. Field direct selects the co-located block using current-field parity and the source frame/MBAFF pair geometry. Spatial colZero sees the original vector units; temporal direct applies frame-to-field vertical scaling and remaps stable picture identity to the current parity. Field-coded source pairs retain their stored reference parity and field vector units. Reservation includes the larger full-frame motion representation and its pair flags without counting the shared map twice.

The previous inter-frame migration refusal fixture is now exact JM/reset acceptance. 128 additional owned CAVLC streams cover 8/10-bit, PAFF frame sources, MBAFF frame-coded/field-coded/mixed sources, both current field orders, spatial/temporal direct, skip/coded syntax and zero/nonzero motion controls. Native decode and reordered software playback, reset, rewind and seek match independent JM pixels. Nonzero P motion changes the reference picture; temporal B controls also change pixels. Independently indexed unit tests cover FLD/FRM and FLD/AFRM selection, L0 preference/L1 fallback/intra cells, invalid coordinates, identity/parity mapping and temporal-only vector scaling. Migration units verify real source pair modes/nonzero vectors, shared allocation and release on reset.

Fixture generation explicitly invokes local JM; ordinary tests consume owned artifacts offline without JM/FFmpeg. Final offline verification passed 193 AVC unit tests, 29 MBAFF tests, 3 parameter-update tests, the 128-case direct-field acceptance, the original inter-migration acceptance and the media visitor. This closes the former frame-to-field motion-storage refusal for the qualified paths. Reverse field-to-frame migration, CABAC/weighted/residual combinations of mixed-reference direct and other codec/profile/tool gaps remain open; overall codec completeness is not achieved.

### 2026-10-07: native field pairs to full-frame references and direct motion

The original `AVC mixed field/frame reference storage is not connected` refusal was reproduced at sample2 of an owned field-pair/full-P fixture, after the initial native pair matched JM. Complete, equally marked pairs now migrate to frame DPB with stable IDs, frame numbers, both post-marking field POCs, long-term indices and limit, initialized state and pixel-free gaps. Unknown gap POCs remain unknown. Native compact motion maps join into field-pair-addressed storage without losing raw vector units or selected reference parity; inherited shared frame motion is reclaimed without another snapshot. Full-frame direct selects the nearest co-located field (bottom on ties), keeps raw vectors for spatial colZero and applies vertical scaling only for temporal direct. Migration reserves both the reference representation and one temporary frame/motion allocation.

192 owned CAVLC cases qualify full P/B after native I/P fields: 8/10-bit, PAFF/MBAFF, both field orders, short/long reference marking, P and spatial/temporal B, skip/coded and zero/nonzero motion. The P fields have different signed vectors by parity and macroblock. Exact JM pixels, native reset and software display order/PTS, rewind and seek pass; nonzero reference-motion and temporal-B controls change pixels. Another 64 cases qualify fields→frame→fields→frame with inter references throughout. The media visitor passes exact pixels, sample identities, PTS and durations for both one-way and repeated migrations. All 258 video/oracle hash pairs (256 acceptance + 2 refusal) match the manifest.

Two additional compact fixtures retain incomplete and mixed-marked reference stores. Their preceding field pictures match JM; the full frame reaches its exact storage-shape refusal and the decoder requires reset. These are refusal tests, not acceptance: a unified DPB is still required to retain those stores across frame decoding. Unit tests qualify metadata/gap transfer, refusal without discarding a live store, independently indexed motion weaving and frame selection, temporal-only scaling, parity, real decoded-store release and marking preservation. Final offline checks passed 198 AVC unit tests, 29 MBAFF tests, 3 parameter-update tests, the 192-case acceptance and the prior 128-case frame→field direct acceptance. Fixture generation explicitly uses local JM; ordinary tests require neither JM nor FFmpeg. Mixed marking/incomplete stores, mixed I/P field-pair combinations, CABAC/weighted/residual crossings and broader codec/profile/tool gaps remain open; overall codec completeness is not achieved.


2026-10-07 unified field/frame reference follow-up: the two historical partial/mixed refusal fixtures now pass full playback acceptance. Canonical field storage retains incomplete references and complete one-short/one-long pairs across intervening full frames; temporary frame views preserve the individual field identities, marking and motion. Another 64 owned PAFF CAVLC fixtures qualify later prediction from retained individual fields (8/10-bit, both field orders, motion/zero, skip/coded and ASO), exact saved JM pixels, reset, rewind, seek and media timing. All 64 video/oracle hash pairs were verified. Offline AVC unit tests pass 200 cases and the media visitor passes. Ordinary tests use checked-in artifacts without FFmpeg, JM or network. Both-long pairs with different indices, mixed motion provenance, CABAC/weighted/residual/B crossings and broader marking/gap/sliding combinations remain unqualified; overall codec completeness is not achieved.


2026-10-07 tight field-budget regression follow-up: the shared test reserve still counted two compact field maps per reference, although migrated full-frame maps retain additional MBAFF flags and share one Arc between parities. Updating the test reserve to max(two field maps, one frame map) restores the two existing exact-budget filtering/multireference regressions without increasing the decoder limit or weakening its admission checks. Both selected regressions and all 200 AVC unit tests pass offline.

Full offline follow-up verification passed: 116 avc_multislice tests, 29 MBAFF tests and 3 parameter-update tests, in addition to the 200 AVC unit tests. This includes the unified retained-field acceptance and the previously failing tight-budget cases.


2026-10-07 CABAC retained non-paired field qualification: 96 owned 8/10-bit PAFF CABAC streams exercise I fields, MMCO removal of one old parity, a full P frame that removes the newer complete pair, later P fields explicitly selecting the surviving old I field, and another full P frame. Both field orders, all three CABAC init banks, frame motion/skip, field coded/skip and ASO match every saved JM pixel after reset, software rewind and seek. Syntax gates confirm CABAC, field/frame boundaries, frame numbers, the exact removal operations and later reference-list modification. The retained-field output stays identical across intervening full-frame motion/skip controls. All 96 video/oracle hash pairs were verified; ordinary tests use only checked-in files. No decoder implementation change was needed for these qualified paths. Residual/weighted/B crossings and broader marking/gap combinations remain open.

Normative scope correction: H.264 (02/2016) 7.4.3.3, notes 3 through 6, requires consistent short/long marking of both retained fields after all MMCO commands, and identical long-term indices when both are long-term. Historical one-short/one-long and differing-long-index shapes are not valid-stream codec support requirements; acceptance of the saved mixed shapes demonstrates decoder robustness, not standard-conforming coverage. Those historical artifacts and tests remain preserved. The earlier list of remaining mixed-long-index codec gaps should be read with this correction; compliant non-paired references remain a separate valid requirement.


2026-10-07 signed CABAC residual across canonical frame/field references: 1,152 owned PAFF streams qualify luma4 AC, chroma DC/AC and combined signed coefficients, empty-CBP-coded blocks and no-residual controls. The matrix covers 8/10-bit, both field orders, all three CABAC init banks, ASO, frame-only/field-only/both residual stages, and later fields selecting either the retained non-paired I field or the intervening full P frame. A final full P frame consumes the new field pair. Acceptance explicitly decodes CABAC macroblock syntax, checks CBP, counts, every signed luma level at the distinct frame/field scan position, chroma levels, reference-list selection and marking; every native/display pixel matches the saved JM oracle after reset, rewind and seek. Controls prove that frame residual survives selection by later fields, field residual affects the final frame, a separately selected old I field remains independent of intervening frame residual, empty-coded residual equals no-residual, and init/ASO do not change pixels. All 1,152 video/oracle hash pairs match. Selected media visitors pass exact pixels, sample identities, presentation times and durations. Shared generator defaults remain byte-identical against HEAD. These paths needed no decoder implementation change; generation alone uses explicitly supplied local JM, and ordinary tests are offline without external codecs. This qualifies split-slice, filter-disabled, QP50, 4:2:0 4x4-transform P crossings with one signed coefficient per coded block; richer coefficients, transform8/bypass, filtering, weighting and B crossings remain separate, and overall codec completeness is not achieved.


2026-10-07 weighted P canonical frame/field qualification: 1,296 owned CABAC PAFF streams exercise identity, fractional positive and negative luma/chroma weights with nonzero offsets. They cover 8/10-bit, both field orders, three init banks, ASO, frame-only/field-only/both weighting, skip/coded/signed luma+chroma residual and later field selection from either the retained non-paired old I field or the intervening full P frame. The final P frame applies identity weights while consuming the new field references, isolating propagation of previously weighted pixels. Acceptance checks actual PPS weighted prediction, denominators2/1, every per-plane weight/offset, reference-list modification and marking, actual skip/coded/residual syntax and coefficients, every native/playback JM pixel, reset/rewind/seek and sample identity/PTS. Identity controls prove weighting changes the appropriate pictures and survives subsequent frame/field references; separately selected old I fields remain independent of full-frame weighting. ASO/init controls match. All 1,296 video/oracle hash pairs are verified; selected media timing/pixel visitors pass. Shared unweighted generator defaults remain byte-identical against HEAD. This required no decoder implementation change. Ordinary tests are offline and consume only saved owned artifacts, with no FFmpeg/JM invocation. Scope: one active reference, P16, split slices, filter-disabled QP50 4:2:0 and 4x4 signed residual. Multireference/subpartition/filtering/transform8/bypass weighted crossings, explicit/implicit B crossings and broader codec/profile/tool gaps remain open; overall codec completeness is not achieved.


2026-10-07 CABAC B-direct canonical non-paired reference qualification: 576 owned 8/10-bit PAFF streams retain a complete initial I pair and an individual field from a later I pair while decoding a newer complete I/P pair, a non-reference full B frame, later P fields explicitly selecting the old individual field, and a final full P frame. The matrix covers both field orders, all three init banks, ASO, spatial/temporal direct, B skip/direct and positive/negative combined luma/chroma residual, with intra/zero-motion/nonzero-motion co-located sources. Actual syntax gates B type/non-reference status/POC/direct mode, residual counts, field/frame boundaries, marking and later reference selection. Exact native pixels (mapped from decode to presentation order), software sample IDs/PTS, reset/rewind/seek match every saved JM picture. Skip/direct and init/ASO controls match; B residual changes only the non-reference B output, and later selected individual-field output stays independent of co-located source motion. All 96 temporal-motion controls change B pixels. Selected media visitors pass exact pixels, reordered timing and durations. All 576 video/oracle pairs and the saved refusal hash are verified. No decoder implementation change was needed.

The first temporal draft removed part of the co-located source's reference pair, so that reference was absent from the full-frame list0; JM correctly rejected it. The separate owned avc-unified-b-invalid-temporal-reference.mp4 preserves that invalid condition. Its refusal regression first matches both decoded field-pair outputs against equivalent valid saved pixels, then gates actual temporal/full-B syntax, the precise co-located-reference-absent refusal, poison/reset and repeatability. It is not playback acceptance. Corrected acceptance retains the complete initial reference and removes a field only from a different pair. Generator defaults for existing frame3 field references remain byte-identical. Generation explicitly uses local JM; ordinary tests are offline with no external codec invocation. Scope: single-MB slices, filter-disabled QP50 4:2:0, direct/skip and 4x4 signed combined residual. Explicit/implicit weighted B, other B partitions, transform8/bypass/filtering and broader codec/profile/tool gaps remain open; overall codec completeness is not achieved.


2026-10-07 explicit/weighted CABAC B canonical non-paired reference qualification: 2,304 owned streams retain a complete initial I pair and a non-paired field from a different I pair while forming the newer complete I/P pair, a non-reference full B picture, later fields explicitly selecting the old non-paired field, and a final full P frame. Coverage includes 8/10-bit, both field orders, three init banks, ASO, intra/motion source pairs, two selected reference indices, L0/L1 16x16 and Bi16x16/16x8/8x16/8x8 partitions, ordinary averaging, explicit identity/non-identity and implicit weighting. Acceptance gates actual PPS mode, both active list counts, every explicit per-plane weight/offset, partition shapes/directions/selected references/MVD, zero residual, native frame/field boundaries, marking and later individual-field selection. Every native pixel (mapped from decode to presentation order), software picture identity/PTS, reset/rewind/seek matches saved JM output. Explicit identity equals ordinary averaging; implicit weighting equals averaging for single-list prediction and changes bidirectional B output. Changing selected reference indices changes the B picture in all 1,152 matched controls while other pictures remain identical. Weighting leaves the other pictures and later retained-field output unchanged; init/ASO controls match. All 2,304 video/oracle hash pairs were verified, and selected media timing/pixel visitors pass. No decoder implementation change was required. Generation alone uses explicitly supplied local JM; ordinary tests are offline without external codecs. Scope: full PAFF B frames crossing canonical field ownership, split single-MB slices, QP50/filter-disabled 8/10-bit 4:2:0, explicit motion with zero residual. Weighted B-field crossings, richer residual/subpartition/transform8/bypass/filtering combinations and other codec/profile/tool gaps remain open; overall codec completeness is not achieved.

## AV1 show-existing metadata (2026-10-07)

Native AV1 now accepts show-existing presentation timestamps and display frame
IDs for hidden key/intra-only references. Stored IDs are validated and stale
references are invalidated at the normative ID window, including wraparound.
At that milestone inter frame IDs remained unsupported; see the update below.
This is not full AV1 conformance.

26 owned OBU acceptance fixtures were independently decoded by libaom; seven
malformed streams assert specific refusals and reset behavior. The original
decoder reproduced the timing/frame-ID unsupported error on the valid fixture.
24 WebM variants verify native pixels, relative display timing, rewind and seek.
Ordinary tests are offline and require neither FFmpeg nor libaom.

## AV1 inter-frame IDs (2026-10-07)

Native inter headers now read each of the seven reference-ID deltas and validate
the expected stored ID modulo the declared width. The decoder also validates
current-ID progression against the previous decoded frame, resets that state
on sequence changes and decoder reset, and restores it for shown existing keys.

175 owned acceptance streams (six slot/window cases plus shown-key ID restoration and 168 crossings of all
84 legal delta/ID-width combinations at the reference-window edge and wrap)
passed native decoding and independent libaom flat-pixel validation. Seven
specific reference mismatch refusals and three current-ID progression refusals
are permanent regressions. Tests exercise reset and repeat decoding offline.
The pre-fix parser reproduces `AV1 inter frame IDs not implemented` on the
accepted next-ID fixture. Current-ID malformed fixtures are independently
refused by libaom. The pre-fix native decoder accepted the repeated-ID malformed
stream; its new refusal regression failed on the old decoder and passes after
the fix. At this milestone short reference signaling was still unsupported;
see the following update. Nonidentity global motion, film
grain, layered operating points and separate header/tile groups remain gaps.

## AV1 short reference signaling (2026-10-07)

Native AV1 now implements the normative set_frame_refs selection from LAST,
GOLDEN and all eight stored order hints. It chooses backward and forward
references with the required slot tie ordering, applies the fallback and
validates that LAST/GOLDEN are forward references. Order hints remain stored
when reference IDs invalidate a picture; reset, sequence changes and showing
an existing key restore the corresponding hint state.

298 owned accepted streams compare short and explicit maps, all eight
order-hint widths, frame IDs on/off, ties, future/past references, modular wrap
and boundary/same LAST/GOLDEN slots. Every stream independently passed libaom
flat-pixel decoding. Native acceptance additionally checks the exact seven-slot
map and repeat decoding after reset. Two malformed anchor streams assert
specific refusals and error/reset behavior; libaom independently rejects LAST
and GOLDEN for the same reason. Ten owned WebM variants verify display clock,
rewind and sync seek. The original parser reproduces the specific unsupported
short-reference error on an accepted stream.

33 AV1 unit tests and the three integration suites pass offline. Integration
verification used a copy of main plus these changes, excluding concurrent
unqualified transform-pool/HEVC drafts in the primary checkout. Global motion,
film grain, temporal motion fields, updated segmentation maps and active
segment features (see the following partial inheritance milestone), layered operating
points and separate header/tile groups remain outside this completed milestone.

## AV1 segmentation inheritance foundation (2026-10-07)

Header parsing now implements segmentation update-map/temporal/update-data
flags, inherits the selected primary reference feature table, replaces it on
update-data and clears it when segmentation is disabled. Signed values retain
normative clipping. The flags and feature table are stored with each header.

Native playback accepts inherited, unchanged all-zero segment maps. Features
in unused segments are retained correctly, including positive/negative ALT_Q,
filter deltas, forced reference, skip/global flags, and zero-valued active
ALT_Q/filter deltas. Map updates and nonzero/forced active-segment tools still
return explicit unsupported errors. This does not complete segmentation.
Every admitted native picture at this milestone has an implicit zero map;
future map reconstruction must replace this invariant with stored segment IDs
and per-block feature application before accepting nonzero maps.

224 owned seven-frame acceptance streams cover all eight reference slots and
seven primary roles, publishing/inheriting data, disable/reenable, signed
clipping, restoration and explicit data clearing. Independent libaom validation
checks six shown flat frames in each stream. Native tests check pixels, reset
and repeat, with additional header-level assertions on flags and exact tables.
Sixteen WebM variants verify relative display intervals, rewind and sync seek.

Two valid map-update streams (temporal off/on) and an active ALT_Q stream pass
libaom but are native refusal/reproduction tests, not acceptance. Enable their
native acceptance with the eventual map/active-feature fixes. A separate
truncated-data stream has a specific parsing
refusal. The pre-fix parser reproduces its exact inherited-segmentation refusal
on an accepted stream. All ordinary checks run offline without FFmpeg/libaom.
34 AV1 unit tests plus the added map-header/refusal test and all four root
AV1 integration suites passed. Root checks exclude parallel unqualified
transform-pool/HEVC drafts via an isolated copy of main plus these changes.

## AV1 active ALT_Q on inherited zero maps (2026-10-07)

The native decoder now applies segment-zero ALT_Q to the current qindex, clips
the result to 0–255 and then applies plane DC/AC deltas. The existing lossless
array selects the corresponding transform path. Forced reference/skip/global features still return explicit errors, as do
updated segment maps. Active filter deltas are covered by the next milestone. This is another step toward full segmentation, not a
claim of complete segmentation or AV1 conformance.

25 owned streams with nonzero residuals compare every reconstructed shown pixel
against independent libaom YUV output: six lower-clamp cases at qindex 0,
thirteen signed-offset cases at qindex 64 and six upper-clamp cases at 255.
Their entropy comes only from owned synthetic pattern videos made by
scripts/av1_fixture.c. A native extraction helper checks the expected template
configuration; pure Python builds the final segmentation headers. Six WebM
variants additionally check exact pixels, relative timing, rewind and sync seek.
The previous flat active-ALT_Q refusal is now native acceptance and its old
refusal expectation is removed.

The pre-fix decoder reproduces the active-feature unsupported error. Removing
only ALT_Q from dequantization while retaining its acceptance causes a real
pixel mismatch on the lower-clamp residual fixture; restoring the change
passes. 35 AV1 unit tests and five root integration suites pass offline without
FFmpeg or external codec execution. Root verification isolates these changes
from parallel unfinished HEVC drafts. Maps and the remaining segment features
still need implementation and their own acceptance coverage.

## AV1 segment-zero ALT_LF acceptance

All four ALT_LF features now adjust segment-zero loop-filter strengths, with
signed addition and clipping before reference/mode deltas. The delta scale
uses the adjusted level. Nominal plane-enable rules remain in effect. This
currently covers inherited implicit all-zero segment maps only. Updated maps,
forced reference/skip/global features and per-superblock delta-LF remain gaps.

82 owned synthetic streams compare every shown pixel against saved independent
libaom output, including signed limits, clipping, the level-32 threshold,
reference/mode deltas and nominal zero levels. Ten WebM variants verify pixels,
timing, rewind and sync seek. All fixtures are generated separately; ordinary
tests run offline without FFmpeg or libaom. The old decoder fails with the
specific active-feature refusal. Removing only the ALT_LF adjustment causes
a pixel mismatch on av1-alt-lf-f1-d-64-refmode0.obu. Restoring it passes.

35 AV1 core tests and 15 tests across six root AV1 integration suites passed.
Compound GLOBAL_GLOBALMV mode classification remains a source-audit lead
requiring its own synthetic reproducer and qualification; these results do
not claim complete compound filtering or AV1 conformance.

## AV1 spatial, temporal and inherited segmentation maps

The picture decoder now reconstructs and retains 4x4 segment-ID maps with
reference pictures. Spatial decoding uses tile-bounded neighbor prediction,
the segment-ID CDF and negative deinterleaving. Temporal updates read the
prediction flag and use the minimum ID under the current block in the primary
reference map. Unchanged maps are copied exactly at wrapup; disabling
segmentation clears the map. Allocation and retained-reference accounting
include the maps. ALT_Q, lossless selection and all four ALT_LF features now
use each block's segment instead of assuming segment zero. Forced reference,
skip and global segment features still fail explicitly.

48 owned streams cover 1–8 active segments, forward/reverse ID patterns,
intra-key and inter spatial updates, temporal prediction flags both true and
false, unchanged maps, disabling and re-enabling segmentation, nonzero residuals,
and per-segment quantizer/filter deltas. All 288 shown pictures compare every
sample against saved independent libaom pixels. Exact decoded maps are checked
for all seven coded pictures per stream. 48 WebM variants additionally verify
pixels, rewind and sync seek. Two old flat map-update refusal expectations
are promoted to acceptance without changing the fixture bytes.

Fixture generation consumes committed symbol records extracted only from the
owned ALT_Q synthetic stream, inserts the normative map symbols and optionally
uses the standalone libaom range writer/reference decoder. Ordinary tests use
only saved files and run offline without external codecs. The pre-fix decoder
reproduces the specific map-update refusal on the new valid nonzero-map input.
Changing dequantization back to segment zero causes an independent pixel
mismatch on av1-seg-map-n2-q1-lf0.obu; restoring it passes.

35 AV1 core tests and 18 tests across seven root AV1 suites passed. At this stage these
fixtures used disabled CDF adaptation and one tile; adaptive qualification
is covered below. Multitile map qualification, mixed lossless/lossy segments,
forced segment tools, delta-LF
and the other documented AV1 tools remain to qualify or implement. This
milestone does not establish complete AV1 or cross-codec conformance.

## AV1 adaptive segmentation CDFs and reference publication

The map matrix now has 144 owned OBU/YUV/WebM triples: the original 48
nonadaptive inputs, 48 with tile CDF adaptation and disabled frame-end CDF
publication, and 48 with both adaptation and reference publication. Header
assertions verify that these are the actual encoded modes. All 864 shown
pictures and all decoded segment maps match the saved independent oracle.
Adaptive and reference-publication variants preserve the same pixels/maps
as their corresponding nonadaptive inputs, and all original fixture hashes
remain unchanged. Tests run offline without external codec tools.

The owned symbol records now identify the CDF table and context for each
adaptive read. The Python generator independently tracks distributions,
adaptation counts and primary-reference count resets; only the standalone
fixture range writer and pixel oracle use optional libaom during generation.
This qualifies the existing native adaptive decoder and CDF-reference state
without introducing an external codec into production. Discarding updates
only to SEGMENT_ID/SEGMENT_ID_PREDICTED CDFs reproduces a failure on the valid
av1-seg-map-n1-q0-lf0-adapt.obu stream. Restoring accumulation passes the
complete matrix. All 19 tests in seven root AV1 suites passed offline.
Multitile and mixed lossless/lossy maps, forced segment tools
and the other codec gaps remain separate implementation/qualification work.

## AV1 mixed lossless/lossy 16x16 intra blocks

A valid mixed-segment frame with SELECT transform mode exposed an invalid CDF
access: the intra decoder attempted to read tx_depth for a lossless 16x16
block. AV1 [read_tx_size](https://aomediacodec.github.io/av1-spec/#tx-size-syntax)
returns 4x4 immediately for a lossless block and consumes no size symbol.
The decoder now applies that early condition using the current block's segment.
The pre-fix native decoder reproduces the specific invalid-CDF error on
av1-mixed-lossless-q1-mask1-tx1-adapt0.obu; the fixed decoder accepts it.

504 owned short streams cover all fourteen mixed assignments of four 16x16
blocks, base qindices 1/64/255, largest/selected transform modes, and disabled
/enabled CDF adaptation. Segment zero has ALT_Q=-base and is lossless; segment
one has ALT_Q=0 and is lossy. 168 streams have zero residuals and 336 add signed
DC coefficients in every plane's lossless blocks. Header assertions verify
that the inputs are truly mixed. Every reconstructed Y/Cb/Cr sample and each
segment map matches saved independent libaom output. Each signed-residual
fixture changes samples in all three planes. 504 WebM variants verify pixels,
rewind and sync seek; ordinary tests execute no generator or external codec.

The pure Python fixture writer emits the normative symbols and uses the optional
standalone range writer only during generation. No source video is an input.
Bypassing only the native lossless WHT reconstruction causes a real pixel
mismatch on av1-mixed-lossless-q1-mask1-tx0-adapt0-dc1.obu; restoration passes.
35 AV1 core tests and 21 root tests in eight AV1 suites pass offline.

This qualifies the stated 16x16 intra mixed-block cases. Broader inter prediction,
other mixed block geometries, multitile maps, forced segment tools and the other
codec gaps remain to implement or qualify; complete codec conformance is not
established by this matrix.

## AV1 mixed lossless/lossy single-reference inter blocks

672 owned synthetic 32x32 streams qualify four 16x16 identity GLOBALMV
blocks with mixed lossless/lossy segments. The matrix covers all seven logical
reference roles, qindices 1/64/255, four mixed maps and their complements,
largest/SELECT transforms, CDF adaptation off/on, and signed DC residuals in
all three planes. Two hidden frames seed distinct pixels in physical slots
0 and 7; every selected role maps to slot 7, which each shown frame refreshes.
This distinguishes correct reference selection from accidentally using slot 0.

Native acceptance compares every shown pixel with saved independent libaom
output and every frame's segment map, then verifies WebM timestamps, rewind
and sync seek. Ordinary tests use no FFmpeg, libav, libaom or generation tool.
Forcing only physical reference selection to slot 0 makes pixel acceptance
fail; restoring the native implementation passes. This stage qualifies the
existing inter path and adds no external production codec dependency.
All 23 tests in nine root AV1 suites pass offline. The 504 previous intra
streams remain byte-identical after sharing the fixture writer.

Nonzero motion, compound prediction, other mixed block geometries, multitile
maps and forced segment tools remain outside this matrix. It does not prove
complete AV1 or overall codec conformance.

## AV1 mixed-block nonzero single-reference motion

384 additional owned 32x32 streams exercise NEWMV on the first 16x16 block,
with horizontal displacements of +/-1, +/-1/2 and +/-1/4 luma pixels. Other
blocks use identity GLOBALMV. Logical LAST/ALTREF select physical slot 7;
qindices 1/64, two mixed maps and their complements, largest/SELECT
transforms, both CDF modes and positive/negative residuals are covered.
The hidden references use signed DC magnitude 14, instead of 1, so rounding
cannot erase the movement signal. During generation every first shown frame
is compared with an independent zero-motion control and must differ.

Native acceptance checks the exact saved independent Y/Cb/Cr output, maps,
WebM timestamps, rewind and seek. The fixture writer now supports owned
DC magnitudes 1..14 through COEFF_BASE_EOB/COEFF_BR symbols. The previous
504 intra streams remain byte-identical. Ordinary tests use no generator,
network or external decoder. Removing only horizontal motion from native
prediction causes a pixel mismatch; restoration passes both acceptance tests.
All six tests across the three mixed-block suites pass offline.

This is horizontal single-reference spatial prediction qualification. Vertical
motion, broader motion-vector classes/precision, compound prediction and
other unsupported AV1 tools still require implementation or qualification.

## AV1 mixed-block vertical and diagonal motion

The owned fixture writer now encodes both NEWMV components and their
MV_JOINT symbols. `av1_mixed_axes` covers 768 vertical and opposite-sign
diagonal streams at +/-1, +/-1/2 and +/-1/4 luma pixels. Each vector has
64 combinations of LAST/ALTREF routing, qindices 1/64, two mixed maps,
largest/SELECT transforms, CDF adaptation and signed residuals.

The first 16x16 block moves against signed magnitude-14 DC references;
other blocks use identity GLOBALMV. Every generated first shown frame must
differ from a separately decoded stationary control. Pixel references are
saved independent libaom output. Generation is separate from ordinary offline
acceptance, which reads only owned streams and checks Y/Cb/Cr pixels, segment
maps, WebM clock, rewind and seek.

This expands Class-0 quarter/half/full-pixel single-reference qualification.
Eight tests across the four mixed-block suites pass offline in the verification
checkout, whose AV1 decoder matches main. Previous 504 intra and 384
horizontal streams remain byte-identical. Removing vertical motion causes
a pixel mismatch on y-8-x0; removing horizontal motion causes a mismatch
on y-8-x8. These checks distinguish both axes of diagonal prediction.

It does not establish other motion classes, eighth-pixel precision, compound
prediction, OBMC, inter-intra or complete AV1 conformance.

## AV1 forced segment reference selection

SEG_LVL_REF_FRAME is now applied by native block decoding. When a frame's
segmentation features require pre-skip IDs, the segment is read before skip.
A forced reference suppresses skip-mode/is_inter/reference-selection symbols
and compound reference selection, then uses the normal single-reference
mode and reconstruction paths. Forced INTRA selects the existing intra path;
forced skip/global segmentation tools remain explicitly unsupported.

256 owned 32x32 streams cover forced INTRA and all seven inter reference roles, two mixed
segment maps and their complements, qindices 1/64, largest/SELECT, CDF
adaptation off/on and signed DC residuals. Two contrasting hidden references
distinguish physical reference selection. Independent libaom accepts every
stream and supplies saved pixel output. The old native decoder specifically
refuses av1-forced-reference-ref1-q1-mask2-tx0-adapt0-dc1.obu with
“AV1 active segmentation features not implemented”; acceptance with the fix
checks every displayed pixel, segment map, WebM timestamps, rewind and seek.
Ordinary tests use saved fixtures and require no external codec or network.
35 AV1 unit tests and 17 tests in seven root suites pass offline in the
verification checkout, whose decoder matches main. The 504 existing intra
and 672 inter streams remain byte-identical with the extended generator.

The acceptance matrix directly qualifies all eight forced-reference values.
Skipped blocks with temporal maps, inherited forced tables and other
block geometries still need additional qualification. SEG_LVL_SKIP/GLOBALMV,
compound prediction tools, OBMC/inter-intra and remaining codec gaps are
not established by this change.

## AV1 forced skip and global segment tools

Native block decoding now applies SEG_LVL_SKIP and SEG_LVL_GLOBALMV using
pre-skip segment IDs. Forced skip omits the skip symbol and residuals. Both
tools suppress skip-mode and inter mode/reference symbols, select LAST and
GLOBALMV unless a segment reference overrides LAST, and disable compound
selection. Existing forced-reference priority is preserved, including INTRA.
Otherwise GLOBALMV forces is_inter; SKIP alone still reads is_inter.
The blanket active-segmentation-feature refusal has been removed.

160 owned streams cover skip/global/both on both segments and opposite
skip/global assignments across mixed lossless/lossy segments. Qindices 1/64,
two maps and their complements, largest/SELECT, CDF adaptation and signed
residuals are included. Independent libaom accepts every stream and provides
saved pixels. The old native decoder specifically refuses the first valid
forced-skip input with “AV1 active segmentation features not implemented”.
Acceptance compares every Y/Cb/Cr sample, segment map, WebM timestamp, rewind
and seek. Ordinary tests read saved fixtures without external codec tools.

The writer's SELECT context uses skipped neighbors' block dimensions, even
when their segment is lossless; the independent oracle checks this mixed
case before fixtures are accepted. 35 AV1 unit tests and 19 tests in eight
root suites pass offline in the verification checkout, whose decoder matches
main. The 504 prior intra streams remain byte-identical.
GLOBALMV uses the identity global model
in this matrix. Nonidentity global motion, inherited forced tables, forced
INTRA combined with skip/global, other geometries, OBMC/inter-intra and
remaining codec tools are not qualified by these streams.

## AV1 nonidentity global motion qualification

Native parsing now accepts translation, rotation/zoom and affine global models,
including differential parameters inherited from the primary reference. Prediction
uses projected motion vectors and the owned affine warp implementation; invalid
shear falls back to projected motion. Translation also consumes switchable filter
symbols, fixing the reproduced truncated-entropy failure.

`tests/av1_global_motion.rs` checks 768 owned global-motion streams and four
GLOBALMV/NEARMV streams against saved independent pixels. Coverage includes header
parameters, mixed segment maps, reset, WebM timestamps, rewind and sync seek.
Generators are separate from tests; ordinary offline tests use no FFmpeg, libav
or libaom. Optional libaom is only a generator-side range writer and pixel oracle.

This matrix covers 32x32 8-bit pictures, mixed 16x16 segments and LAST/ALTREF
routing. It does not establish full AV1 conformance: high-precision motion,
scaled references, OBMC, inter-intra and general compound profiles remain outside
this qualification. Earlier nonidentity-global-motion limitations above are
superseded only within this documented scope.

## AV1 inter-intra prediction and wedge masks

The former `AV1 inter-intra blending not implemented` refusal is reproduced
by an owned 32x32 stream and replaced with native prediction. DC, vertical,
horizontal and smooth intra predictors blend with the inter predictor using
the normative mode weights. All 16 wedge masks are constructed from the master
profiles and shape codebook; 4:2:0 chroma uses rounded four-sample mask averaging.
Inter-intra suppresses overlapped/local-warp mode syntax, and size-group contexts
follow the block-size table rather than the shorter dimension alone.

`generate_av1_interintra_samples.py` writes 1152 synthetic OBU/WebM/YUV triples.
The matrix covers all four intra modes and all 16 wedge indices, LAST/ALTREF,
contrasting hidden references, mixed lossless/lossy segmentation, adaptive CDFs
and signed residuals. `tests/av1_interintra.rs` compares every displayed pixel
against saved oracle output, resets the decoder and checks WebM timestamps,
rewind and sync seek. Generation stays separate from ordinary offline tests;
production and tests do not load FFmpeg/libav/libaom. Optional libaom is used
only during generation for range writing and independent oracle pixels.

The pixel matrix qualifies 8-bit 16x16 inter-intra blocks in 32x32 pictures.
Rectangular block masks and other bit depths use the native implementation but
are not established by this matrix. OBMC, general masked inter-inter compound
and scaled-reference prediction remain separate unresolved AV1 gaps.

Validation: all 23 regression tests in ten AV1 suites and all 35 AV1 core
tests pass offline. The integration executable has no libav, FFmpeg or libaom
dynamic linkage. Existing sequence and entropy generator defaults remain
byte-identical in the checked compatibility cases.

## AV1 masked compound prediction

The owned reproducer for `AV1 masked compound prediction not implemented`
now decodes through native reconstruction. Two inter predictors blend using all
16 wedge indices with either sign, or using a difference-weighted mask with
either inversion. Difference weights are computed before clipping predictors,
with bit-depth/post-round normalization; chroma reuses the rounded subsampled
luma mask. Neighbor compound-group state contributes to later CDF contexts,
and joint distance-weight syntax is read only for the unmasked group.

`generate_av1_masked_compound_samples.py` produces 1056 owned OBU/WebM/YUV
triples. Coverage includes two distinct physical references selected as the
unidirectional LAST/LAST2 pair, all-masked and alternating masked/average
blocks, adaptive CDFs, mixed lossless/lossy segments, signed residuals and
strong hidden-reference contrast. The stronger difference cases exercise
spatially varying weights instead of only the constant base weight.
`tests/av1_masked_compound.rs` checks saved independent pixels, all segment maps,
decoder reset, displayed WebM timestamps, rewind and exact pixels after seek.
Ordinary tests require no encoder, external decoder, FFmpeg/libav or network.
The generator optionally uses libaom only as range writer and pixel oracle.

This matrix establishes 8-bit 16x16 blocks in 32x32 pictures and LAST/LAST2
routing. High-depth and rectangular compound blocks, alternate reference pairs
and interaction with global/local affine models still need further fixture
qualification. OBMC and scaled-reference prediction remain unresolved gaps.

Validation: 24 regression tests in eleven AV1 suites and 35 AV1 core tests
pass offline. A controlled fixed-weight counterfactual fails the strong-contrast
fixture with a pixel mismatch; restoring the difference-weighted implementation
passes. The integration executable links no FFmpeg, libav or libaom. Checked
legacy sequence/frame/entropy generator defaults remain byte-identical.

## AV1 overlapped motion compensation (OBMC)

The owned stream previously refused with `AV1 overlapped motion compensation
not implemented` now reconstructs natively. Above-neighbor blending precedes
left-neighbor blending, with normative masks, candidate stepping and neighbor
limits. Each overlap uses the neighbor's first reference, stored vector and
interpolation filters; neighboring warp and compound blends are excluded.
Tile boundaries and the small-plane restriction on the above pass are retained.

`generate_av1_obmc_samples.py` writes 2048 synthetic OBU/WebM/YUV triples.
Coverage includes left-only, above-only and both-pass blocks, two segment
reference assignments, four segment maps, adaptive CDFs, selected transform
sizes, signed residuals and contrasting hidden references, with zero and fractional global translation
vectors. Quarter- and half-pixel motion exercises overlap interpolation. Additional maps
make the top and left candidates use different references, so pass ordering
changes the output. `tests/av1_obmc.rs` checks saved independent pixels, segment
maps, decoder reset, WebM timestamps, rewind and exact pixels after sync seek.
Ordinary tests need no FFmpeg/libav, external decoder, encoder or network.
Optional libaom is limited to generator-side range writing and pixel oracle.

This matrix establishes 8-bit 16x16 blocks in 32x32 pictures, with 8-pixel luma
and 4-pixel chroma overlaps, regular interpolation and LAST/ALTREF routing. Additional block sizes,
high depth, small chroma planes, tile boundaries, compound/local-warp neighbors
and multiple candidates still need wider fixture qualification. Scaled reference
prediction remains an explicit unresolved AV1 gap.

Validation: 25 regression tests across twelve AV1 suites and all 35 AV1 core
tests pass offline, including the final expanded 2048-stream matrix. Controlled
counterfactuals disabling OBMC or reversing the above/left passes each fail with
a pixel mismatch; restoring the canonical implementation passes. Checked legacy
generator defaults remain byte-identical.

## AV1 prediction from scaled references

The owned stream formerly refused with `AV1 scaled reference prediction not
implemented` now reconstructs natively. Reference/current ratios use 14-bit
precision; signed sample-center mapping, phase rounding and independent X/Y
steps use 10-bit coordinates. Horizontal/vertical filters vary their phases
per output sample, with a correctly sized intermediate buffer and reference
border clipping. Unscaled prediction retains its existing path. Local-warp
mode syntax is gated by the computed reference scale, and scaled prediction
uses the motion-vector path rather than the affine warp kernel. Ratios outside
the normative reference/current size limits fail as invalid input.

`generate_av1_scaled_reference_samples.py` writes 768 owned OBU/WebM/YUV
triples and one invalid-ratio OBU. Twelve size pairs cover enlargement,
reduction, crossed horizontal/vertical scaling and odd reference dimensions.
The matrix varies LAST/ALTREF, all four fixed interpolation filters, zero and
fractional translation, adaptive CDFs and segment maps. The generator also
handles hidden 16x16/rectangular references and explicit frame-size overrides;
legacy defaults remain byte-identical in the checked compatibility cases.
`tests/av1_scaled_reference.rs` compares every displayed sample, reference and
output dimensions, segment maps, decoder reset, WebM dimensions/timestamps,
rewind and exact pixels after seek. The invalid-ratio test is a refusal test,
separate from the successful playback acceptance test. Ordinary offline tests
need no FFmpeg/libav, external decoder, encoder or network; optional libaom
is limited to fixture-generation range writing and independent pixel oracle.

This matrix establishes 8-bit single-reference prediction in 16x16/32x32 and
rectangular pictures. High depth, larger pictures, more extreme valid ratios
and scaled references combined with compound/OBMC/inter-intra tools still
need additional fixture qualification. This does not establish full AV1
conformance or close the remaining codec gaps.

Validation: 27 tests across 13 AV1 integration suites and 35 AV1 core tests passed offline, including the scaled-reference pixel, map, dimension, rewind, timestamp and seek acceptance tests.

## Scaled AV1 masked compound qualification

The scaled-reference implementation now also has independent pixel acceptance
coverage for two-reference masked compound prediction. The 544 owned fixtures
exercise all 16 wedge indices, both signs, difference masks, adaptive tile CDFs,
four interpolation filters, opposite synthetic residuals and four configurations
of differently sized LAST/ALTREF references. Sizes include odd dimensions,
rectangular axes, upscale, downscale, and a scaled/unscaled reference combination.
The two references stay distinct: shown frames refresh no reference slots.

`tests/av1_scaled_compound.rs` checks all hidden/shown dimensions, segment maps,
all displayed Y/Cb/Cr pixels against saved independent oracle output, decoder
reset, WebM rewind, timestamps and seek. All 544 cases passed offline. All 1,632
fixture hashes were verified. Deliberately replacing the sampling step with unity
or adding one filter phase made the pixel test fail; the restored decoder passed.
The optional generator uses libaom only to write entropy and save reference pixels;
ordinary tests read checked-in files and need no FFmpeg, libav, libaom or network.

This extends the preceding single-reference qualification; it does not establish
full AV1 conformance. Scaled OBMC/inter-intra, distance-weighted compound,
higher depths, chroma formats and extreme valid size ratios still need coverage.

## Scaled AV1 inter-intra qualification

The native scaled-reference predictor is independently qualified with inter-intra
blending in 1,632 owned streams. The matrix covers DC, vertical, horizontal and
smooth modes; all 16 wedge masks; LAST and ALTREF logical references; 12 pairs of
reference/current dimensions including odd dimensions, downscale, upscale and
unequal horizontal/vertical factors. Interpolation filters and adaptive CDFs vary
across the matrix. Hidden reference pixels have a synthetic spatial pattern.
Shown frames refresh no references, preserving the intended size difference.

`tests/av1_scaled_interintra.rs` compares all displayed Y/Cb/Cr pixels with saved
independent oracle output and checks sizes, segment maps, decoder reset, WebM
rewind, timestamps and seek. All 1,632 cases passed offline, and all 4,896 hashes
were verified. Disabling inter-intra blending in the isolated verification copy
caused a pixel mismatch; restored source is rechecked before qualification.
Fixtures are generated separately with optional libaom entropy/oracle helpers;
ordinary tests require no FFmpeg, libav, libaom or network and contain no private
media. This extends the preceding scaled-reference coverage; scaled OBMC,
distance-weighted compound, higher depths/chroma and full AV1 conformance remain
unproven. The explicit film-grain, temporal motion-field and separate frame-header
limitations are still implementation gaps, not passing playback acceptance.

## Native AV1 separate frame-header and tile-group OBUs

The decoder now accepts coded OBU_FRAME_HEADER followed by ordered OBU_TILE_GROUP
units, including groups delivered in separate decode calls. It parses the header's
trailing-one bit and zero padding separately from OBU_FRAME byte alignment.
Header/CDF/reference state is published only after all tiles validate. Compatible
OBU_REDUNDANT_FRAME_HEADER copies are accepted, including extra zero padding;
layer mismatches, different header contents, missing/repeated/out-of-order groups
and a new header or delimiter before frame completion are specific errors.
Pending compressed data shares the decoder budget with retained reference pictures.
Reset drops pending state. A configuration record cannot hide an unfinished coded
frame. `Decoder::finish()` validates transport EOF, and native MP4/WebM readers
call it so an unfinished frame cannot silently become successful EOF.

The owned generator `scripts/generate_av1_separate_frame_samples.py` saves 96
scaled-reference/inter streams (12 size pairs, LAST/ALTREF, fixed/adaptive CDFs,
with/without redundant headers) and four lossless synthetic tile layouts with
2, 4 or 8 groups covering 4 or 8 tiles. Ten invalid streams exercise exact parser
failures; four partially delivered tile layouts reproduce incomplete EOF in raw
OBU, WebM and MP4. All fixture parameters and pixels are owned synthetic data.
No private source media or codec parameter sets are used.

`tests/av1_separate_frame.rs` checks exact oracle Y/Cb/Cr pixels, segment maps,
dimensions, publication at packet boundaries, reset, WebM/MP4 clocks, rewind,
seek, specific malformed-input failures, redundant-header padding, pending-byte
limits, configuration refusal and incomplete EOF. The pre-fix acceptance test
reproduced `AV1 separate frame header/tile groups not implemented`, rather than
an unrelated entropy error. Ordinary tests use only saved data and need no
FFmpeg, libav, libaom or network. Optional libaom tools are generator/reference
helpers; the generator's input images are synthesized internally. The generator
also preserves existing key/inter-header and WebM bytes for checked defaults.

This removes the separate-header refusal, not the remaining AV1 temporal-motion,
film-grain, palette, intrabc, super-resolution, restoration/profile/chroma and
layering gaps. Full codec conformance remains unproven.

Final validation: all 47 tests across 21 checked-in AV1 integration suites and 35 AV1 core tests passed offline on the final source. The six new tests passed; all 326 fixture/container hashes were verified. The regression executable has no FFmpeg/libav/libaom linkage. Verification and canonical AV1/MP4/WebM sources match.

## Native AV1 palette prediction

The native intra decoder now reads Y/UV palettes (2–8 colors), merges and deduplicates
eligible above/left palette caches, decodes palette color deltas and signed/modular
V deltas, builds diagonal color-index maps with normative context ordering and
extends map edges before predicting visible samples. Palette prediction replaces
ordinary intra prediction while retaining transform/residual reconstruction.
Filter-intra syntax is omitted when a Y palette is present. Neighbor palette state
is stored with the block grid; image accounting includes the added state.
`Picture::palette_counts` and `palette_cache_hits` expose actual coded use rather
than inferring palette coverage from source images.

The owned generator saves 168 streams: 8/10/12-bit 4:2:0, all palette sizes 2–8,
constant/varied chroma, matching/opposite U/V patterns, 64x64 and odd 125x117
sizes. Saved pixels agree byte for byte between independent libaom and dav1d
references; normal tests require neither oracle, FFmpeg/libav nor network.
All ordinary sources are synthesized internally, including palette parameters.
The fixture manifest records original source hashes separately from decoded hashes:
18 opposite-chroma cases produce reconstructed samples differing from the source
image with these generator settings, despite lossless encoding requested. Both
reference decoders and native FVid agree on the coded reconstruction. These cases
are decoder acceptance, not a claim of exact source-preserving encoding.

`tests/av1_palette.rs` checks visible sample cropping, all sample bits, sizes,
actual palette sizes in both plane groups for every depth, cached-color use,
reset/finish, WebM raw depth, timestamps, rewind and seek. The pre-fix stream
reproduced the exact native palette refusal. Deliberately replacing palette map
lookup by color zero or dropping the V-delta sign caused pixel mismatches; the
restored decoder passed. The WebM fixture wrapper now recognizes implicit visibility
in reduced headers. Saved existing wrappers remained identical for checked cases.

This qualifies the saved lossless-coded palette tools and 4:2:0 profiles; lossy
palette/residual combinations, further layouts and full AV1 conformance are still
unproven. Temporal motion fields, film grain, intrabc/super-resolution, restoration,
quantization matrices and additional layering/chroma behavior remain codec gaps.

Final validation: all 48 tests in 22 checked-in AV1 integration suites and 35 AV1 core tests passed offline. The 168-case palette acceptance also passed in the canonical checkout. All 504 fixture hashes were verified. The palette regression executable has no FFmpeg/libav/libaom/dav1d linkage. All canonical AV1 sources match the verification snapshot after restoration of counterfactual mutations.

## Lossy AV1 palette qualification in progress

The owned lossy generator saves 339 single-frame 4:2:0 streams and matching
WebM wrappers and pixel goldens. The matrix spans 8/10/12 bits, source color
counts 2–8, constant/varied chroma, both U/V directions, and even/odd sizes.
336 cases use small source perturbations at quality 8/32; three additional
gradient cases at quality 48 exercise palette residuals and active loop filtering.
The gradient noise is selected per depth; enabling the encoder filter alone did
not guarantee nonzero coded filter levels. Saved libaom and dav1d pixels agree.

`tests/av1_lossy_palette.rs` checks decoded pixels, reset/finish, WebM timestamps,
rewind/seek, actual palette sizes 2–8 in both plane groups at every depth,
nonzero palette residuals, and active loop-filter levels. Each gradient case
individually requires nonzero filtering and luma palette residuals.
`Picture::palette_residual_blocks` counts transform blocks with actual nonzero
dequantized residuals in a palette plane group. Ordinary tests use only owned
saved fixtures, with no encoder, oracle, FFmpeg, or network invocation.

The 339-case baseline acceptance passed, including coded coverage assertions.
Omitting loop filtering in the verification copy caused a pixel comparison
failure on the owned filtered stream; the source was restored and matched
the canonical source. All 1017 saved file hashes were verified. The test
executable has no FFmpeg/libav/libaom/dav1d linkage. The final strengthened 339-case acceptance and the existing 168-case lossless
palette acceptance both passed offline after restoration. This does not claim full AV1 conformance or coverage of the
remaining codec tools listed above.

## AV1 delta-loop-filter owned reproduction

Six owned 8/10/12-bit streams at quality 32/48 now reproduce the native
`delta_lf` refusal. The generator forces 64x64 superblocks; auto superblock
selection had omitted delta-LF syntax in one high-depth gradient stream, so
that earlier encoding did not qualify as a reproducer. Both independent
reference decoders agree on saved pixels. All 18 payload/container/pixel
hashes were verified. The offline reproduction test independently parses
each header and requires coded delta-LF before checking the specific native
refusal. This is a passing refusal test, not a playback acceptance test.
This refusal was subsequently replaced by native pixel acceptance below.

## Native AV1 delta-loop-filter reconstruction in progress

Native reconstruction now reads signed scalar or per-direction/plane delta-LF
symbols, extends values with the normative escape syntax, accumulates/clips
deltas, resets tile state, and records the current deltas in each intra/inter
block. Filter strength clamps the delta-adjusted base before segmentation and
reference/mode adjustments. The CDF generator also emits four independent
multi-delta probability contexts, preserving all previous table IDs.

The six owned 8/10/12-bit streams now match independent saved pixels. The
12-bit quality-48 case initially exposed a second bug: chroma-from-luma at
the bottom frame edge used clipped MI-grid luma extents instead of the fully
reconstructed transform extent. Reconstruction now retains complete edge
transforms in padded internal storage while using logical MI boundaries for
intra neighbor availability. Picture/transform grids are cropped before the
existing filter pipeline. The memory estimate includes added storage.

The old refusal assertion has been replaced by pixel acceptance; reset/finish
and WebM replay/rewind/seek are checked. All 50 tests across 24 prior/current AV1 integration suites and 35 AV1
core tests passed offline. Scalar nonzero deltas are observed in the six encoder fixtures. An additional
64 directly synthesized streams cover scalar/multi contexts, all four
resolutions, positive/negative/zero and escaped/clipped values, and adaptive
CDFs. Both independent oracles agree; the strengthened nonzero residual
produces six distinct golden images, unlike the initial weak residual.
Native pixel, reset/finish and WebM replay/rewind/seek acceptance passed. All
192 added hashes were verified. Ignoring decoded deltas when choosing filter strength in the verification
copy caused a pixel mismatch on the owned negative-delta stream. The source
was restored and matched canonical sources; focused restored checks for all 6 encoder and 64 directly owned streams
passed offline. Tile reset and multi-superblock accumulation remain to be qualified. This implementation is not yet a claim of complete AV1 support.

## Delta-LF multi-superblock/tile qualification in progress

Six owned 256x128 8/10/12-bit streams with eight 64x64 superblocks and either
one or four tiles match both independent pixel oracles and native FVid.
However, removing the tile-state reset still passed: diagnostics showed
nonzero delta-LF state but zero base filter levels throughout the original
matrix. The frame-level filter bypass makes these insufficient to qualify
state effects on pixels. A periodic palette/gradient pattern also accepts,
but acceptance now explicitly requires nonzero base levels before claiming
state qualification. The owned test/generator are unfinished; they must
produce active filtering and detect both tile carry and per-SB reset mutants
before the gap is declared closed.

## Active delta-LF accumulation and tile reset qualification

The final owned generator saves seven 256x128 8/10/12-bit streams with eight
64x64 superblocks, one/four tiles, and active base loop-filter levels. It uses
a periodic palette/gradient pattern with quality 48 for 8 bits and 40 for
10/12 bits, plus a reversed-period 12-bit case whose final superblock retains
a nonzero delta at the tile boundary. Both independent reference decoders
agree on saved pixels; all 21 new hashes were verified.

The earlier zero-filter cases were insufficient even though pixels matched.
The test now independently requires coded delta-LF, nonzero base filter
levels and the expected actual tile count, then checks pixels, reset/finish,
WebM replay, rewind and pixel-exact seek. The final seven-case baseline passed.
Resetting delta-LF at every superblock caused an owned 12-bit pixel mismatch;
retaining delta-LF across tiles caused a pixel mismatch on the reversed-period
case. Each controlled mutation was restored; canonical and verification
production sources match. The focused restored run across all 77 delta-LF fixtures passed offline. This qualifies the saved accumulation/reset cases,
not full AV1 conformance or its remaining unsupported tools.

## Native AV1 quantization matrices in progress

Native coefficient reconstruction now applies the normative per-coefficient
quantizer weight with rounding before multiplying the decoded coefficient.
Lossless segments, level 15 and identity/one-dimensional transform types
bypass matrix adjustment. Large transform shapes use the specified 32-sized
coefficient layouts and matrix offsets. The saved matrix data contains 100320
bytes generated from the AV1 specification tables; source/data SHA-256 values
are embedded in the private Rust module. No runtime dependency is added.

The owned encoder generator saves 48 single-frame 4:2:0 streams covering all
16 coded levels at 8/10/12 bits with nonzero residuals, palette disabled and
loop filtering disabled to isolate reconstruction. Both independent libaom
and dav1d decoders agree; all 144 stream/pixel/container hashes were verified.
The pre-fix test independently parsed the intended matrix levels and
reproduced the precise native refusal on every stream. The refusal was then
replaced with pixel acceptance; its initial 48-case run passed. Ignoring the
weights in the verification copy caused a pixel mismatch on the level-0
owned stream, and the source was restored to match canonical code.

Final reset/finish and WebM replay/rewind/seek acceptance passed for all 48
streams; all 35 core AV1 tests also passed. The full integration regression passed: 53 tests across 27 AV1 suites. Inter frames, independently selected plane levels,
lossless/identity bypass and rectangular transform combinations need
additional owned qualification; the current streams do not prove full AV1
conformance or the remaining codec tools.

An additional 48 owned 64x64 streams fix partition sizes 4/8/16/64 with
largest-transform selection and levels 0/7/14/15 across all three depths.
High-frequency perturbations ensure small transforms contain AC residuals.
Both independent references agree. Temporary reconstruction diagnostics
confirmed actual weighted nonzero AC coefficients at square sizes 4, 8, 16,
32 and 64, including chroma and the normative capped layout for 64x64.
The diagnostics were removed and sources matched canonical code. All 96
matrix streams passed restored pixel/reset/WebM replay/rewind/seek checks;
the extra block-suite test passed separately alongside the full 53-test run.

### AV1 inter-frame quantization matrix qualification

Nine additional owned 64x64 three-frame streams cover depths 8/10/12 and
matrix levels 0/7/15. The test parses every frame header with refreshed
references and asserts the actual frame types [KEY, INTER, INTER] and all
three coded plane levels. Independent libaom and dav1d references agree
on all 27 frames; all 27 fixture/container/reference hashes were verified.
Offline FVid tests match every pixel, reset/finish, WebM timestamps, rewind
and seek back to the sync frame from the third-frame timestamp.

A verification-only mutant disabled weighting exclusively for INTER frames.
It failed pixel acceptance on the owned 8-bit level-0 stream, proving the
test exercises matrix-dependent inter reconstruction rather than only the
key frame. The original source was restored and matched canonical code.
All three matrix suites passed together after restoration (105 streams,
123 decoded frames per replay). The executable links neither FFmpeg/libav
nor either reference decoder; generator execution remains separate. The
optional dav1d helper now accepts a frame count, and its existing default
single-frame invocation still matches the original saved reference.

Separate plane levels, lossless/identity bypass and rectangular transform
combinations still require owned qualification. AV1 restoration and other
remaining codec tools are not claimed complete.

### AV1 independent Y/U/V quantization matrix qualification

Nine owned one-frame 64x64 streams cover depths 8/10/12 with independent
plane levels [0,7,14], [14,0,7] and [7,14,0]. Both generation-only libaom
and dav1d references agree on every pixel, and all 27 hashes were checked.
The ordinary offline test asserts the coded separate-UV flag and all three
actual matrix levels, then checks native pixels/reset/finish and WebM
replay/rewind/seek against saved independent references.

The installed encoder's per-plane control calls did not change its coded
[7,7,7] levels, so generation uses a deterministic owned baseline plus a
native optional header rewrite utility. It changes only matrix levels and
the required separate-UV flag, diff-UV signaling and byte alignment. Full
parsed sequence/frame equality checks protect every other field; tile
payload bytes are copied unchanged. No private media or parameters enter
the fixtures, and ordinary tests do not execute the generator.

Two verification-only mutants replaced chroma levels with Y, and replaced
V's level with U respectively. Both failed pixel acceptance on the owned
8-bit [0,7,14] stream. Sources were restored and matched canonical code.
Lossless/identity bypass and rectangular matrix transform combinations
still require owned qualification; remaining codec gaps stay open.

Final restored-source run passed all four matrix suites together: 114 owned
streams and 132 decoded frames per replay. The distinct-plane test binary
links neither FFmpeg/libav nor either reference decoder.

### AV1 rectangular quantization matrix qualification

Fifty-four owned one-frame 64x64 streams cover depths 8/10/12, matrix
levels 0/7/15 and six oriented stripe/ramp patterns. Independent libaom
and dav1d pixels agree, and all 162 hashes were verified. Offline tests
assert coded levels and compare native pixels/reset/finish plus WebM
replay/rewind/seek. Generation remains separate from ordinary tests.

The first noisy patterns encoded only square transforms despite enabling
rectangular search, so they were replaced before qualification. Temporary
reconstruction diagnostics on the final fixtures confirmed weighted nonzero
AC coefficients at 4x8, 8x4, 8x16, 16x8, 16x32 and 32x16 for every depth,
and additionally 32x64 at 8 bits. This is coded use rather than inference
from encoder settings. Diagnostics were removed and sources matched
canonical code. A verification-only mutant ignored weights exclusively
when width differed from height; pixel acceptance failed on the owned
8-bit level-0 orientation-2 stream, then the source was restored.

This qualifies those exercised shapes, not every AV1 rectangle. Remaining
32x64 high-depth, 64x32 and 4:1 aspect-ratio shapes still need owned
qualification, as do lossless/identity bypass and other codec tools.

Final restored-source run passed all five matrix suites together: 168 owned
streams and 186 decoded frames per replay. The rectangle test executable
links neither FFmpeg/libav nor either external reference decoder.

### AV1 lossless quantization matrix bypass qualification

Nine owned one-frame streams cover 8/10/12 bits with coded levels
[0,7,14], [14,0,7] and [7,14,0] while all segments are lossless. The
ordinary offline test parses and asserts both the coded levels and all
lossless flags/base quantizer zero. Saved libaom and dav1d pixels equal
the exact generator arithmetic source; the test independently reconstructs
that source as well as checking native pixels/reset/finish and WebM
replay/rewind/seek. All 27 hashes were checked, including equality of
reference and source hashes.

The external encoder rejects lossless with enabled matrices. The optional
native rewrite tool therefore supports adding matrices to an owned lossless
baseline, verifying full parsed sequence/frame equality except the requested
matrix/UV signaling and alignment, while retaining original tile bytes.
The prior nine lossy plane fixtures and manifest regenerated byte-identically
after the utility change. Generation is never part of ordinary tests.

A verification-only mutant removed the per-segment lossless bypass guard.
It failed pixels on the owned 8-bit [0,7,14] stream, demonstrating that the
acceptance actually depends on ignoring weights in lossless reconstruction.
The source was restored and matched canonical code. This covers all-lossless
frames; mixed lossless/lossy segments and identity transforms still require
owned qualification, along with remaining rectangular shapes and codec tools.

The final restored-source run passed all six matrix suites together:
177 owned streams and 195 decoded frames per replay. The new executable
links neither FFmpeg/libav nor either external reference decoder.

### AV1 identity-transform quantization matrix bypass qualification

Eighty-one owned one-frame streams cover depths 8/10/12, qualities 4/12/32,
matrix levels 0/7/15 and three synthetic noise/checker/grid patterns. Both
independent libaom and dav1d references agree, and all 243 hashes were
verified. Offline acceptance asserts actual coded levels and native pixels
with reset/finish plus WebM replay/rewind/seek. Ordinary tests consume saved
fixtures without executing reference decoders or generation.

Temporary reconstruction diagnostics confirmed nonzero coefficients with
IDTX (type 9), V_DCT (10) and H_DCT (11) at every depth. The initial two
qualities did not exercise 12-bit IDTX, so quality 4 was added before that
coverage was claimed. Diagnostics were removed and canonical code restored.
This qualifies the actual identity and one-axis DCT cases observed; it does
not prove all one-axis ADST/FLIPADST types, mixed lossless/lossy segments,
remaining rectangular shapes, or the remaining codec tools.

Two verification-only mutants applied matrices to IDTX, and to V_DCT/H_DCT
respectively. Each failed pixel acceptance on owned 8-bit quality-4 level-0
streams (checker and noise patterns respectively). Both sources were restored
and matched canonical code, showing independent sensitivity to both bypasses.

Final restored-source run passed all seven matrix suites together:
258 owned streams and 276 decoded frames per replay. The identity test
executable links neither FFmpeg/libav nor either external reference decoder.

### AV1 loop restoration: owned reproduction and header remapping fix

Eighteen owned 192x128 synthetic ramp/noise one-frame streams cover depths
8/10/12, qualities 32/48/56 and two noise amplitudes. Generation-only
libaom and dav1d references agree on every frame; all 54 hashes were checked.
Four frames have restoration disabled in their frame headers and pass
independent pixel acceptance. Fourteen have active SGRPROJ and/or Wiener
planes and reproduce precisely `AV1 loop restoration not implemented`.
The passing active test is refusal/reproduction, not playback acceptance.

The fixture investigation exposed a native header parsing bug: `lr_type`
was stored without normative Remap_Lr_Type conversion. The parser now maps
bitstream [NONE, SWITCHABLE, WIENER, SGRPROJ] to semantic values [0,3,1,2].
The owned test asserts all 18 expected semantic plane triples, including
both SGRPROJ and Wiener. Its pre-fix run failed on the first coded SGRPROJ
plane with the specific semantic-value mismatch. No refusal is removed:
restoration entropy parsing and filtering still need implementation, after
which this active refusal expectation must become pixel acceptance.

Normative syntax: https://raw.githubusercontent.com/AOMediaCodec/av1-spec/master/06.bitstream.syntax.md
(`lr_params`, Remap_Lr_Type). No private media or parameter sets enter the
fixtures; ordinary tests use only saved streams/reference bytes.

Final verification passed all 35 core AV1 tests and 60 integration tests
across 34 AV1 suites. The restoration regression executable links neither
FFmpeg/libav nor either generation-only reference decoder. The active
restoration reproduction remains a known codec gap, not completed playback.

### AV1 restoration unit entropy parsing

Native decoding now reads restoration units before each superblock's
partition syntax. It uses adaptive Wiener/SGRPROJ/switchable CDFs, signed
reference-centered subexponential coefficients, chroma tap rules and SGR
radius-dependent projection parameters. Unit storage uses checked frame
dimensions, validated unit sizes and explicit accounting in the existing
image allocation budget. Reference coefficients reset for each tile;
duplicate/missing units are rejected. No decoder dependency was introduced.

All 14 active owned frames now reach validated tile entropy termination and
block reconstruction before the precise remaining refusal,
`AV1 loop restoration filtering not implemented`. The four inactive controls
still match independent reference pixels. This is entropy-stage acceptance
and filtering-stage refusal; it does not yet prove restoration pixels or
the numerical correctness of every retained coefficient. The final filter
pixel comparison is still required.

A verification-only mutant omitted unit reads. The owned first stream failed
with `invalid AV1 entropy trailing bits`, showing that reading these fields
is necessary to align the subsequent block stream. The source was restored.
The test's refusal expectation was updated to the filtering-stage message
to distinguish this progress from the previous early header refusal.
Initial single-unit fixture qualification includes Wiener and SGRPROJ.
The multi-unit extension below expands syntax coverage; coefficient
numerics and final filtering still require pixel acceptance.
Normative syntax: https://raw.githubusercontent.com/AOMediaCodec/av1-spec/master/06.bitstream.syntax.md
(`read_lr`, `read_lr_unit`, signed subexponential restoration syntax).

An additional 18 owned 384x640 frames use two entropy tiles and 128x128
superblocks. Seventeen active frames reach the filtering stage with valid
entropy termination; one inactive frame matches independent pixels.
Sixteen frames have active luma with a 2x3 unit grid at size 256. The
chroma-only active case uses size 128 and multiple chroma units. Two frames
select SWITCHABLE, so syntax acceptance now exercises all three frame
restoration modes. Both reference decoders agree and all 54 new hashes
were verified. Tests assert dimensions, tile count, superblock mode and
actual unit sizes; no reference process runs during ordinary tests.

Final verification passed 35 core AV1 tests and the 60-test full run across
34 integration suites. The additional multi-unit suite and original
restoration suite passed together after the final assertion changes, giving
61 unique integration tests across 35 suites. Final restoration filtering
acceptance and numerical coefficient validation are still outstanding.


### Native AV1 restoration filtering

The decoder now applies owned Wiener and SGRPROJ reconstruction after
deblocking and CDEF. Restoration stripes use the retained deblocked source
above and below their boundaries and the CDEF source inside each stripe.
Temporary source planes and filter scratch are included in the checked
decoder memory budget. Multi-unit playback explicitly tests refusal at
16 MiB and acceptance at 32 MiB.

The original 18-frame suite and the 18 multi-unit/two-tile frames now check
complete reference pixels, reset, WebM replay, rewind and seek instead of
expecting a filtering refusal. An additional owned 18-frame suite enables
CDEF and deblocking with restoration; libaom and dav1d agree on its reference
pixels. Its 54 manifest hashes were verified. Generation is separate from
ordinary offline tests and requires neither private media nor FFmpeg.

This qualifies the tested intra 4:2:0 cases at 8, 10 and 12 bits; it does not
claim complete AV1 profile/tool coverage, inter restoration qualification,
or measured 60 fps performance.

Final offline validation passed 35 core AV1 tests and 62 integration
tests across 36 AV1 suites. The restoration regression binary links
only system libraries; no FFmpeg/libav, libaom or dav1d linkage was present.


### Owned inter restoration qualification and inter-intra warp fix

An 18-stream set (three frames each, 192x128, 8/10/12-bit 4:2:0) exercises
restoration in actual inter frame headers. A matching 18-stream control set
disables restoration. Both were generated from owned moving gradients and
deterministic noise, with libaom/dav1d pixel agreement and 108 verified hashes.

Before the fix, seven parameter combinations failed native entropy decoding,
on the same frame with restoration enabled or disabled:
8-bit q48/orientation0 (frame 2), q56/orientation0 (frame 3);
10-bit q48/orientation0 (frame 2), q56/orientation0 (frame 3),
q56/orientation1 (frame 2); 12-bit q32/orientation0 (frame 2),
q48/orientation0 (frame 3). A separate 10-bit q32/orientation0 case differed
at byte 110558: native 240 versus reference 241 in both configurations.

An instrumented external reference build located the first entropy divergence
in motion mode selection: native selected a three-symbol warped-motion CDF
where the reference selected the two-symbol OBMC CDF. Native Block storage
had conflated a missing second reference with INTRA_FRAME in inter-intra
prediction. It now retains an explicit inter-intra flag and excludes these
neighbors from warp sample gathering, as required by AV1 find_warp_samples.
The external reference build is diagnostic only, never a runtime/test dependency.

All 36 streams now check full native pixels, reset, WebM replay, timestamps,
rewind and seek. The former refusal and mismatch expectations have been
removed, and full acceptance is enabled in ordinary offline tests. These
108 owned frames qualify the tested inter restoration configurations; other
AV1 profiles/tools and overall codec completion remain separate requirements.

Final validation after the warp candidate fix passed 35 core AV1 tests and
64 integration tests across all 37 AV1 suites, with no ignored acceptance
tests in that run. The new acceptance binary links only system libraries.


### AV1 odd-dimension restoration qualification

Eighteen owned 191x127 intra fixtures cover 8/10/12-bit 4:2:0 with
restoration, CDEF and deblocking. Their coded headers contain 13 active
restoration frames and five inactive controls; at least one active frame
combines nonzero CDEF strengths and loop filter levels. Native cropped
pixels match independent libaom/dav1d references, including the final odd
luma row/column and ceil-divided chroma planes.

The acceptance suite checks two native decode/reset cycles and WebM replay,
EOF, rewind, timestamps and seek. All 54 saved fixture hashes were verified.
A controlled mutation that clamps restoration samples to padded storage
instead of the actual coded extent fails on the first fixture's restored
pixels. The source was restored before final acceptance validation. These
fixtures are generated separately and ordinary tests require no FFmpeg or
reference decoder. Other dimensions and AV1 tools remain unqualified.


### AV1 super-resolution owned reproductions

Eighteen owned 192x128 output fixtures force super-resolution denominators
9, 12 and 16 at 8/10/12 bits, with two deterministic noise patterns each.
Their headers prove actual scaled coded width and exclude intrabc and
restoration to isolate this tool. libaom and dav1d agree on all reference
pixels; all 54 fixture hashes were verified. Before reconstruction was implemented, a default regression pinned
the native super-resolution refusal and reset behavior.

The initially ignored acceptance test required full 192x128 native pixels,
reset, WebM replay, timestamps, rewind and seek. That initial refusal stage
was a reproduction rather than supported playback; native reconstruction
and enabled acceptance are recorded in the following section. Generating these owned
fixtures is separate from offline tests and uses no FFmpeg.


### Native AV1 super-resolution reconstruction

Owned horizontal upscaling now uses the normative 64-phase, eight-tap
filter, signed initial phase, 14-bit step and clipped seven-bit rounding.
Output dimensions and planes retain the restored width. Source samples
are read from the coded MI extent, including edge padding. Additional
upscaled planes are included in checked memory admission. A 1 MiB decoder
budget refuses an owned 96-to-192 example; sufficient-budget acceptance
checks its pixels and playback.

Restoration unit storage uses the upscaled width, and tile/SB unit reads
project coded horizontal positions with the super-resolution denominator.
Both deblocked and CDEF sources are upscaled before restoration filtering.
Eighteen additional owned combined-tool fixtures agree between libaom and
dav1d; all 54 new hashes were verified. The original tool refusal is removed
and both 18-stream native/playback acceptance suites are enabled, covering
8/10/12-bit 4:2:0 and denominators 9, 12 and 16.

This verifies the owned single-frame configurations, not all inter reference,
segmentation inheritance, multi-tile or multi-unit super-resolution cases.
These combinations and other unsupported AV1 tools remain to be qualified.

Final offline validation passed 35 core AV1 tests and 67 integration tests
across all 39 AV1 suites, with no ignored acceptance tests. The super-resolution
acceptance binary links only system libraries.


### Inter-frame super-resolution with restoration qualification

Eighteen owned three-frame streams combine super-resolution denominators
9/12/16, 8/10/12-bit 4:2:0, CDEF and restoration. Header assertions prove
actual super-resolution in each coded frame, a key/inter/inter sequence
and active restoration on 35 of the 36 inter frames. All 54 decoded frames
match saved independent libaom/dav1d pixels. Native reset and WebM replay,
EOF, timestamps, rewind and seek are checked for every stream.

All 54 saved fixture hashes were verified. A controlled mutation that
disables reference scaling leaves the first frame intact but fails on byte
36867 (the second frame) of the first 8-bit denominator-9 stream. This
proves sensitivity to restored-reference prediction rather than just the
intra upscaler. The source was restored before final acceptance validation.

The fixture generator is separate from ordinary offline tests and needs no
FFmpeg. These fixed-denominator inter cases qualify the tested references;
changing denominators across frames, inherited segmentation maps, tiled
restoration units and other AV1 tools remain separate outstanding cases.


### Super-resolution with inherited segmentation maps

Eighteen owned three-frame streams combine denominators 9/12/16, 8/10/12-bit
4:2:0, inherited nonzero segment maps, CDEF and restoration. All 36 inter
headers explicitly inherit the primary reference map; native map equality is
checked separately from full pixels. All 54 frames match unmodified libaom
and independent dav1d. Reset, WebM replay, timestamps, EOF, rewind and seek
are checked for every stream.

Before the fix, the denominator-9 8-bit stream decoded its key frame correctly
but differed at byte 36864, the start of its first inter frame. FVid compared
upscaled picture dimensions with coded dimensions and discarded the inherited
map. Reference pictures now retain coded MI grid dimensions independently of
display size. Map inheritance compares MI dimensions as required by AV1 section
7.8, including matching grids whose exact pixel dimensions differ.

The separate generator uses a documented libaom v3.15.1 fixture-only patch:
keep variance AQ enabled under super-resolution, select LAST as primary
reference, and preserve the map on inter frames. Both independent reference
decoders remain unmodified. No external codec, FFmpeg or network is required
by ordinary acceptance tests. Changing denominators, temporal map updates,
multiple tiles and other AV1 profiles/tools remain separate qualification gaps.

Validation: all 35 fvid-codecs AV1 unit tests and 69 integration tests across
41 AV1 suites pass offline with no ignored acceptance tests or FFmpeg.


### Tiled and changing-denominator AV1 super-resolution

Two further owned suites each contain 18 three-frame streams across 8/10/12-bit
4:2:0 and two deterministic source patterns. The tiled suite uses odd 385x385
output, 128x128 superblocks, exactly two entropy tiles, denominators 9/12/16,
CDEF and restoration. Headers require active restoration on 34 inter frames
and multiple luma restoration units on 40 of the 54 frames. Full native pixels,
reset, WebM replay, timestamps, EOF, rewind and seek match saved independent
libaom/dav1d pixels for every stream.

A controlled mutation that projects restoration-unit boundaries with denominator
8 instead of the signaled denominator fails the first 8-bit denominator-9
fixture with truncated AV1 entropy data. The exact original source was restored
before the final acceptance run. This demonstrates sensitivity to the coded-to-
upscaled unit projection across tiles rather than only the final image filter.

The changing-denominator suite forces key/inter transitions 9->16, 16->9 and
8->12 at 192x128 output. Every header proves the requested denominator and
changed coded width; decoded coded-grid dimensions change while the display
extent remains constant. All 54 frames match both references, including reset,
WebM replay, timestamps, EOF, rewind and seek.

Fixture generation uses independent tools separately from offline tests and
requires no FFmpeg. These suites qualify the recorded fixed-size transitions
and two-tile layouts. Temporal reference motion fields, temporal segmentation
map updates, film grain, intrabc and additional profiles/chroma remain gaps;
these acceptance suites do not establish complete AV1 support.


### AV1 temporal segmentation with super-resolution

Temporal segmentation already has explicit owned map/pixel coverage in
`av1_segmentation_maps`. A further 18 three-frame streams now combine temporal
map prediction with super-resolution denominators 9/12/16, 8/10/12-bit 4:2:0,
CDEF and restoration. All 36 inter headers require enabled segmentation,
update_map and temporal_update, with LAST as primary reference. Key maps are
required to contain nonzero IDs. Full pixels for all 54 frames match unmodified
libaom and independent dav1d; reset, WebM replay, timestamps, EOF, rewind and
seek are checked for every stream.

The first generation attempt exposed an insufficient fixture: temporal_update
was signaled but all traced prediction symbols were false, so zeroing the
previous map did not affect pixels. The final fixture-only encoder patch uses
segment 1 and emits predicted-ID symbols when the previous map matches. Its
hash is pinned in the manifest. Both independent decoders remain unmodified.
A controlled native mutation that discards the previous coded-grid map now
leaves the key frame intact but differs at byte 36864 of the first 8-bit
Denominator-9 stream, the start of the first inter frame. The source is restored
before final acceptance validation. The initial insufficient assets were
replaced rather than presented as tool coverage.

No production codec change is required for these tested combinations; fixture
generation is separate from ordinary offline tests, with no FFmpeg or network.
Temporal reference motion fields, film grain, intrabc and additional chroma/
profiles remain separate outstanding AV1 work. These fixed-grid cases do not
establish temporal map behavior across changing coded grids or multiple tiles.

Final focused validation: six tests across temporal maps, inherited super-resolution
maps and temporal super-resolution maps pass offline without ignored tests.


### Native AV1 temporal reference motion fields

The native decoder now retains filtered forward-reference MVs on the coded
8x8 grid, projects them using saved frame distances, and inserts temporal
samples between nearest and farther spatial MV candidates. The implementation
covers source-frame priority, order-hint wrapping through signed distances,
reference-grid matching, projection rounding/clipping, bounded projected
positions, compound candidates and the temporal ZERO_MV context. Storage and
working allocations are included in decoder admission/retained memory.

Six owned six-frame 192x128 sequences at 8/10/12 bits use deterministic
translated gradients and noise. Before the fix, they specifically refused
with `AV1 temporal motion field not implemented`. Acceptance requires all
30 inter headers to enable reference MVs, retained nonzero motion samples,
full independent libaom/dav1d pixel parity for all 36 frames, reset, WebM replay,
timestamps, EOF, rewind and seek.

A controlled mutation that discards all projected field samples keeps the first
three frames intact but differs at byte 110592, the first pixel of the fourth
frame in the first 8-bit fixture. This proves use of projected temporal
candidates rather than only enabling the header flag. Source is restored
before final validation. Two internal tests cover signed projection, rounding,
clipping and bounded negative position offsets. Generation is separate from
ordinary offline tests, with no FFmpeg or network dependency.

These first sequences establish forward-reference temporal prediction at one
tile and fixed coded dimensions. Bidirectional references, wrapped order hints,
changing coded grids, tiled temporal sampling and super-resolution combinations
still require dedicated full-pixel qualification. Film grain, intrabc, mixed
sub-8x8 intra/inter chroma and additional profiles/chroma remain separate gaps.

Validation: all 37 AV1 fvid-codecs unit tests and 73 integration tests across
45 AV1 suites pass offline, with no ignored acceptance tests or FFmpeg.


### Temporal reference motion with super-resolution and restoration

Eighteen owned six-frame 192x128 sequences combine temporal reference motion
with super-resolution denominators 9/12/16, loop filtering, CDEF and restoration
at 8/10/12 bits and two translated source patterns. All 90 inter headers
enable reference MVs, while the coded grid is narrower than the display grid.
The native acceptance checks full independent libaom/dav1d pixel parity for
108 frames, retained nonzero motion, coded-grid storage, reset, WebM replay,
20 ms timestamps, EOF, rewind and seek. All 54 checked-in asset hashes match
the generation manifest. Ordinary execution is offline and requires no FFmpeg
or external decoder. This qualifies fixed-denominator forward-reference
combinations; bidirectional references, order-hint wrap, changing coded grids
and tiled temporal sampling still need dedicated qualification.


### Bidirectional temporal motion-field qualification

Six owned twelve-frame 192x128 sequences at 8/10/12 bits and two translated
source patterns use lagged encoding and alternate references. The test confirms
69 actual reference-MV inter headers and 78 future-reference slots using signed
order-hint distances. It decodes one OBU at a time, retains hidden references,
and compares all 72 displayed frames with matching independent libaom/dav1d
goldens. Reset, full WebM replay, displayed PTS intervals across hidden frames,
EOF, rewind and sync-seek replay are checked. The test never invokes external
codecs, FFmpeg or the network. Generation remains separate.

A controlled mutation removing only BWDREF/ALTREF2/ALTREF projection makes the
first 8-bit fixture fail native entropy trailing-bit validation. This proves
the streams require future-source temporal projection, not merely positive
reference distances. The production source was restored byte-for-byte before
final acceptance. This qualifies the owned single-tile, fixed-grid streams;
wrapped order hints, changing coded grids and tiled temporal sampling remain
unproven, along with film grain, intrabc and additional chroma/profiles.


### Native AV1 intra block copy

The native decoder now reads the intra-block-copy flag and its separate
displacement-vector CDF context, builds the spatial stack from previously
copied blocks, applies the normative default displacement, and copies from
the current coded picture. Integer luma displacement permits half-sample
chroma phases; these use bilinear interpolation with normative rounding.
Tile bounds, vector magnitude/precision, 256-pixel delay and wavefront
constraints are validated, and each source sample must already be decoded.
Transform syntax and coefficient contexts distinguish copied blocks from
ordinary intra blocks while retaining INTRA_FRAME as their reference role.
Existing admitted prediction/residual buffers are reused.

Twelve owned 384x192 single-frame streams at 8/10/12 bits, lossless/lossy and
two repeating texture patterns reproduced the specific former
`AV1 segmentation/intrabc/superres reconstruction not implemented` refusal.
Their enabled acceptance verifies actual copied blocks, independent libaom
and dav1d YUV parity, reset, WebM replay, EOF, rewind and seek. Replacing the
copy predictor with zeros differs at byte 320 in the first 8-bit lossless
fixture, proving use of copied pixels rather than only the header flag.
The source is restored before final validation.

Twelve additional 769x257 streams combine two tiles, superblock 64/128,
sub-8x8 copy blocks, all four chroma half-sample phases and nonzero Y/U/V copy
residuals, for every 8/10/12-bit lossless/lossy combination. They exposed two
real context gaps: chroma smooth-neighbour lookup must select the MI carrying
coded UV modes, and intra transform-size contexts must treat an intrabc
neighbour as inter-coded. The first issue also occurs with copying disabled:
12 owned control streams reproduce the original pixel difference at byte
197717 in the first 8-bit lossless stream, and pass with corrected parity
offsets. The second previously desynchronised the 10-bit lossy stream and
produced a spurious invalid displacement; correcting the CDF context restores
complete pixel parity without relaxing displacement validation.

All three fixture groups are generated separately from ordinary tests; their
108 OBU/WebM/YUV asset hashes are pinned in manifests. Tests need neither
external codecs nor FFmpeg/network access. This qualifies the owned 4:2:0
combinations; film grain, mixed sub-8x8 intra/inter chroma, other chroma/profiles
and layered/tile-list decoding remain separate gaps.

Validation: all 38 AV1 fvid-codecs unit tests and 78 integration tests across
50 AV1 suites pass in release mode with locked offline dependencies, no
FFmpeg and no ignored acceptance tests. The initial 12-stream acceptance and
all 12 extended copy cases also pass in debug mode.

### AV1 film grain parameter syntax (2026-10-08, initial syntax milestone)

The owned frame-header parser now reads the complete film-grain syntax instead
of refusing at `apply_grain`: piecewise scaling points, luma/chroma AR
coefficients through lag 3, chroma multipliers/offsets, overlap/range flags,
seed, and inter-frame parameter inheritance. Fixed arrays bound every signaled
count. Point counts/order and reference membership are validated. Inherited
parameters replace only the seed; reset parameters are inherited as reset.

Six owned 64x64 8/10/12-bit input patterns encoded with public libaom grain
presets 1/16 have identical grain-applied goldens from aomdec and dav1d. Generation
is separate (`scripts/generate_av1_film_grain_samples.py`); ordinary tests read
checked-in assets and do not launch external codecs. Header syntax acceptance
checks all six streams and every truncated header. Unit tests exercise maximum
point/AR counts, inheritance/new seed, invalid references, point ordering and
absent/reset branches.

At this initial syntax-only milestone, commit `006991e78`, playback still
refused at `AV1 film grain synthesis not implemented`. The synthesis milestone
below removes that gate and replaces the refusal with native pixel/playback
acceptance using the same goldens.

### Native AV1 reference film-grain synthesis (2026-10-08)

Owned safe Rust now implements the normative Gaussian/LFSR noise generation,
luma/chroma autoregression, piecewise intensity scaling including high-depth
interpolation, horizontal/vertical block overlap, luma-coupled chroma noise and
restricted/full-range clipping. The Gaussian constants are the specification's
normative table, not an external decoder implementation. Bounded two-stripe
scratch storage is reused across stripes. Admission includes that workspace,
the displayed picture copy and the retained-reference accounting margin.

Reference pictures remain ungrained. Visible frames receive a separate copy;
hidden frames stay in their reconstruction form and show-existing synthesizes
from the saved reconstruction/header. Parameter inheritance retains the seed
replacement semantics and records the actual reference slot for qualification.
The previous playback refusal test is replaced by pixel/playback acceptance.

Qualification assets comprise six original 64x64 single frames, 48 four-frame
149x85 streams covering all public libaom grain presets, 24 four-frame streams
with original authored parameter tables covering AR lag 0/1/2/3, and six five-
output-frame streams ending in show-existing. Both unmodified aomdec and dav1d
with grain enabled produce exactly identical goldens. In total these assets
contain 324 displayed frames at 8/10/12-bit 4:2:0. Offline acceptance compares
all samples through native decode/reset, WebM/EOF/rewind and seeks. The 48-stream
suite proves 27 actual inherited parameter headers, overlap/range enabled and
disabled, and chroma scaling from luma. The custom suite proves all four AR lags.

Zeroing synthesized noise fails the pixel regression at byte 0. A mutation that
stores displayed/grained pictures as references fails on byte 0 of the next
inter frame. These
are tests of actual synthesis and reference ownership, not only parsing flags.
Other chroma/profile/layered/tile-list support and broader combinations with
other AV1 tools remain separate qualification gaps; this is not full AV1
conformance.

Validation: 44 AV1 fvid-codecs unit tests and 83 integration tests across 52 AV1
suites pass in release with locked offline dependencies, no FFmpeg and zero
ignored tests. All 252 qualification asset hashes and all 2048 normative Gaussian
constants were verified. The grain test binary links only libSystem/libiconv,
not libav, libaom or dav1d. Both controlled mutations failed and production code
was restored before the final regression run.

### AV1 mixed intra/inter sub-8 chroma qualification (2026-10-08)

The previously listed mixed sub-8 chroma gap was an unqualified existing path,
not absent reconstruction. The prediction syntax's `SomeUseIntra` selection was
already implemented: any intra constituent selects the current inter block's
vector for the entire UV group; an all-inter group uses its constituent luma
blocks' inter predictions. The invalid-reference invariant guard misleadingly used an `Unsupported` error
naming the whole feature as missing. It now reports inconsistent reference state
as invalid input instead, and the source comment states the normative selection.

Nine owned four-frame 128x96 streams at 8/10/12 bits now qualify actual decoded
mixed groups. A first periodic Q32 pattern had zero mixed groups despite matching
pixels; it was rejected as insufficient evidence. Original deterministic hashed
4-sample patches at Q16, surrounded by predictable moving texture, produce 458
mixed chroma groups: 423 with 4x4 luma, 23 with 4xN and 12 with Nx4. Every depth
and rectangular orientation must contain both rectangular mixed shapes, and each
stream must contain a 4x4 mixed group and all-inter sub-8 groups. Intra counts
exclude undecoded padding and intrabc. The same streams exercise 1003 all-inter
sub-8 groups. These are decoded-block counts, not only encoder permission flags.

Both unmodified aomdec and dav1d give identical goldens for all 36 frames. Native
acceptance compares every sample after decoder reset, through WebM/EOF/rewind,
with presentation intervals and seek replay. No external decoder, FFmpeg or
network is used during ordinary tests. Fixture generation remains separate in
`scripts/generate_av1_mixed_sub8_samples.py`.

Disabling the intra-neighbor selection fails on the specific inconsistent
inter-chroma reference. Collapsing all-inter groups to the current block's vector
fails at frame 1 byte 12296 (U plane sample 8) of the first 8-bit stream. Both
mutations are restored before the final regression run. Broader chroma/profile
and layer/tile-list tools remain separate gaps; this qualification does not imply
full AV1 conformance.

Validation: all 44 AV1 fvid-codecs unit tests and 84 integration tests across
53 AV1 suites pass in release with locked offline dependencies, no FFmpeg and
zero ignored tests. All 27 new asset hashes and generator syntax were verified.
The mixed-sub8 test binary links only libSystem/libiconv, not libav, libaom or
dav1d. The two controlled mutations failed for the intended chroma selection
errors, and production code was restored before the final regression run.

### AV1 4:2:2/4:4:4 geometry foundation (2026-10-08, playback pending)

Twelve owned two-frame 64x48 streams cover 4:2:2/4:4:4 at 8/10/12 bits in
lossless and lossy modes. Unmodified aomdec and dav1d emit identical packed
plane goldens; all six lossless goldens also equal the original source bytes.
The offline reproduction confirms actual sequence profile/depth/subsampling,
frame-header dimensions/lossless flags and the specific current native refusal
`AV1 native reconstruction requires 4:2:0`. This is a refusal test, not playback
acceptance. The goldens are checked in for enabling native acceptance with the
complete reconstruction fix.

Owned `av1_chroma_geometry.rs` now computes plane extents with separate horizontal
and vertical subsampling, including odd last samples. Reconstruction allocation
and final plane/transform-grid cropping use that geometry. Storage admission
counts plane samples and the existing transform/decoded maps instead of assuming
all chroma is 4:2:0. It preserves the former 4:2:0 margin and rejects arithmetic
overflow; unit expectations account for both 32-bit and 64-bit map element sizes.

The production format gate remains until block/transform/chroma-reference
geometry, intra/CFL/inter prediction, filters, intrabc, restoration/superres and
raw/player output paths have all been generalized and pixel/reset/WebM/seek
acceptance enabled. These streams must not be reported as successfully decoded
on the basis of passing header/refusal or geometry tests.

Validation: 46 AV1 release unit tests and seven selected offline integration
tests passed, including the twelve-case chroma refusal, intrabc, film grain,
odd restoration and temporal-motion super-resolution suites.

### AV1 intra/CFL/deblocking axis conversion (2026-10-08, playback pending)

Reconstruction intra chroma presence, residual chunk/transform coordinates,
intra edges and smooth-neighbor selection, coefficient contexts and deblocking
now use independent horizontal/vertical sequence subsampling. CFL sums the
corresponding 2x2, 2x1 or 1x1 reconstructed luma footprint and retains three
fractional bits; edge clamping uses the matching footprint on each axis.
A deterministic unit vector checks all three layouts and replicated edges.
The existing 4:2:0 reconstruction gate remains: inter chroma-reference grouping,
motion paths, other filters and output still need conversion and whole-stream
pixel/reset/WebM/seek acceptance. This step does not enable 4:2:2/4:4:4 playback.

Validation: 47 AV1 release unit tests and eight selected offline integration
tests passed, with zero failures/ignored tests. The integration set includes
chroma refusal, mixed-sub8 prediction, intrabc tools, film-grain tools, odd
restoration and temporal-motion super-resolution. Existing 4:2:0 full-pixel,
reference/reset and seek comparisons remain passing; these checks do not prove
4:2:2/4:4:4 reconstruction acceptance while the production gate is present.

### AV1 inter chroma axis conversion (2026-10-08, playback pending)

Reference pictures now retain sequence subsampling. Normal/scaled motion
compensation and warped prediction use separate horizontal/vertical reference
coordinates and active image bounds. Inter residual chunks, overlap prediction
geometry and inter-intra/compound mask averaging use the same independent axes.
Sub-8 chroma-reference groups expand only along subsampled axes; 4:2:2 therefore
combines horizontal neighbors without combining rows, and 4:4:4 has no implicit
sub-8 grouping. The original 4:2:0 counters and behavior remain in regression.

A deterministic motion test replaces decoded owned ramp reference planes with
original patterned 65x49 plane samples. It checks positive/negative integer
motion, active-edge replication and identity scaled prediction at all three
layouts and 8/10/12-bit depths against explicitly indexed expected samples.
This is motion-path coverage, not whole-stream 4:2:2/4:4:4 acceptance. The
production format gate remains pending intrabc, other filters/output conversion
and pixel/reset/WebM/seek acceptance of the twelve checked-in chroma streams.

Validation: final 48 AV1 release unit tests (including all 108 normal/scaled
motion cases) and eight selected offline integration tests passed, with zero
failures/ignored tests. Mixed-sub8, intrabc tools, film-grain tools, odd
restoration and temporal-motion/super-resolution retain their existing full
pixel/reference/reset/seek comparisons. Chroma-stream refusal remains passing
and is explicitly not an acceptance result. Formatting and scoped diff checks
passed.

### AV1 4:2:2/4:4:4 basic reconstruction acceptance (2026-10-08)

The native format gate is removed. All twelve owned two-frame 64x48 streams at
4:2:2/4:4:4 and 8/10/12 bits, lossless/lossy, match both independent oracle
pixel goldens. Decoder finish/reset, WebM raw output plane geometry/depth/pixels,
20ms presentation intervals, EOF rewind and sync seek replay pass. The former
refusal expectation is replaced with enabled acceptance. The initial lossless
4:2:2 entropy trailing-bit failure was reproduced by the existing owned fixture
and fixed by making lossless CFL eligibility depend on the actual UV 4x4 block.

Intrabc phases, CDEF block dimensions/direction remapping, horizontal upscaling
and restoration-unit/stripe geometry now honor separate subsampling axes.
WebM planar8 and high-depth packed output preserve full-height/full-width chroma
when required. Forty-eight final AV1 release unit tests pass. The base chroma
fixtures disable CDEF/restoration/intrabc/super-resolution; dedicated non-4:2:0
qualification for those tools, film grain, odd dimensions and broader inter
profiles remains pending and must not be inferred from base acceptance.

Final validation: 48 AV1 release unit tests and eight selected offline
integration tests passed with zero failures/ignored tests. The enabled chroma
acceptance covers all twelve streams, including container/reset/rewind/seek;
existing mixed-sub8, intrabc, film-grain, odd restoration and temporal-motion
super-resolution full-pixel regressions also pass. Scoped diff and formatting
checks pass. No external codec, FFmpeg or network is used by ordinary tests.

### Odd AV1 4:2:2/4:4:4 post-filter qualification (2026-10-08)

Eighteen owned two-frame 191x127 streams qualify deblocking, rectangular CDEF
(including 4:2:2 direction remapping) and Wiener/SGR restoration with independently
rounded chroma extents at 8/10/12 bits. All native pixels match unmodified
aomdec/dav1d goldens; decoder reset and WebM raw geometry/depth/pixels, intervals,
EOF rewind and seek pass. Enabled flags were insufficient: stock encoder trials
chose zero deblocking. Six generator-only patched-lib-aom cases therefore force
legal directional/UV levels [16,24,20,28], sharpness 2; the twelve remaining
cases use stock aomenc. The pinned source/patch and hashes are documented in
fixture provenance. No generator library is linked to FVid or used by tests.

Actual per-plane decode statistics cover CDEF, deblocking and non-None restoration
units in each layout/depth group, including all Y/U/V planes. First-decode totals
are 15,890 CDEF blocks, 28,300 deblocking edges, 19 Wiener and 26 SGR units.
Restoration allocation admission now reserves eight bytes per padded luma sample
for 4:4:4 (all pre-restoration planes plus the current plane clone), preserving
the prior six-byte margin for 4:2:0/4:2:2. Super-resolution, intrabc and film-grain
combinations in these layouts still need separate qualification; this stage does
not establish whole-codec conformance.

Validation: 48 AV1 release unit tests and nine selected offline integration
tests passed, zero failures/ignored tests. The new acceptance covers all eighteen
streams; base chroma, mixed-sub8, intrabc tools, film-grain tools, odd restoration
and temporal-motion/super-resolution regressions remain passing. All 54 artifact
hashes, generator Python syntax, Rust formatting and scoped diff checks passed.

### AV1 4:2:2/4:4:4 super-resolution qualification (2026-10-08)

Thirty-six owned three-frame 191x127 streams qualify constant denominators 9/12
and changing key/inter denominators 16/9 at 4:2:2/4:4:4 and 8/10/12 bits. Actual
headers and decoded coded MI grids are checked independently of display size;
all output pixels match stock aomdec/dav1d goldens. Decoder reset, raw WebM
plane dimensions/depth/pixels, 20ms intervals, EOF rewind and sync seek replay
pass. Deblocking/CDEF/restoration remain enabled and are proved by real per-plane
statistics, not flags alone: 61,548 CDEF blocks, 73,826 deblocking edges,
73 Wiener and 67 SGR units in the 108 first-decode frames. The initial single
noise pattern omitted U restoration at 4:2:2/12-bit; the accepted set uses two
original patterns and requires non-None restoration in every plane/layout/depth.

Generation uses the previously pinned forced-deblocking encoder only as a fixture
tool; both reference decoders remain unchanged. All 108 OBU/WebM/YUV artifact
hashes, generator syntax, formatting and scoped diff checks pass. Final 48 AV1
release unit tests and ten selected offline integration tests pass, zero
failures/ignored tests. These streams do not establish intrabc, film grain,
layered operating points or whole AV1 conformance.

### AV1 full-chroma film grain acceptance (2026-10-08)

156 owned 149x85 streams qualify native grain for 4:2:2/4:4:4 at 8/10/12 bits:
96 public-preset cases, 48 authored AR lag 0–3/scaling cases and twelve
show-existing cases. All 636 displayed frames match both stock independent
reference decoders, including four-frame inter ownership and a repeated display
of a stored showable reference. All pixel/depth/geometry, finish/reset, raw WebM,
20ms intervals, EOF rewind and sync-seek replays pass. Header coverage is required
separately in every layout/depth group, including actual inheritance in presets.
No production change was needed for this qualification. Intrabc/full-chroma,
more inter-tool combinations, layering and whole-codec conformance remain open.

Validation: 48 AV1 release unit tests and nine offline integration tests pass,
zero failures/ignored tests. Generator syntax, Rust formatting, all 468 artifact
hashes and scoped diff checks pass. The new test binary links only system
libSystem/libiconv, with no FFmpeg/libav or external codec linkage.

### Full-chroma AV1 intrabc tile-boundary acceptance (2026-10-08)

24 owned one-frame 769x257 intrabc streams and 24 disabled controls cover
4:2:2/4:4:4, 8/10/12 bits, lossless/lossy, SB64/128 and two tile columns.
The initial 4:2:2/8-bit lossless/SB64 fixture failed on displacement [-992,0]
at MI [113,31], 4x4 luma: an obsolete 4:2:0-only chroma margin incorrectly
rejected a legal zero top edge. `HasChroma` and sub-8 source margins now use
each subsampling axis independently, as required by AV1 `is_mv_valid`. An exact
unit regression checks that case plus the analogous full-resolution left edge.

Actual decode covers 31,661 copy blocks, including 20,525 sub-8 blocks, with
nonzero residual transform counts [2719,80061,80061] for Y/U/V. All source
luma displacement parities are required in every layout/depth group; a single
12-bit 4:4:4 lossless case does not select every parity and is not misreported
as such. Each stream verifies actual chroma phases: both horizontal phases in
4:2:2, only integral samples in 4:4:4. Phase statistics now count actual
chroma-reference blocks; displacement parity is recorded separately.

All pixels match independent unmodified libaom/dav1d goldens, including raw
WebM, 20ms intervals, finish/reset, EOF rewind and sync seek inside the frame.
Controls have zero copy/residual/phase/parity counters. All 144 artifact hashes
are verified. Fixtures and generation remain separate from offline tests.
Broader full-chroma inter combinations, layered streams and codec conformance
outside these owned cases remain open.

Final validation: 49 AV1 release unit tests and 15 selected offline integration
tests passed, zero failures/ignored tests. Generator syntax, Rust formatting,
all 144 artifact hashes and scoped diff checks pass. The intrabc test binary
links only system libSystem/libiconv, with no external codec/libav linkage.

### AV1 full-chroma inter tools and scaled-reference acceptance (2026-10-08)

24 original eight-frame moving sequences at 4:2:2/4:4:4 and 8/10/12 bits
match stock libaom/dav1d goldens in every output sample. Two authored patterns
exercise fixed 191x127 output and a 96x64 keyframe followed by 191x127 inter
frames. Finish/reset, all raw WebM frames, 20ms intervals, EOF rewind and sync
seek/full replay pass. Every layout/depth group has actual selected single,
average/distance compound, wedge/difference masks, OBMC, inter-intra,
local/global warp and scaled-reference prediction. New block statistics make
that coverage explicit; merely enabled encoder flags do not satisfy acceptance.

First-decode block totals are [70328,2908,596,515,141,9599,883,2562,168,8294]
in the tool order above. Actual scaled references are required in every resized
stream and forbidden in fixed-size streams. All 72 artifact hashes verify.
This qualifies these owned combinations; it does not prove whole-codec
conformance, every profile/tool interaction or layered/tile-list streams.

Final validation: 49 AV1 release unit tests and 23 selected offline integration
tests passed, zero failures/ignored tests, including existing OBMC, masked
compound, inter-intra and scaled-reference suites. Generator syntax, Rust
formatting, all 72 artifact hashes and scoped diff checks pass. The new test
binary links only system libSystem/libiconv, with no FFmpeg/libav/codec linkage.

### AV1 temporal operating-point selection and filtering (2026-10-08)

The native layered-operating-point blanket refusal is removed. Decoder callers
can select point 0–31 at creation or configuration seeding; the sequence validates
availability and reset preserves the selection. Extended OBUs outside either
selected temporal/spatial mask are dropped before payload parsing or changes to
pending frames, references, CDFs or HDR. Sequence headers and delimiters remain
unconditional. Included spatial-layer reconstruction still refuses explicitly.

An original stock-libaom SVC generator supplies three actual temporal layers,
64x48, 8-bit 4:2:0. Both unmodified reference decoders agree at operating points
0/1/2 (eight/four/two frames). The synthetic first sequence reproduces the former
blanket refusal. Lower-point acceptance also fixes the error-resilient dormant
reference invalidation gate: headers record invalidated physical slots; decoded
state clears them, while active invalid references remain errors. Acceptance
requires real invalidation at lower points, exact pixel replay, packetized/full
input, seeded/unseeded reset, default WebM timing/rewind/seek, filtered malformed
payload robustness, and HDR exclusion with a valid global control.

This stage does not complete spatial layering, tile-list assembly or all layered
profiles/decoder-model combinations. Whole-codec conformance remains unproven.

### Delivery verification: temporal operating points (2026-10-08)

A clean detached checkout of `211ac29d1`, using a separate Cargo target directory,
passed 49 AV1 unit tests and 19 integration tests across operating points, separate
frame groups, full-chroma inter/intrabc/grain, film-grain tools and scaled references.
Commands used `--release --locked --offline`; root integrations additionally used
`--no-default-features`. Scoped rustfmt, Python syntax, commit diff checks and all
five new fixture hashes passed. The operating-points test executable links only
libSystem and libiconv, with no libav or external codec library. This verification
does not qualify spatial layers, tile lists, every codec tool or live 60 fps.

### AV1 two-spatial-layer reconstruction acceptance (2026-10-08)

The spatial-ID blanket refusal is removed. Decoded frames expose `spatial_id`,
including show-existing outputs, so callers can distinguish included layers.
The original two-layer SVC fixture reconstructs four temporal units at
32x24/64x48, with actual scaled interlayer GOLDEN predictions; all outputs
match both stock libaom and dav1d. The lower operating point filters the upper
layer and reproduces its four independent lower frames. Whole-stream and
individual-OBU input, seeded/unseeded configuration and reset are qualified.

The raw decoder returns all included decoded frames. This milestone does not
qualify display-layer selection within temporal units, layered container
timing/seek, three/four-layer combinations, tile-list assembly, or all codec
profiles/tools. Those requirements remain open for the full codec goal.

Final validation: 49 AV1 unit tests and 12 selected integration tests passed
offline in release mode (zero failed/ignored). Scoped Rust formatting, generator
syntax, three artifact hashes and diff checks pass. The spatial acceptance binary
links only libSystem/libiconv; no external codec or libav linkage.

### AV1 spatial container presentation and temporal-unit boundaries (2026-10-08)

`decode_temporal_unit` reconstructs every included layer, then selects the
highest actually shown spatial ID for the container timestamp; raw decoding
continues returning all layers. WebM previously refused two visible spatial
layers in a packet, while MP4 returned the first/lower one. Owned MP4 and WebM
wrappers reproduce both errors. Four 20ms temporal units now show the upper
64x48 frame; a separate stream omitting the last upper layer shows the actual
32x24 lower frame in that unit. All selected pixels, timestamps and durations,
EOF rewind and full sync-seek replay are checked against committed dual-oracle
raw goldens.

Multiple temporal units in one timestamped packet remain refused, including
explicit delimiters and repeated shown layers without a delimiter. Errors
require reset; the raw all-layer API remains available. The wrappers are
generated with repository-owned Python only, without FFmpeg or external codecs.
This qualifies the two-layer owned cases, not all layered profiles, tile lists,
live playback performance or overall codec conformance.

Final validation: 49 release AV1 unit tests and 19 unique selected integration
tests passed offline, zero failures/ignored. The strengthened four-test container
suite passes after the full selected run. All eight container hashes, generator
syntax, scoped formatting and commit diff checks pass. The container test binary
links only libSystem/libiconv. These checks use the current local checkout;
whole-codec conformance and live GUI/performance remain separate qualifications.

### AV1 combined three-spatial/three-temporal operating points (2026-10-08)

The original 32x24/64x48/128x96 SVC sequence has eight temporal units and all
nine operating points, with temporal IDs 0/2/1/2/0/2/1/2. Complete independent
libaom/dav1d goldens qualify every output sample at every point; FVid passes
configuration/reset, temporal-unit/individual-OBU input and actual display
selection, including temporal units entirely excluded by the selected point.
Reference routing uses all eight physical slots without coupling excluded
temporal layers into admitted lower layers. Reconstructed scaled blocks are
required in all six upper spatial/temporal combinations, and base-layer
temporal prediction is measured at all three IDs. No runtime algorithm change
was needed for these owned cases.

MP4/WebM qualify eight 20ms units, upper 128x96 display, final 64x48 fallback
when the top frame is absent, all pixels and intervals, EOF rewind and full
sync-seek replay. Distinct multiple-unit/repeated-layer refusal fixtures and
reset-after-error remain covered. These generators use no FFmpeg; ordinary
tests read committed assets only. Broader layered bit depths/chroma, reference
motion fields, tile lists, decoder-model timing and whole-codec conformance
remain open.

Final validation: 18 selected offline release integration tests passed, zero
failed/ignored. All 18 new artifact hashes plus eight unchanged prior container
hashes verify; scoped formatting, Python syntax and commit diff checks pass.


## Native AV1 external camera tile lists (2026-10-08)

`Decoder::decode_tile_list` accepts an explicit camera header and owned external
anchor pictures, reconstructs camera tiles using the native AV1 entropy and
picture path, and assembles them in entry order. Sparse lists preserve the
previous canvas. Camera constraints, anchor indices, coordinates, payload
bounds and retained/output memory are checked before reconstruction.

The owned 128x128 lossless 8-bit 4:2:0 fixture uses four 64x64 tiles and LAST-only
inter prediction. All pixels match stock libaom and the independently authored
permutation. Tests cover sparse updates, repeat calls, anchor index 127, malformed
lists, camera/canvas geometry and memory refusal. Ordinary packet decoding
refuses OBU 8 with a request for external camera context; that refusal is distinct
from explicit API acceptance. Automatic container-side provisioning, additional
depths/chroma and SB128 are qualified in the subsequent matrix below; additional CDF combinations remain unqualified.


## AV1 distinct tile-list anchors (2026-10-08)

The owned tile-list regression now qualifies two different external anchors,
selected 0,1,0,1 across four output tiles. Every native output pixel matches
stock libaom and the independent authored formula. Replacing the second anchor
with the first is a failing pixel mutation, demonstrating that this case
actually exercises anchor selection rather than repeated pointers. Generation
uses a clipping-safe original pattern; previous one-anchor, sparse-canvas,
bounds and memory regressions remain enabled. This does not establish all
external camera profiles or automatic container-side camera context.


## AV1 camera tile-list depth/chroma/SB matrix (2026-10-08)

The same native external-context API now has exact pixel acceptance for all
18 combinations of SB64/SB128, 8/10/12-bit depth and 4:2:0/4:2:2/4:4:4.
Each original lossless camera uses a 2x2 tile grid (128x128 or 256x256 frame),
LAST-only prediction and two distinct selectable anchors. Stock libaom and
independent Python calculations agree for one/two-anchor permutations. Offline
Rust tests also recompute every sample, including non-square 4:2:2 chroma
tiles and actual values above 255. All cases retain sparse canvas preservation,
anchor mutation sensitivity, malformed/budget refusals and state isolation.

Four test functions cover the 18 acceptance cases and the separate ordinary
packet refusal without external camera context. This closes the unqualified
depth/chroma/SB matrix for these camera lists, not remaining codec tools: other
CDF contexts, lossy camera combinations, motion/border cases and automatic
container external-context provisioning still need qualification.


## Native AV1 lossy external camera tile-list reconstruction (2026-10-08)

The owned Q32 camera matrix now covers all 18 SB64/SB128, 8/10/12-bit and
4:2:0/4:2:2/4:4:4 combinations. Anchors remain lossless authored pictures;
camera samples add a spatial 8x8 variation before quantization. Generation
compares stock tile-list output to separate stock camera-tile reconstruction,
and requires lossy output to differ from source. Native acceptance compares
every one/two-anchor oracle sample, requires nonzero quantization and inter
blocks, preserves sparse canvas and repeats malformed/budget/state checks.

A dequantization mutation must successfully reconstruct different pixels,
not merely refuse the modified header. Replacing the second external anchor
with the first also changes pixels. Five offline test functions now cover 36
lossless/lossy camera cases plus ordinary missing-context refusal. No external
codec is called or linked in ordinary tests. Further motion/border, CDF-state,
quantizer/tool combinations and container-side external context remain open;
this stage does not claim complete codec conformance.


## AV1 camera tile lists with inherited adapted CDF (2026-10-08)

The explicit native camera-context path now has 36 additional cases whose
normal lossless anchor enables symbol CDF adaptation and frame-end CDF saving.
The anchor uses a 2x2 tile layout. The generator switches to large-scale camera
mode without an intervening keyframe, so primary reference LAST inherits the
actual anchor CDF rather than a fresh default context. Camera CDF updates remain
disabled as required for this large-scale mode.

All lossless/Q32, SB64/SB128, 8/10/12 and 420/422/444 cases match stock libaom
one/two-anchor pixels and separately reconstructed camera tiles. Native tests
assert the parsed anchor adaptation/save flags and camera primary reference 0.
A mutation that selects PRIMARY_REF_NONE must fail entropy reconstruction or
produce different pixels; that mutation refusal is distinct from the passing
normal acceptance. Six test functions cover 72 accepted camera cases plus the
ordinary missing-external-context refusal.

Primary-ref-none encoded camera streams, more reference/CDF slot mappings,
context-update-tile choices, motion/border interactions and automatic container
side information remain separate qualifications. This evidence does not imply
whole-codec conformance.


## AV1 camera PRIMARY_REF_NONE encoded acceptance (2026-10-08)

The external-context matrix adds 36 camera streams that genuinely encode
PRIMARY_REF_NONE using stock libaom's frame flag. Anchors retain adapted CDF
state, so the camera must initialize fresh non-coefficient and coefficient
contexts instead of silently loading LAST. All lossless/Q32, SB64/SB128,
8/10/12 and 420/422/444 cases match stock list and separate tile reconstructions.

Native tests assert primary_reference=7 in parsed camera headers, actual anchor
adaptation/save flags and all one/two-anchor samples. A mutation selecting LAST
instead must produce wrong pixels or entropy refusal; it is distinct from the
passing encoded PRIMARY_REF_NONE acceptance. Existing dequantization mutation,
anchor selection, sparse canvas and malformed/memory checks remain active.
Seven offline test functions cover 108 accepted cases and a separate ordinary
packet missing-context refusal, with no external codec calls/linkage.

Additional reference-slot mappings, context-update-tile choices, motion/border
interactions and automatic container external context remain unqualified, as
do other codec/profile gaps. This stage does not establish complete conformance.


## AV1 camera tile lists with proven nonzero motion (2026-10-08)

The native inter-block path now exposes counters for successfully reconstructed
blocks with nonzero coded motion and fractional luma motion. Tile-list outputs
sum the counters for only the current call, including sparse canvas updates.
The new 108 camera cases shift the owned image by 4x2 luma pixels, respecting
each chroma axis and clamping the authored source at its visible edges.
Every case requires nonzero_motion_blocks>0, in addition to exact stock pixels.

A fast stock encoder candidate yielded no nonzero motion in a 10-bit lossless
case; it was replaced, not accepted as motion coverage. Generation uses cpu-used
0 for moving cases, preserves stock coding/oracles, and extends the second
external anchor border with edge samples. Individual entry lengths remain
bounded by their 16-bit syntax; the generation tool's aggregate scratch buffer
now accommodates four entries rather than incorrectly imposing 64 KiB overall.

All three CDF contexts, lossless/Q32, SB64/SB128, 8/10/12 and 420/422/444 moving
cases pass pixel, CDF/dequantization/anchor mutation, sparse-canvas and bounds
checks. Eight offline functions cover 216 accepted cases plus ordinary missing
context refusal. Forty-six AV1 units and full-chroma/spatial/spatiotemporal
integration regressions pass. Fractional motion is counted but not yet a required
fixture property; coded off-frame vectors, additional motion tools/interactions
and automatic container external context remain separate qualifications.


## AV1 tile-list coded border and fractional motion acceptance (2026-10-08)

The inter-block path now counts unscaled translation blocks whose visible
coded reference rectangle crosses the frame boundary. It excludes scaled/warp
geometry and zero-MV frame padding, and increments only after successful
reconstruction. Tile-list outputs expose the count and reset it for each call,
including sparse updates.

All 108 moving fixtures require border_motion_blocks>0, alongside nonzero
motion and exact stock pixels. Forty-two also require fractional luma motion:
lossless 8-bit 422 at both SB sizes; Q32 422 at SB64 for all depths; Q32 at SB128
for all depths/chroma. Each group spans default, adapted-LAST and encoded
PRIMARY_REF_NONE camera contexts. Requirements are explicit in manifests and
asserted by offline acceptance; diagnostics alone are no longer the evidence.

All 216 camera cases, 46 AV1 units and full-chroma/inter/spatial/spatiotemporal
regressions pass without external decoders. Existing fixture binaries were
unchanged on regeneration. This qualifies the observed coded border/fractional
paths, not every direction/phase/filter combination, scaled/warped boundary
interaction or all remaining codec gaps.

## AV1 tile-list reverse motion and directional border proof (2026-10-08)

108 additional owned reverse-motion cases bring acceptance to 324 cases.
The authored shift is -4/-2 luma pixels with clamped source coordinates.
Decoded border counters distinguish left, top, right and bottom; manifest
requirements assert actual coded coverage, not inferred source motion.
Periodic source patterns can cause the encoder to select opposite-sign vectors
and compensate with residuals. Each of the 108 positive/reverse pairs requires
all four edges jointly across depth, chroma, SB, quantizer and CDF contexts.
Observed positive coverage counts are 108/0/72/108, reverse 72/108/98/0
in left/top/right/bottom order. False requirement flags do not assert absence.

Nine tile-list test functions, 46 AV1 units and three adjacent integration
suites pass offline. All 2268 artifact hashes were checked. Every fractional
phase/filter, scaled/warped boundary interaction, external container context
and wider codec gaps remain separate qualification work.

## AV1 fractional predictor phase/filter/edge matrix (2026-10-08)

An offline scalar-convolution qualification now evaluates 4,423,680 prediction
configurations against both normal translation and identity-scaled prediction.
It spans 420/422/444, 8/10/12-bit, all three planes, 4x4/4x8/8x4/8x8 blocks,
four frame corners plus an interior origin, all 16 filter pairs, signed row/col
vectors -8..7, and both single and compound intermediate rounding. This covers
all eight reachable luma/full-chroma phases and all sixteen subsampled chroma
phases on their respective axes. Odd 33x29 reference dimensions exercise edge
clamping independently of padded geometry.

The oracle computes each pixel independently with 64-bit accumulators instead
of sharing optimized source-row or intermediate-buffer indexing. Filter taps
are shared normative tables, so this proves coordinate selection, axis mapping,
border extension and rounding consistency, not independent correctness of the
filter constants. References use the existing owned synthetic ramp picture
metadata with original procedural plane samples; no private media, external
codec or fixture generation runs in these tests. Whole-stream coded phase
coverage, nonidentity scaling and warp interactions remain separate work.

All 47 AV1 unit tests pass in both the active checkout and a clean committed
copy with only this test added. Deliberately replacing the production vertical
subsampling shift with the horizontal one makes the new test fail precisely on
8-bit 422 Cb at the top-left edge; the mutation was removed afterward. This
checks that the oracle detects the axis regression rather than merely matching
two shared implementations.

## AAC-LC ancillary EXT_DATA_ELEMENT playback (2026-10-08)

The owned fill parser now accepts version-zero ancillary data, including empty
payloads, chained 255-byte length fields and multiple extension payloads per FIL.
It bounds every read by the declared fill length and commits the bit cursor only
after complete success. Ancillary bytes never change audio synthesis. EXT_FIL
and conforming EXT_FILL_DATA remain supported; malformed fill-data bytes are
now rejected. SBR/audio tools and unknown ancillary versions remain explicit
unsupported behavior, including an SBR extension after accepted ancillary data.

Nine six-frame synthetic ADTS variants plus a matching authored Y4M companion
reproduce acceptance/refusals. Pre-fix ordinary acceptance failed on the precise
fill-tool refusal. Four enabled integration tests verify exact nonzero PCM,
explicit PCE setup, malformed/unsupported extensions, all escaped-packet
truncations, unchanged decoder state and the six matching video intervals.
A parser unit checks transactional bounds at every initial bit offset.

The four integrations pass in a clean copy; 13 existing PCE and five coupling
tests also pass there. The parser unit passes locally. The clean test binary
links only libiconv/libSystem, without FFmpeg/libav. Existing EXT_FILL_DATA test
construction was corrected from invalid zero padding to the required 0xa5 bytes.
This closes ancillary fill playback, not SBR/HE-AAC, gain control, height layouts
or the broader unresolved codec tools. Generation is explicit, uses committed
owned media only, and requires no external encoder or network.

## AAC explicit PCE height layouts (2026-10-08)

The owned PCE implementation now parses normal/top/bottom information from
CRC-protected height comments, with bounded payload reads and transactional
PCE cursor updates. Short 0xac comments remain ordinary application comments.
The decoder exposes `channel_positions()` in emitted PCM order; positions carry
layer, front/side/back/LFE group and ordinal/group size. Recognized horizontal
and top layouts retain exact WAVE masks. Layouts absent from WAVE or lacking
a unique standard bit assignment use mask zero and explicit positions rather
than rejecting audio or inventing a horizontal speaker mapping. This also
accepts valid four-front-channel, multiple-LFE and separate-SCE PCE groups.

In-band PCE checks now compare height layers in addition to the previous
configuration fields. Equal zero masks cannot hide a top/bottom layout change.
Checkpoint configuration equality and per-channel synthesis state remain intact.
WAV retains PCM and representable masks; unrepresentable positions require the
explicit decoder metadata because the standard WAVE mask cannot encode them.
MP4 and Matroska remux retain complete AAC initialization and exact decoded PCM.

Seventeen originally authored six-frame cases compare each channel to an
independent direct-cosine/sine-window oracle. Two malformed derivatives cover
CRC damage and reserved layer 3; additional tests cover all PCE truncations,
reset, same-mask layout-change rollback, explicit positions, WAV export and
container remux. A matching six-frame Y4M companion and 37 artifact hashes are
committed. The pre-fix height refusal and the independent unrecognized-group
refusal were each reproduced before enabling their acceptance.

Five height tests, four fill tests, thirteen existing PCE and five coupling
tests pass in a clean copy without active drafts. Fifty-two AAC unit tests pass
locally. The clean binary links only libiconv/libSystem, without FFmpeg/libav.
This closes explicit PCE height/group decoding; HE-AAC/SBR, gain control and
other unresolved codec tools remain independent work.

## AAC-LC pulse selection for spectral and non-spectral bands (2026-10-08)

The owned channel path now applies pulse amplitudes only within coded spectral
bands (codebooks 1..11 with a spectral scalefactor). Valid positions beyond
max_sfb or in ZERO_HCB/PNS/intensity bands no longer spuriously reject playback
or contaminate those tools. Coded pulses still accumulate before inverse
quantization, including repeated positions and the negative zero branch.
Selection evaluates at most four positions and introduces no scratch allocation.

Twelve originally authored six-frame cases, twelve pulse-free controls and a
matching synthetic video cover near/far uncoded starts, repeated/crossing/zero
spectral pulses, zero bands, PNS and both intensity codebooks. Seven spectral
pairs have independently evaluated direct-cosine PCM; PNS/intensity cases match
their valid pulse-free controls exactly across all frames. Thirty-nine artifact
hashes were verified. Before implementation, ordinary acceptance reached the
specific above-coded-band refusal; an independent pre-fix probe proved each
zero/PNS/intensity case reached its precise zero-codebook failure after its
pulse-free baseline successfully decoded.

Five pulse tests verify nonzero PCM, exact tool/random-state preservation,
invalid band/offset rollback, all packet truncations, reset/checkpoint behavior,
owned WAV export and video intervals. These pass in a clean copy alongside
four fill, five height, thirteen PCE and five coupling tests. Fifty-two AAC units
pass locally. The clean binary links only libiconv/libSystem; ordinary tests
need neither FFmpeg/libav nor network. This closes the discovered pulse/band
interaction failures, not HE-AAC/SBR, SSR gain control or full codec conformance.
### 2026-10-08 — indexed AAC-LC 7.1 Top

The owned ASC parser and native AAC decoder now accept channelConfiguration 14,
with normal FC/FL/FR/BL/BR/LFE followed by top-front left/right. PCM uses ascending
WAVE speaker bits and mask 0x503f; existing configurations retain their maps.

Eight original six-frame Matroska streams cover 44.1/48 kHz, 960/1024 sample
frames, common and separate CPE windows. Independently authored PCE controls
prove the identical packets were valid before the fix; direct-cosine PCM covers
every speaker with distinct signed energy. Pre-fix acceptance in a clean HEAD
failed exactly at `unsupported AAC channel configuration`.

Five ordinary offline tests pass for nonzero PCM, complete channel geometry,
fractional interval/preroll boundaries, reset/checkpoint, every second-packet
truncation after a valid frame, wrong horizontal element order, incompatible
checkpoint refusal, WAV export/mask and synthetic video intervals. Forty
artifact hashes and deterministic regeneration were checked. Adjacent fill,
height, pulse, PCE, coupling and 960-sample acceptance also pass in a clean copy.
The test binary links libiconv/libSystem only. No FFmpeg/libav is invoked.
This closes the indexed 7.1 Top refusal, not HE-AAC/SBR or full codec conformance.

### 2026-10-08 — owned SBR frequency-table foundation

`owned_aac::aac_sbr_bands::FrequencyTables::from_qmf_bounds` implements master,
high/low envelope and noise frequency geometry from supplied k0/k2 bounds.
Linear scales include endpoint adjustment; logarithmic scales include one/two
regions, the 1.3 warp and bounded width correction from the published 2003
standard (the older working draft omitted the correction limit). Crossover,
core QMF range and noise-band limits are checked before tables are returned.
All allocations are bounded by the 64-subband domain; no foreign decoder is used.

Explicit generation produces 1,152 original 80-digit Decimal reference vectors:
656 accepted geometries and 496 invalid combinations. Ordinary offline unit
tests compare exact master/high/low/noise borders, cover all three-bit crossover
values and all QMF bounds, and pin each linear endpoint adjustment. Disabling
the warp in a disposable clean copy makes the Decimal test fail specifically
for [11,35,3,true,0,0]; restoring it returns the owned AAC unit suite to green.
Deterministic generation and the oracle SHA256 were verified.

This is a DSP building block, not SBR playback acceptance. Rate-dependent
start/stop derivation, header/grid/envelope parsing, QMF analysis/synthesis,
HF generation/adjustment, state/timeline integration and encoded nonzero PCM
regressions remain required. No unsupported SBR refusal has been converted to
an acceptance claim, and the full codec goal remains open.


### Owned SBR header syntax foundation

`owned_aac::aac_sbr_header::Header` reads the published legacy SBR header
inside an explicit absolute payload bit boundary. Reads commit only on success;
absent extra sections select the normative defaults independently of past headers.
The three-bit crossover range is now also enforced by frequency geometry.
Hand-authored bit strings cover all four extra-section combinations at every
byte offset, every exact bit truncation, impossible payload limits, and a trailing
sentinel proving the parser stops before the following payload. These are syntax
acceptance tests, not encoded HE-AAC playback acceptance. Rate-dependent bounds,
grid/envelope/noise parsing, QMF and high-frequency reconstruction remain pending.
Reference: GOST R 53556.4-2013 tables 63 and 105–111 (MPEG-4 Audio syntax).


### Owned SBR grid syntax foundation

`owned_aac::aac_sbr_grid::GridSyntax` transactionally parses FIXFIX, FIXVAR,
VARFIX and VARVAR, retaining both relative-border lists and the pointer.
FIXVAR frequency-resolution fields are reversed into envelope order. Noise
envelope counts and the one-envelope FIXFIX amplitude-resolution override are
exposed. Hand-authored syntax vectors check exact consumption and rollback at
every truncated bit and every byte offset. The layer parses bounded raw syntax;
it does not yet validate envelope counts, pointer legality, time geometry or
frame-to-frame continuity, and is not HE-AAC playback acceptance.
Reference: GOST R 53556.4-2013 table 69 (legacy MPEG-4 SBR).


### Owned SBR time grid foundation

`GridSyntax::time_grid` reconstructs envelope and noise-floor borders for
15/16-slot (960/1024 core sample) frames. It applies NINT fixed-grid spacing,
leading/trailing relative borders and the class-specific middle-border rule.
It validates class shape, legal relative steps, offsets, envelope counts,
pointer bounds and strictly increasing borders. `read_validated` commits bit
position only after syntax and geometry succeed. Independent hand-written
expected borders exercise all four classes and both core lengths. Malformed
examples first pass the raw parser before failing count/overlap validation,
so rejection cannot pass accidentally through unrelated truncation. Public-field
extremes are checked as well. This is still syntax/geometry coverage, not coded
HE-AAC playback acceptance; cross-frame continuity and QMF/HF decoding remain.
Reference: GOST R 53556.4-2013 sections 6.18.3.3, 6.18.3.6 and table 174.


### Owned SBR delta/inverse-filter control syntax

`owned_aac::aac_sbr_controls` reads envelope/noise delta directions and
noise-band inverse-filter modes transactionally within an extension boundary.
Delta counts come from a validated time grid; inverse-filter counts are
independent frequency-table noise bands (1–5), including the shared list in
coupled stereo. Exhaustive tests cover every direction pattern for 1–5
envelopes and every filter sequence for 1–5 bands, all eight starting bit
offsets, every truncation, trailing sentinels and invalid external geometry.
No coefficient Huffman decoding or PCM rendering is claimed by these syntax
tests. Huffman/delta reconstruction, frame state, rate-dependent frequency
bounds, QMF and high-frequency adjustment still remain before HE-AAC acceptance.
Reference: GOST R 53556.4-2013 tables 70, 71, 116 and 117.


### Owned SBR Huffman decoding foundation

`Book::decode` decodes all ten normative SBR prefix books to signed deltas
inside a payload bit boundary, committing position only on a complete codeword.
All 604 symbols are tested at eight bit offsets, with every exact truncation
and trailing sentinels. Full prefix-free/Kraft checks are independent of the
decoder. Generation cross-checks every protocol constant against both published
translation and independently parsed ISO draft tables; explicit errata prevent
translation duplicates and malformed lengths from entering production.
The decoder is owned Rust. It does not yet reconstruct envelope values or
render HE-AAC; book selection, stereo/temporal delta reconstruction, rate bounds,
QMF and high-frequency processing still require integration and PCM acceptance.


### Owned SBR coefficient syntax and delta reconstruction

`aac_sbr_coefficients` reads level/balance envelope and noise rows inside an
extension boundary. Selection covers time/frequency books, 1.5/3 dB envelope
resolution, 5/6/7-bit absolute envelope values, 5-bit noise starts, and the
one-envelope FIXFIX resolution override. Whole blocks restore bit position on
truncation. Noise frequency books reuse the normative envelope 3 dB books.
`reconstruct` performs checked frequency accumulation or temporal addition with
physical interval mapping across nested high/low band tables; it doubles newly
transmitted stereo-balance values without doubling previous stored values.
Mixed-resolution/direction tests assert independently written quantized values
and exact encoded consumption, and exercise all-bit rollback for complete blocks.
Invalid dimensions, incompatible histories and arithmetic overflow are rejected.

This is coefficient-level coverage, not HE-AAC playback acceptance. Previous
frame coefficients are still supplied explicitly; production frame state,
quantized conformance bounds, dequantization/stereo uncoupling, rate-dependent
frequency bounds and the QMF/high-frequency pipeline remain to be integrated.
Reference: GOST R53556.4-2013 tables 72/73 and section 6.18.3.4; ISO SBR
normative codewords are separately qualified by the Huffman generator.


### Owned SBR dequantization and coupled stereo

`aac_sbr_dequant` converts reconstructed quantized envelope/noise scalefactors
into energy/noise ratios and recovers coupled left/right values. Both amplitude
resolutions, their distinct pan offsets, even reconstructed balance values,
legal noise-level ranges and the opposite envelope/noise panning orientations
are implemented. Independent channel fractions avoid subtractive loss for the
quieter channel. Nonfinite/unrepresentable output is rejected explicitly.
Tests compare 530 independent 80-digit Decimal cases, neutral panning, mirrored
channels, envelope energy conservation, explicit asymmetric left/right examples,
parity/noise bounds and numerical extremes.

This remains coefficient/DSP qualification. HE-AAC encoded playback acceptance
requires production frame state, output-rate frequency bounds, integration of
SBR syntax, QMF and high-frequency generation/adjustment. No HE-AAC support claim
is made from these standalone numerical tests.


### Owned SBR analysis QMF foundation

`aac_sbr_qmf::Analysis` implements the normative 32-band complex analysis
bank: 320-sample retained history, newest-first input orientation, every-other
QMF window coefficient, five polyphase contributions and complex modulation.
Immutable modulation tables are shared; cloned histories remain independent.
Full-call PCM validation precedes mutation; reset discards prior filter state.
960/1024 core frames produce 30/32 subband slots. Four independently generated
22-block direct-convolution traces cover impulses, boundary samples, dense
nonzero signals and tones. Tests also verify bit-identical chunked/whole
operation, checkpoint replay, reset and invalid-call rollback. All 640 window
constants were cross-checked against two standard layouts.

This is analysis-bank DSP qualification only. The production HE-AAC path still
requires rate bounds, frame history/integration, HF generation/adjustment and
QMF synthesis, followed by encoded fixtures with complete PCM acceptance.
The direct matrix implementation has not yet been benchmarked or optimized.


### Owned 64-band SBR synthesis QMF

`aac_sbr_synthesis_qmf::Synthesis` implements the normative complex modulation,
1280-sample retained history, window extraction and 64 chronological output
samples per subband slot. Modulation tables are shared; filter history is owned
and cloneable. Retained history commits only after complete finite input and
arithmetic validation, including failure after an earlier valid slot.
Four independent 22-slot traces cover real/imaginary basis signals, dense
complex input and analysis bypass. Tests compare direct convolution, exact
whole/chunked execution, clone/replay/reset, NaN/infinity and finite overflow
rollback, and the composed owned analysis/synthesis path against saved PCM.
30/32-slot frames produce 1920/2048 samples.

DSP components are qualified separately; HE-AAC playback still requires
rate-dependent bounds, full frame syntax/history integration and high-frequency
generation/adjustment with encoded PCM acceptance. The direct matrix kernels
have no performance qualification yet. Downsampled 32-band synthesis remains
to be implemented as part of full SBR coverage.


### Owned downsampled SBR QMF synthesis

`aac_sbr_downsampled_qmf::Synthesis` implements 32-band complex synthesis with
640 retained samples, 64-sample shifts, the normative half-sample phase and
every-other window coefficient. It produces 32 chronological samples per slot
(960/1024 per 30/32 slots), preserving atomic call commit and clone/reset state.
Four independently calculated direct traces and an exact decimation identity
against zero-upper-band 64-channel synthesis qualify phase, scaling, window
extraction and ordering. The actual owned analysis/synthesis composition,
chunking, checkpoint replay, reset and invalid-input rollback are also tested.
Both output-rate synthesis variants are now present as standalone DSP; neither
is wired into production HE-AAC playback yet. Rate bounds, frame integration,
HF generation/adjustment and encoded acceptance remain. No performance claim.

### Owned SBR inverse-filter chirp history

`aac_sbr_chirp::Chirp` retains inverse-filter modes and bandwidth factors for
1–5 noise frequency bands. It implements all 16 previous/current mode targets,
asymmetric smoothing, and the strict 1/64 zero threshold in published
GOST R53556.4-2013 section 6.18.6.2/table 175. Geometry mismatches fail before
changing history; clone/replay and reset retain deterministic frame state.

The independent 80-digit Decimal oracle covers every five-frame mode sequence
(1024 sequences), followed by eight Off frames to exercise decay and zeroing.
Tests also check exact threshold behavior, independent multi-band histories,
checkpoint replay and invalid-call rollback. These are original numeric DSP
vectors, not encoded HE-AAC acceptance fixtures. Predictor covariance, patch
generation, HF adjustment and production integration still remain. No new
encoded-media failure or full HE-AAC playback acceptance is claimed.

### Owned complex SBR covariance predictor

`aac_sbr_predictor::predict` computes complex covariance and both prediction
coefficients for legacy SBR windows of `2*numTimeSlots+6` samples plus two
preceding samples (15/16 slots). It applies the published zero-denominator and
magnitude >=4 reset rules, with relaxation epsilon 1e-6 verified in the ISO
working draft. Common input normalization preserves coefficient ratios while
preventing finite-input covariance overflow. Invalid windows/non-finite inputs
are rejected before calculation.

Independent 80-digit Decimal vectors cover silence, real/complex constants,
dense rational signals, a complex ramp and unstable exponential growth for
both frame sizes. Additional checks cover zero past energy, huge finite input,
power-of-two scale invariance and quarter-turn complex phase invariance.
This remains an isolated DSP component: the enclosing frame buffer must select
the specified window; patch construction, HF generation/adjustment and encoded
HE-AAC production acceptance remain incomplete.

### Owned SBR patch selection and complex HF generation

`aac_sbr_hf::patches` implements the protocol patch decision flow (ISO SBR
Figure 9): output-rate goal, master-boundary descent, parity alignment,
bounded source bands, progress checks and removal of a final patch shorter
than three bands. `generate` composes source-band covariance predictors with
per-noise-band chirp factors, including the squared factor for the second
coefficient. It fills the envelope time interval at the tHFAdj origin and
leaves unused slots/bands and a discarded short tail zero. Input frame, grid,
patch, noise and bandwidth geometry is validated before generation.

Original 80-digit Decimal traces independently compute source covariances and
HF output for both frame sizes, two patches, different noise factors and an
offset envelope. Tests connect actual patch selection to these traces, verify
zero-chirp exact copies, and sweep thousands of patch geometries across nine
output rates for bounded sources, phase parity and coverage. These remain DSP
traces, not encoded HE-AAC playback acceptance. Rate-derived frequency bounds,
limiter tables, envelope adjustment, frame buffering and production integration
are still required.

### Owned SBR limiter frequency borders

`aac_sbr_limiter::borders` implements limiter modes 0–3, merging low-resolution
borders and internal patch borders. The Figure 10 density/0.49-octave rule
removes duplicates and close non-patch boundaries while preserving distinct
patch boundaries. It validates contiguous parity-aligned patch geometry and
the optional one/two-band unpatched tail. All modes preserve the full low-table
endpoints. Internal candidates follow Figure 4.40; range endpoints additionally
remain protected so the corrected gain k(m) covers every m<M. A discarded
tail is not itself a patch boundary. See the composed DSP qualification below
for the distinction between this coverage resolution and literal flowchart
conformance.

1680 original geometries are independently evaluated with 80-digit Decimal
exponential thresholds, rather than the implementation's floating log2.
Coverage includes all modes, duplicate borders, varied low-table spacing,
multiple patches, and tails. Source: ISO SBR Figure 10 (printed page 41),
with published GOST R53556.4-2013 6.18.3.2.3 confirming table construction.
This is frequency geometry, not amplitude limiting or encoded HE-AAC
acceptance. Envelope energy estimation, gain/noise/sinusoid adjustment and
production frame integration remain necessary.

After retaining range endpoints, the arbitrary-geometry oracle has 48
incompatible source-range cases (refusal checks), and 1632 frequency-geometry
acceptances. Twenty formerly empty-partition refusal expectations became
acceptance checks; retaining full endpoints gives every gain band an interval.

### Owned SBR current-envelope energy estimation

`aac_sbr_energy::estimate` implements both `bs_interpol_freq` modes in published
6.18.7.3: per-QMF-band time means or joint time/frequency means replicated over
the current envelope's low/high resolution bands. Input and output use the
HF generator's tHFAdj-relative origin; SBR time borders are multiplied by RATE=2.
Published inclusive frequency width (`kh-kl+1`) is used, including one-band
regions. A normalized RMS calculation avoids intermediate square overflow
when the final mean is representable; genuinely unrepresentable energies fail
explicitly. Geometry and non-finite inputs are validated before calculation.

Independent 80-digit Decimal references cover both frame sizes, both modes,
mixed frequency resolutions, offset/multiple envelopes, silence, dense complex
signals, a boundary impulse, constant bands and independently generated HF.
The last case connects actual owned patch/predictor/HF generation to the saved
energy oracle. Additional checks cover exclusive boundaries, single-band
normalization, sparse huge inputs, subnormal energy and malformed public
geometry. These are numeric DSP acceptance checks; encoded HE-AAC acceptance
and the rest of gain/noise/sinusoid adjustment remain incomplete.

### Owned SBR gain, additional levels and amplitude limiting

`aac_sbr_gain` calculates raw gain/noise/sine amplitudes, limits gain to the
per-limiter-band maximum, reduces noise proportionally, and applies capped
energy compensation. It handles all four limiter-gain modes and suppresses
noise's gain/boost contribution at the attack/carried attack envelope (the
caller supplies that flag). Sine energy is assigned only to `S_IndexMapped`
QMF lines, while `S_Mapped` band presence controls the raw gain branch.

Official ISO/IEC 14496-3:2001/Amd.1:2003/Cor.1:2004 pp4/11 corrects epsilon
to 1 and the sine indicator to `S_IndexMapped`. This supersedes the older
formula printed in the translated GOST for sine amplitude. These functions
use the standard's QMF energy units; normalized core PCM needs the matching
scale conversion at production integration. Common energy/amplitude scaling
avoids finite-input sum/square overflow in limiter ratios. Input amplitudes
must remain within the physical pre-limiter bounds.

60 independent 80-digit Decimal references cover harmonic presence versus
actual harmonic line, zero/tiny/current energy, attack suppression, all four
gain modes and three limiter partitions. Tests additionally cover huge finite
inputs, all-zero output and malformed public inputs. Numeric acceptance is not
encoded HE-AAC playback acceptance. Harmonic/history mapping, final signal
assembly and production frame integration remain incomplete.

The same official corrigendum p8 replaces the patch goal equality test with
`fMaster[k]-sb<3`. `aac_sbr_hf::patches` now follows it; an original master
table 21..63 step 2 at 48 kHz distinguishes the corrected two-patch result
from the old unnecessary internal two-band patch. This is a protocol geometry
regression, not a newly reported failure of an encoded/private source video.

### Owned SBR parameter and harmonic mapping history

`aac_sbr_mapping::History` maps dequantized envelope/noise rows and estimated
current energy to the gain calculator's per-QMF `Band` inputs. It computes
attack index for all four frame classes, including an attack at the frame end
carried to the next frame. Harmonic lines start at the attack unless that
absolute QMF line was active in the previous frame's final envelope; bandwise
harmonic presence is separate from actual center-line placement. The midpoint
is the integer floor of the high-resolution band borders.

History is transactional and clone/reset capable, retaining absolute QMF
lines rather than high-table indices. New/out-of-range lines cannot inherit
unrelated harmonic history after a frequency-range change. The bounded
`read_harmonics` parser rolls back on truncation and clears the entire list
when the enclosing harmonic flag is absent.

256 original exact mapping oracles cover FIXVAR/VARFIX pointers, all five-band
harmonic masks, mixed frequency resolution and both noise-time rows. Other
tests cover FIXFIX/VARVAR, 15/16-slot frames, deferred attack/history replay,
range changes, late-row invalid input rollback, and all short harmonic bit
patterns at every bit offset/truncation. Mapping connects to the owned gain
calculator. These are protocol/DSP tests, not encoded HE-AAC playback
acceptance; final signal assembly and production extension integration remain.

### Owned complex SBR HF signal assembly

`aac_sbr_assembly::Assembly` applies smoothed gains, the normative complex
noise sequence and parity-aligned four-phase sinusoids to the generated HF
QMF matrix. It retains four raw gain/noise history slots and noise/sine phases
across frames. Attack/carried-attack envelopes bypass gain smoothing and mute
noise; sinusoidal channels also mute noise. Header reset primes smoothing
history and resets noise index while preserving sine phase; decoder/seek
reset clears all state. Geometry changes require header reset. Entire calls
are transactional, including late arithmetic overflow.

All 512 complex A.91 noise constants were independently extracted from the
published GOST and cross-checked against every value in ISO draft table 1.A.13.
The extraction generator requires local standard HTML/text; ordinary tests
consume saved numeric constants/traces. Four independent 80-digit Decimal
assembly traces cover five frames each, both frame sizes, smoothing enabled
and disabled, envelope offsets, transients, harmonic noise suppression, noise
index wrap and header-reset sine continuity. Other tests check exact sine
phase/parity, initial noise index, checkpoint replay, seek reset and failed
call rollback.

This is standalone complex DSP assembly. The production frame engine must
still manage low/high QMF delays, overlap/tail routing, extension syntax,
rate-derived frequency bounds and AAC PCM scaling, with original encoded
HE-AAC PCM acceptance fixtures. No full HE-AAC playback acceptance is claimed.

### Owned rate-dependent SBR header geometry

`aac_sbr_bands::qmf_bounds` now derives the master start/stop bounds from the
four-bit header fields and the mapped internal SBR rate. It covers all twelve
legacy internal rates (16 through 192 kHz), the six normative offset rows,
13 sorted geometric stop widths, 2*k0/3*k0 stop shortcuts and the 48/35/32
rate-specific bandwidth constraints. `FrequencyTables::from_header` composes
those bounds with existing master/high/low/noise construction. The internal
rate remains twice the mapped AAC core rate in downsampled output mode.

The original published ISO equations resolve translation errors at the 32/64
kHz thresholds, the stop-width exponent and stop index 14. Independently
generated 80-digit Decimal references cover all 3072 rate/start/stop pairs,
including 2376 legal bounds and 5536 complete frequency-table acceptances.
Ordinary tests read the saved JSON offline; neither FFmpeg nor a generator is
invoked. Exact threshold/shortcut and malformed-field tests are included.

`Header::requires_reset` follows the six geometry fields in 4.6.18.3.1;
initialization resets, while amplitude/limiter/interpolation/smoothing changes
alone do not. All 2048 change combinations are covered. Stream owners still
need to reset when their internal sampling frequency changes.

These are protocol/numerical acceptance checks. Full FIL/SCE/CPE payload
assembly, retained coefficient and time-grid history, QMF delays and PCM
scaling, production AAC integration and original encoded HE-AAC playback
acceptance remain incomplete.

### Owned non-scalable SBR SCE/CPE data syntax

`aac_sbr_data::Data::read` composes the owned header-derived frequency geometry,
validated grids, delta flags, inverse-filter modes, envelope/noise Huffman rows
and harmonic masks for non-scalable mono and stereo elements. Coupled CPE
shares its grid/inverse-filter modes and reads level envelope/noise followed
by balance envelope/noise; uncoupled CPE reads both envelopes before both noise
sets. Reserved fields and length-escaped extended-data areas are bounded.
The complete call restores the input reader on any failure. Extended bytes
are retained explicitly, not interpreted as PS or silently decoded.

81 original bit syntax vectors (4587 bytes) exercise all four frame classes,
FIXFIX one/two envelopes, both amplitude modes, both delta directions,
level/balance books, independent/shared channel controls, harmonics and
extension lengths absent/0/1/14/15/16/270. Every bit truncation at each of eight
start offsets is checked; successful reads must leave following bits intact.
An isolated mutation which groups the coupled envelopes ahead of noise fails
on exact coefficient values in case 8, demonstrating order-sensitive coverage.
No foreign encoder/decoder or private media is used, and ordinary tests do
not generate fixtures or invoke FFmpeg/network access.

These are payload syntax acceptance checks, not HE-AAC PCM acceptance. Outer
FIL/header/CRC handling, coefficient history, QMF overlap/scaling and production
AAC integration remain; PS needs its own extension decoder.

### Owned legacy SBR outer extension, CRC and retained headers

`aac_sbr_extension::State::read` now wraps the data parser in non-scalable
`sbr_extension_data` framing. It reads the optional 10-bit checksum, header
presence flag and header, retains the last valid header for headerless frames,
and consumes this element's 0..7 fill bits while leaving following extensions
untouched. It enforces AAC FIL's byte-count/bit boundaries. Header geometry
reset is separate from full stream reset for rate/frame-size/channel changes.
Reader and retained state commit together after all syntax and CRC checks;
reset/seek clears the saved header and format.

CRC is the published zero-initialized polynomial x^10+x^9+x^5+x^4+x+1,
covering header flag through fill bits. The earlier working draft's shorter
CRC and limited coverage are not used. 280 original binary extension vectors
use an independent Python GF(2) polynomial-division oracle instead of the
Rust feedback register. They cover both CRC modes, present/reused headers,
all eight bit offsets, every truncation, following-extension preservation,
checkpoint replay and stream reset. CRC-field and fill-bit mutations must
fail with the specific checksum error and preserve both input and state.

This validates outer syntax and CRC, not decoded HE-AAC PCM. Production FIL
dispatch, retained/reconstructed coefficient history, QMF overlap/scaling
and original encoded HE-AAC playback acceptance still remain.

### Owned SBR cross-frame coefficient history and dequantized parameters

`aac_sbr_history::History::decode` retains the last quantized envelope and noise
row of each channel, reconstructs within/across frames, then dequantizes the
whole mono/uncoupled/coupled frame. Envelope references retain physical band
borders for high/low resolution mapping; noise references use the previous
band index per Q(k,l), including when borders change. Coupled balance doubles
only new deltas. Current grid amplitude resolution controls dequantization;
the published g_E reference retains previous reconstructed quantized values.
Typed dimensions, coupled shared controls and normative noise bounds are
validated before any history commit.

`Stream::read` composes this with the owned extension/header/CRC parser. Input,
retained header and both channels' coefficient history commit together only
after the complete frame dequantizes. Format change/seek resets history;
changing only header amplitude resolution preserves temporal references.
Unknown PS extension bytes remain retained rather than decoded as audio.

12 original four-frame bit sequences (mono and both stereo modes, both core
frame sizes, even/odd high-band counts) compose the actual extension/CRC/data
parser with reconstruction and dequantization. An independent 64-bin expanded
reference checks integer mappings, and 80-digit Decimal checks the final
channel energy/noise values. They exercise header updates/reuse, all grid
classes, fine/coarse changes, within/across-frame high/low mapping, checkpoint
replay and reset. Dedicated typed geometry checks prove noise references
follow indices rather than previous physical intervals. Original five-bit
Q=31 vectors reach the intended normative noise-range refusal after accepted
syntax; their late failure preserves header/history and the following
headerless frame still matches its original expected result.

This is composed bit-syntax/coefficient/DSP coverage, not HE-AAC PCM playback
acceptance. Production FIL dispatch and AAC object-type wiring, complete
cross-frame QMF/time-grid overlap, PCM scaling/synthesis and encoded playback
acceptance remain unfinished.

### SBR cross-frame QMF delay and synthesis row routing

Owned `aac_sbr_buffers` now constructs XLow using tHFGen=8: retained raw
analysis columns use the previous crossover, current columns use the current
crossover, and the rest of the 32-band range is zero. Geometry changes retain
the old analysis tail; full decoder/seek reset clears it.

The non-scalable synthesis-row merger uses tHFAdj=2 and previous-frame
lTemp=2*tE_previous(last)-2*numTimeSlots_previous. It retains six adjusted HF
columns and selects previous kx/end for that overhang, then current geometry
for the remainder. Out-of-range synthesis bands are zero. Startup has zero
overhang/history; calls validate all dimensions and finite values before
committing history. Clone/replay and explicit reset are deterministic.

Four original numeric regressions cover both 960/1024 core frame sizes,
all four overhang lengths, crossover increases/decreases including 1/32,
previous/current HF endpoints, the eight-column analysis history, composed
six-column output latency, and late nonfinite-input rollback. They use tagged
complex columns, independent direct timeline/index expectations, and do not
need FFmpeg, networking or fixture generation. Normative equations were
checked against GOST R 53556.4-2013 section 6.18.5 (published MPEG-4 SBR
translation); this implements non-scalable routing, not scalable bsco.

This closes the standalone frame-buffer/routing gap. Complete SBR frame DSP
composition, production AAC FIL/object-type wiring, PCM scaling, EOF/delay
handling and original encoded HE-AAC playback acceptance remain unfinished.

### Composed SBR analysis / HF / energy / parameter mapping

`aac_sbr_prepare::Preparation` now joins normalized core PCM analysis,
eight-column low delay, inverse-filter/chirp history, owned patch generation,
complex HF generation, envelope energy estimation and dequantized parameter /
harmonic mapping. Analysis columns are scaled by exactly 32768 into standard
16-bit QMF units before prediction and energy estimation. Header geometry
reset rebuilds chirp history while retaining delayed low columns and absolute
harmonic/attack history; format/seek resets the whole preparation state.
Both channels commit together only after every stage succeeds.

The existing 12 original four-frame syntax/coefficient sequences now drive
this complete preparation path with silence and original deterministic PCM,
covering mono, uncoupled/coupled stereo, 960/1024 core frames, changed amplitude
resolution, all time-grid classes, saved-state replay and late second-channel
mapping failure rollback. Four saved direct-time-convolution analysis oracles
independently verify the composed PCM-to-QMF scale, zero startup delay and
crossover masking. No ordinary test runs generators, FFmpeg or networking.

This is preparation for envelope adjustment, not full synthesized HE-AAC
acceptance. A remaining interface discrepancy needs normative resolution:
nonzero limiter modes can remove the upper boundary of a discarded 1/2-band
HF patch tail, while gain limiting requires borders covering the entire M
range. No silent mode substitution or limiter bypass was added. Remaining
work includes resolving that boundary, gain/assembly/synthesis composition,
production FIL/ASC wiring, PCM output/delay/EOF handling and original encoded
HE-AAC playback acceptance.

### Composed non-scalable SBR payload/core-PCM to output-PCM

`aac_sbr_dsp::{Dsp,Decoder}` now composes owned preparation, gain calculation,
limiter/boost, smoothing/noise/sine assembly, previous/current QMF row routing,
and either 64-band or downsampled 32-band synthesis. Normalized core f32 is
converted to standard QMF units and synthesized f64 is divided by 32768, with
no clipping or accidental change in sample gain. Output is channel-planar;
ordinary/double-rate modes yield 960/1024 or 1920/2048 samples per channel.
Core decoder dispatch, interleaving, timestamp/delay compensation and EOF
handling are deliberately still responsibilities to implement in the caller.

`Decoder::read` commits the reader, retained header/coefficients and both
channels' DSP histories together. Seek/format resets clear everything;
header geometry reset retains QMF overlap/synthesis history. Output-rate
changes require a full reset. Nonempty SBR extended audio (including PS) is
explicitly unsupported, not silently rendered as ordinary mono/stereo.

The limiter table now retains the terminal range endpoint as well as the
initial one. This is an explicit resolution of conflicting normative
requirements, not a claimed ISO flowchart erratum: literal Figure 4.40 may
remove the final 1/2-band unpatched tail boundary; corrected Cor.1 4.6.18.7.5
requires k(m) for every 0<=m<M, and the limiter definition specifies coverage
of the SBR range. Protecting that endpoint maintains the selected limiter
mode and supplies the required full partition. 1680 independent Decimal
geometries were updated (1632 acceptance, 48 source-range refusal); a direct
short-tail preparation-to-gain regression reproduces the old missing-boundary
error and now accepts all limiter modes. Real encoded-stream/reference PCM
qualification of this endpoint resolution remains necessary.

32 original three-frame SBR payload-to-PCM traces cover both core frame sizes,
all limiter densities, both smoothing flags, CRC/header reuse, temporal
coefficient history, noise-index wrapping and ordinary/downsampled synthesis.
Their supplied core PCM is silent; the independently computed nonzero output
uses 80-digit Decimal noise gain/boost and a direct time-index synthesis
convolution without production history buffers. The headers select k0=10,
k2=27 and a discarded short HF tail, so the payloads reach actual gain and PCM
acceptance rather than an unrelated unsupported syntax refusal. This proves
composed SBR payload/core-PCM DSP, not an encoded complete AAC/HE-AAC file.
Additional 12 four-frame mono/coupled/uncoupled sequences with original varying
core PCM reach finite output for all time grids; checkpoint/reset, late DSP
failure rollback, output-mode refusal and explicit PS refusal are checked.

Production AAC FIL/ASC integration, original encoded HE-AAC playback/PCM
acceptance, EOF/delay handling and PS/other codec gaps remain unfinished.


### Native AAC SBR packet integration

The extension-aware AudioSpecificConfig parser and native AAC FIL decoder now
compose core LC decoding with owned SBR DSP. Core and output sample rates are
separate, and checkpoints, reset, failure rollback and retained-memory accounting
include SBR state. Original fixtures cover 64 three-packet mono sequences, both
frame sizes and output modes, explicit and sync-extension signalling, CRC failure
rollback and exact PCM comparison against the independent DSP oracle. A short
authored AVC/HE-AAC MP4 also verifies demux and the AAC codec adapter. Ordinary
tests consume checked-in fixtures without FFmpeg or network access.

This is not full player/export HE-AAC qualification: the owned MP4 audio planner
and playback clock integration remain to be completed. Native stereo packet
acceptance, nonzero spectral core reference PCM, EOF/delay, implicit ADTS SBR,
missing-payload upsampling, multi-element/PCE SBR and PS remain unqualified or
unsupported.


### HE-AAC MP4 timelines and Matroska remux

The authored mono 24-to-48 kHz HE-AAC MP4 now reaches the player's native audio
timeline and the owned float32 export path: both preserve 6144 PCM samples and
four seek intervals exactly. The shared MP4-to-Matroska packet planner uses the
output rate and scales the core frame duration (1024 to 2048 output samples),
including delay/padding arithmetic. The Matroska AAC track writer reads the
extension-aware ASC and preserves complete encoded packets. A regression on the
same short authored video reproduces the previous LC-only remux refusal.

This supersedes the earlier outstanding unbounded MP4 decode/seek integration
note; it does not qualify live audio-device playback, bounded SBR export
admission, all edit schedules, Matroska HE-AAC decode, priming/EOF conformance
or encoded nonzero-core reference PCM. These and PS/profile gaps remain open.


### HE-AAC controlled decode and Matroska acceptance

ASC-aware container AAC memory admission now includes a conservative SBR DSP
and transaction reserve instead of rejecting SBR via the LC-only parser. MP4
charges decoder/checkpoint/restore state separately; Matroska charges its own
index and decode state. This estimate covers controlled allocation payload,
not process RSS or allocator overhead. Its buffer geometry is documented in
`owned_aac/stream.rs` and must evolve with supported tools.

The authored synthetic video is remuxed without decoding during the test, then
its Matroska HE-AAC audio is decoded through both player and owned export paths.
Whole PCM, seek and rewind intervals exactly match the original MP4 PCM. A
64 MiB controlled limit accepts both container exports; a 1 KiB limit rejects
before writing. This closes the earlier ASC refusal in controlled MP4/Matroska
decode and supplies Matroska HE-AAC decode acceptance for this mono fixture.

Implicit ADTS SBR, missing-payload upsampling, native stereo packet/reference
qualification, PS, spectral-core conformance, priming/EOF and broader codec
profile coverage remain unfinished. Filtered multitrack export still has an
LC-only AAC duration parser and needs its own targeted regression.


### Native stereo HE-AAC packet acceptance

64 original three-packet LC CPE+SBR sequences now qualify the complete native
stereo decoder for centered coupled and uncoupled SBR. They cover 960/1024
core frames, both output modes, four limiter densities, both smoothing flags,
header reuse, temporal coefficients, CRC failure rollback and checkpoint/reset
replay. The silent CPE core has independently signalled windows. Each channel
uses the existing independent mono Decimal/direct-convolution PCM oracle: the
centered coupled energy/noise split equals the uncoupled mono values. Every
float32 sample must match exactly in both channels.

A short original AVC/stereo HE-AAC MP4 additionally checks both native player
audio and controlled owned export against the corresponding mono fixture.
No production decoder change was necessary for this centered stereo scope.
This closes the earlier absence of native stereo packet acceptance; it does
not qualify asymmetric panning, nonzero spectral cores, shared LC windows,
PS, implicit SBR, missing payload or all encoded-stream conformance.


### Owned pure-upsampling SBR DSP

The delay-only DSP path follows published GOST R53556.4-2013 6.18.5:
all 32 unmasked analysis bands (including retained history) are selected at
XLow(k,l+tHFAdj), with tHFGen=8 and tHFAdj=2, while bands 32..63 are zero.
Analysis and synthesis histories remain transactional across frames. Ordinary
and downsampled synthesis are covered by four original nonzero three-frame
PCM oracles at 960/1024 core samples. Expectations use direct time-index
analysis and synthesis convolution, without production state buffers. Mono
and duplicated stereo, checkpoint/replay, format/NAN rollback and reset pass.

This DSP API is not yet the missing-FIL native AAC fix. Switching from/to
header-bearing SBR must preserve the correct QMF/history state, and requires
a short encoded synthetic video regression before native dispatch is enabled.
The old working draft has different delay constants and was not used for
this delay-only path.


### Native HE-AAC blocks without SBR FIL

Signalled HE-AAC now routes a valid AAC block with no SBR FIL through owned
delay-only upsampling, preserving the configured output clock and QMF history.
The first received SBR header initializes frequency-dependent state without
resetting an already established delay-only pipeline of the same format.
Malformed FIL/core packets still fail transactionally.

Twelve original three-packet sequences cover SBR/no-FIL/SBR, no-FIL/SBR/no-FIL
and all-no-FIL for both core frame sizes and output modes. A short authored AVC
MP4 reproduces the former missing-payload refusal and now decodes 6144 samples
at 48 kHz; full/seek/rewind PCM is checked against a direct-convolution oracle
and across player/controlled export paths. These packet fixtures use a silent
LC core and nonzero authored SBR noise; nonzero core transition/reference PCM,
all grids/overhangs, concealment and implicit signalling remain unqualified.

The first-header regression also feeds nonzero core PCM through the delay-only
DSP before the first SBR header and verifies that retained QMF contributions
are not replaced by a fresh decoder. Reinstating the former native missing-FIL
refusal makes the new encoded fixture test fail with that exact diagnostic;
restoring the fix passes the complete owned media suite.


### Implicit SBR with declared container output rate

An extension-aware output-clock resolver preserves the distinction between
unspecified SBR and explicit SBR=false. The new native constructor accepts a
container-declared double-core output clock for unspecified SBR, initializes
SBR/upsampling state and keeps that clock across packet/checkpoint/reset paths.
MP4 metadata preserves a valid declared dual-rate clock instead of replacing it
with the core-only ASC clock. Player metadata, packet codecs, owned MP4/Matroska
decode, controlled admission and shared remux writers use the same resolution.
Explicit false or an incompatible output hint is rejected; the unhinted raw LC
API remains strict.

Sixteen original mono three-packet sequences, both frame sizes and all limiter/
smoothing choices, match the independent dual-rate PCM oracle exactly. A short
authored AVC MP4 uses core-only ASC (24 kHz) and declared 48 kHz output. It
checks player metadata/decode, controlled export, equivalent explicit SBR PCM,
Matroska remux/decode and seek.

This closes declared-dual-rate implicit container acceptance, not automatic
ADTS detection or implicit core-rate/downsampled discovery. Unhinted streaming
clock negotiation, late detection, implicit stereo/PCE qualification and PS
remain open.


### Transactional unhinted SBR discovery in the native decoder

`NativeAacDecoder::new_with_sbr_detection` accepts unspecified mono/stereo ASC
without a container output hint. A fully valid SBR FIL atomically enables
dual-rate output; failed parsing/DSP leaves the previous clock and state
unchanged. Pre-discovery LC blocks retain QMF analysis/delay/synthesis history
while returning core PCM. Once detected, missing-FIL blocks use upsampling.
Checkpoints preserve discovery policy and output clock; reset returns an
automatic decoder to the undetected state. Explicit SBR=false remains strict.

Sixteen original mono packet sequences match the fixed-clock reference, every
byte truncation of the first SBR block refuses without changing the clock,
and restoring pre-discovery checkpoints recovers the core rate. Delayed
discovery after a valid silent LC block retains the same QMF contributions
as the fixed-clock decoder. Original ADTS framing of the paired synthetic
video's packets is decoded through the new discovery API and matches its PCM.

The high-level ADTS streaming exporter/player now negotiates one output clock
before publishing PCM, including late mono/stereo SBR discovery. It spools
the selected prefix to a bounded disk stream and replays that prefix through
the fixed-clock decoder; replay does not double-count source progress. Original
delayed/stereo ADTS and paired MP4 fixtures cover fractional ranges, packet
limits, memory admission, WAVE headers, resampling, plans and loudness exports
(`tests/native_adts_sbr.rs`). Implicit downsampled signalling, PS and broader
codec gaps remain open.

### Owned parametric stereo syntax (2026-10-08)

`owned_aac::aac_ps_data` and `aac_ps_huffman` implement the normative PS
payload syntax with all six IID/ICC modes: 10/20/34 bands, both IID quantizers,
both ICC mixing modes, 5/11/17 IPD/OPD bands, frequency/time deltas, retained
headers, fixed/variable 0-4 envelopes, and nested escaped extension lengths.
Mode fields survive disabled tools. Failed/truncated parsing commits neither
reader position nor header state. SBR extension ID 2 routes to PS; the other
SBR IDs consume the remaining fill area as specified in appendix A.

All 242 normative words have an independent saved protocol oracle, exhaustive
bit-truncation tests, and prefix/Kraft checks. Twenty-eight authored sequences
cover 24/30/32 QMF slots and header continuity. Two original short MP4 videos
contain explicit and implicit PS, with valid LC/SBR syntax and an independently
verified second-packet CRC. `tests/he_aac_ps.rs` accepts their PS syntax and
reproduces the exact pending synthesis refusals. Generation is offline and
separate from ordinary tests; it uses no foreign codec or FFmpeg.

This closes PS syntax, not HE-AACv2 playback. Native-band reconstruction and
dequantization are implemented below. Global parameter-band mapping, hybrid
filters, decorrelation, interpolated complex stereo mixing, PCM synthesis and
stereo geometry/implicit PS negotiation still need implementation and stereo
PCM acceptance references. The native player continues to refuse PS explicitly
until those stages are implemented.

### Owned PS native parameter history and dequantization (2026-10-08)

`owned_aac::aac_ps_history` reconstructs IID/ICC signed indices and IPD/OPD
modulo-8 indices within and across envelopes/packets. It preserves the original
native grids for zero-envelope reuse, even when a new header changes modes;
disabled tools use zero-index defaults. Later time rows address available raw
previous-band indices (otherwise index zero), independently of later global
parameter-band mapping. Frequency-coded first rows are required after a mode
change. Header/dimensions/codebook ranges/reconstructed grids are checked even
for manually constructed public frames. Slot-count changes require reset.
Current retained mode fields are exposed separately from the old physical
grids, including a no-envelope mode change followed by tool disable. The later
common-band stage must select the current configuration, not infer it from an
old retained grid or lose it when the enabled header field becomes absent.

Startup becomes ready only after a header with nonzero envelope count and
independent first rows for every enabled tool. Clone checkpoints replay the
same parameters; reset clears both syntax and numeric history. The composed
PS `Stream` commits input position, header and numeric history together, with
rollback on a numeric failure in a later block of the same SBR extension.
Repeated phase extension assignments use the final transmitted parameter rows.
`aac_ps_dequant` exposes normative IID dB, ICC coherence and phase radians,
with independent saved Decimal/Machin phase references for all grid values.

Twenty original varying-target sequences cover all six modes and 24/30/32 QMF
slots, alternating time/frequency rows, phase wraps, startup, disabled/re-enabled
tools, no-envelope reuse and resolution changes. Expected indices are authored
absolute targets before encoding, not decoder-generated references. Five new
short MP4 videos cover varying valid PS and invalid coarse/fine IID or ICC
indices made from otherwise legal Huffman words, plus mode-change/disable with
no new envelopes. SBR syntax/CRC and PS syntax
are checked before asserting the exact numeric refusal. All generation is
explicit and offline, with no FFmpeg, foreign codec or private source media.

These tests establish native parameter recovery, not PS stereo PCM. The old
and varying valid videos retain explicit pending-synthesis refusal checks until
real hybrid/decorrelation/mixing/synthesis and container stereo negotiation are
implemented. The common-band mapping stage below now supplies the separate
parameter geometry required before mixing; native values alone cannot replace it.

### Owned PS common-band mapping (2026-10-08)

The owned mapper selects common 20/34 bands, treating a disabled tool as
20 bands and preserving the previous configuration when both are disabled.
It maps IID/ICC integer indices before dequantization with truncation toward
zero, pads lower IPD/OPD phase grids, and maps real mixing coefficients with
floating weights. Numeric protocol tables also describe 71/91 hybrid subband
bindings and the starred complex-coefficient conjugations. Zero-envelope
configuration changes do not manufacture parameter envelopes: retained real
mixing coefficients must be transferred separately when DSP is connected.

Offline regression oracles cover 286 integer, 120 real coefficient, 24 phase,
and 144 native parameter cases, all 20 saved history sequences, transactional
mapping/reset, and seven original three-packet MP4s with mixed resolutions
and disabled-tool selection. These are mapping acceptance tests. Full hybrid
filtering, decorrelation, phase smoothing, stereo mixing/synthesis and PCM
acceptance remain unfinished; native PS playback still refuses explicitly.

### Owned PS real matrices and phase primitives (2026-10-08)

`aac_ps_mixing` computes Ra/Rb h11/h12/h21/h22 from validated quantized
IID/ICC, including the normative Rb coherence floor 0.05 and modulo-pi/2
angle correction. It derives matrices from validated common-band envelopes.
Phase primitives smooth ordered oldest/previous/current IPD and OPD phasors
with weights 1/4, 1/2, 1, rotate left/right columns and optionally conjugate
all coefficients for starred hybrid bindings, then applies them to mono and
decorrelated complex signals to produce separate left/right outputs. The history owner must map
phase history to current geometry and reset it at required configuration
transitions; this pure stage does not invent temporal state.

An offline 90-digit Decimal oracle covers all 736 coarse/fine IID x ICC x
Ra/Rb matrices. Rb uses independent algebraic eigenvectors instead of the
production trigonometric path. All 512 phase histories are checked separately
for IPD and OPD (1024 rotations), with conjugation and invalid-input checks.
Energy, channel intensity ratio and coherence invariants also verify matrix
orientation. The seven saved mixed-resolution MP4 regressions now verify the
real matrix stage after container/SBR/PS/history/common-grid processing.

This completes these numeric primitives, not native PS playback. Stateful
phase history, temporal interpolation, hybrid filters, decorrelator, final
stereo QMF synthesis and independent full PCM acceptance remain necessary.

### Owned PS temporal matrix interpolation (2026-10-08)

`aac_ps_interpolation` now owns chronological 24/30/32-slot common-grid
complex coefficient interpolation and the retained final boundary. The first
segment follows n/n0, interior segments follow (n-ne)/(ne1-ne), border zero
applies its endpoint immediately, and the last endpoint is held to the frame
end. Zero-envelope frames repeat the retained matrices. Startup/reset uses
zero matrices. The state commits only after complete input validation and
successful output construction; clones provide replay checkpoints.

Configuration transfer is explicit: the surrounding DSP must map retained
real h_ij and rebuild the complex boundary after phase reset, then replace
the temporal owner's common geometry. This stage never reinterprets old
native indices or invents envelopes. Interpolation avoids an overflowing
difference between opposite finite endpoints and accepts finite extremes.

Thirty-six independent exact-Fraction cases cover both common grids, all
three slot counts, startup boundaries, interior ramps, shortened tails and
reuse. Two original three-packet MP4s (Ra/20 and Rb/34) have border zero,
shortened tails, retained headers/CRC and a final zero-envelope packet. They
verify actual MP4→SBR→PS history→common mapping→real/complex matrices→temporal
processing against saved Decimal endpoint/Fraction timing oracles. Additional
checks cover exact startup/next-frame anchors, checkpoint replay, reset,
explicit reconfiguration and transactional rejection of malformed input.

This is temporal matrix acceptance; native HE-AACv2 PCM remains unimplemented.
The surrounding matrix controller, hybrid filters, decorrelator, stereo QMF
synthesis and full independent PCM acceptance remain required.

### Owned PS two-position phase history (2026-10-08)

`aac_ps_phase_history` now retains ordered oldest/previous common-grid IPD
and OPD across envelopes and frames, derives new smoothed complex endpoints,
and routes/conjugates endpoints for the 71/91 hybrid bindings. A new disabled
phase envelope uses zero parameters while preserving the preceding position.
Zero new envelopes do not advance parameter positions; a 20/34 configuration
change resets both positions even when the frame carries no new envelopes.
Processing is transactional across all envelopes, including late numeric
validation failures. Clones provide replay checkpoints and reset clears state.

Unready parameter frames are refused without advancing physical history. A
ready repeated frame cannot bootstrap a fresh physical owner: it requires
its checkpoint/preroll instead of silently manufacturing zero history.
The first ready frame initializes at its actual current grid even if an earlier
unready header already changed the parser's grid. Later frames enforce common
grid continuity. An original three-packet 34-band startup video covers this
case separately from the ordinary ready-grid transitions.

Independent authored position traces and Decimal matrix/rotation references
cover all 20 saved native sequences and seven mixed-resolution videos. Four
new original MP4s cover two-envelope phase history around a repeated frame,
phase disable/reenable, a grid change without envelopes, and delayed 34-band
startup. Tests traverse MP4/SBR CRC/PS native history/common mapping/phase
history and all starred hybrid bindings. They retain the explicit full-native
PS synthesis refusal; only endpoint/state acceptance is established here.

The matrix controller below now transfers retained real h_ij and applies the
no-envelope retained-complex versus unrotated-real policy from 6.4.6.5 before
the temporal owner. Hybrid filtering, decorrelation, stereo QMF synthesis and
full independent PCM acceptance remain unfinished.

### Owned PS matrix controller and hybrid-signal mixing (2026-10-08)

`aac_ps_matrix_controller` composes native-parameter common mapping, ordered
phase history, retained real h_ij, complex boundaries and temporal processing.
A 20/34 change remaps the retained real coefficient vectors with floating
weights, resets phase and rebuilds the unrotated complex boundary; it never
remaps complex H or re-dequantizes old indices through the new native mode.
Without new envelopes, enabled phase retains the actual previous-frame H,
while disabled phase selects unrotated retained real h_ij (6.4.6.5). Reenabling
phase without new parameters retains that actual boundary rather than
resurrecting a stale earlier rotated endpoint.

The controller outputs full chronological common-grid matrices and routes
71/91 hybrid bindings with starred coefficient conjugation. Given original
mono/decorrelated hybrid inputs, it computes separate left/right complex
hybrid signals. Parameter processing is transactional across every contained
state; process_and_mix also rolls back all state on a late signal error.
The first accepted frame fixes the 24/30/32 slot epoch until explicit reset.
Unready and fresh-owner repeated frames retain their precise refusal gates.

All 20 saved native sequences now compare EVERY common-band coefficient at
EVERY QMF slot and retained real/complex/phase snapshots to independent
Decimal endpoints, normative floating grid weights and Fraction timing. Eleven
existing videos and two new three-packet MP4s additionally compare EVERY
left/right hybrid sample to independent outputs for original rational signals.
The new videos cover phase off/on without new envelopes and 20→34→20 retained
real-coefficient transfers across different quantizer/mixing modes. Binary64
reference rows are deduplicated offline; tests consume saved assets only.

This completes the numeric matrix controller and supplied-hybrid mixing stage.
Hybrid analysis, decorrelation and stereo QMF synthesis are still missing
from native PS playback; valid videos keep the full synthesis refusal until
those stages and full independent PCM acceptance are implemented.

### Owned PS raw hybrid FIR primitives (2026-10-09)

`aac_ps_hybrid_filter` implements all five 13-tap low-frequency prototypes
from GOST R 53556.8-2013 6.4.3, tables 36–38: 20-band Q=8 Type A and Q=2
Type B, and 34-band Q=12/8/4 Type A. Complex modulation uses the positive
exponent and half-bin offset; the two-band real cosine has no half-bin offset.
A causal stream owner retains all 13 history samples across arbitrary chunks,
checkpoint replay and empty calls, and commits only after every output is
finite. Raw synthesis sums split bands without an extra FIR or normalization.

Independent saved Decimal convolution references cover every raw subband of
real/imaginary impulses and an original complex rational sequence. Tests also
verify six-slot delayed reconstruction, published decimal precision, arbitrary
chunk boundaries, reset/checkpoints and rollback on late overflow. Generator
execution is separate from ordinary offline tests and uses no FFmpeg or codec.

This is the raw modulation-index layer, not the complete routed hybrid bank.
20-band folding/order, the 71/91-band bank with aligned upper QMF channels,
decorrelation, stereo QMF synthesis and whole-PCM startup/timeline integration
remain to be implemented and independently qualified. No full PS playback
acceptance or PCM output is claimed; existing native synthesis refusals remain.

### Complete owned PS QMF-domain hybrid bank (2026-10-09)

`aac_ps_hybrid` composes the FIR primitives into the full 71/91-band analysis
and inverse synthesis topology. Twenty-band QMF 0 routes raw modulation bins
6,7,0,1,(2+5),(3+4); the QMF 1 two-way real split routes 1,0 and QMF 2 routes
0,1 (SP-040428 figure 8.3). Thirty-four-band splits retain their per-channel
raw index order (figure 8.5). QMF 3..63 or 5..63 are delayed by six slots.
Inverse synthesis adds every routed subband back into its QMF channel,
without normalization, extra filtering or signal conjugation (6.4.7).

The stream owns shared raw 13-slot history for all 64 QMF channels. A 20/34
change evaluates the new configuration on actual previous input, including
previously unsplit channels 3/4; it does not zero histories or reinterpret
retained filtered outputs. Empty calls change neither history nor grid.
Clone/replay and explicit reset are supported. Processing commits the grid
and history only after the entire block, including folding, is representable.

Six independent saved Decimal full-convolution cases compare all routed
subbands and all 64 synthesized QMF channels. They cover both configurations,
complex impulses/rational inputs, 32-slot 20→34→20 and 30-slot 34→20→34
transitions. A previously authored three-packet MP4 with CRC on its middle
SBR packet now drives actual MP4/SBR/PS native state and the matrix controller
into the bank. Its mono/decorrelated matrix path can consume actual analyzed
hybrid input; zero diffuse input in this component test is not a decorrelation
or PCM oracle. Full native PS refusal remains independently checked.

Raw FIR and complete hybrid-bank numerics are now accepted. Decorrelation,
stereo QMF synthesis wiring, one-time PCM/QMF startup compensation, native
AAC/SBR integration and full independent stereo PCM acceptance remain open.

### Owned PS decorrelation and complete stereo QMF component path (2026-10-09)

`aac_ps_decorrelation` implements the full 20/34-grid decorrelation stage from
GOST R 53556.8-2013 6.4.5, tables 39–43. Lower 30/50 hybrid bands use a
two-slot fractional phase delay and three allpass links with delays 3/4/5,
frequency-dependent feedback and normative center frequencies. Upper bands
use 14-slot or 1-slot delays. Feedback retains raw, unattenuated outputs.
Transient detection sums complex power by the normative parameter bindings,
tracks decaying peaks and two smoothers, and applies the shared parameter-band
ratio to every hybrid sample. Silence has gain one; a representable ratio
never requires forming an overflowing 1.5*difference intermediate.

Grid changes reset all decorrelator state while the upstream hybrid bank
retains raw QMF history. Frame controls implement full reset after an absent
preceding PS element and filter-history clearing above exclusive generated
QMF limit kx+M (A.3). Partial clearing retains lower-band histories and the
shared transient state. Empty calls have no effects. The entire frame,
including control resets and late energy/output errors, commits atomically.

Saved references independently construct closed-form geometric allpass impulse
responses and direct convolution, with transient quantities calculated from
full weighted histories rather than the Rust streaming recurrences. Eleven
cases cover silence, both-grid complex impulses/bursts, 32/30-slot grid
changes, missing-PS full reset, and partial clearing above QMF 16. Every
sample, raw output, power, gain and retained transient vector is compared.
Unattenuated complex-impulse energy is also checked independently. Tests
cover arbitrary chunking, checkpoint replay, reset and failure rollback.

The original three-packet grid-retain MP4 now drives native MP4/SBR CRC/PS
parameters into hybrid analysis, actual decorrelation, stereo matrices and
inverse hybrid synthesis. Independent references compare BOTH channels at
EVERY hybrid/QMF sample, including starred mixing coefficients. The provided
QMF input is original numeric data, not decoded PCM. This accepts the complete
QMF-domain component path but leaves full native PS refusal in place.

Stereo QMF-to-PCM synthesis wiring, startup/timeline compensation, a combined
transactional PS DSP owner, native AAC/SBR packet integration and independent
whole stereo PCM acceptance still remain. Full HE-AAC v2 playback is not yet
claimed; this milestone does not close the broader codec goal.

### Aligned transactional PS QMF-to-stereo PCM DSP (2026-10-09)

`aac_ps_dsp` composes retained matrices, hybrid analysis, decorrelation,
complex stereo mixing, inverse hybrid sums and two owned QMF synthesizers.
Six QMF lookahead slots compensate hybrid delay. Subsequent calls verify the
retained overlap and consume it only once. Both output rates (64 and 32 PCM
samples per slot), synthesis histories and all upstream states commit together.
Changing output rate requires reset. PCM inherits the supplied QMF units;
native AAC normalization and packet/startup/timeline integration remain pending.

Eight original numeric cases cover 24/30/32-slot frames, 20/34 grid changes,
phase toggles, retained parameters and zero EOF lookahead. The explicit Python
generator uses Decimal FIR/direct allpass convolution and weighted transient
histories, independently saved matrix coefficients, and direct QMF synthesis
contributions. Ordinary tests consume saved references without FFmpeg/network.
Both PCM channels and every QMF sample pass comparison; checkpoint replay,
reset, overlap rejection and late-error rollback pass. Existing synthetic MP4
packets provide native parameter parsing; encoded full PS playback remains
refused. This is component acceptance, not full HE-AAC v2 conformance.

### Owned streamed SBR/PS stereo PCM bridge (2026-10-09)

`aac_sbr_qmf_dsp` is now the shared preparation/adjustment/routing stage used
by ordinary SBR PCM synthesis and the new PS bridge. Its 16-bit-unit QMF rows
are exposed before synthesis; existing mono/stereo, 960/1024, upsampling,
header/history, rollback and normalized PCM references remain accepted.

`aac_sbr_ps::Decoder` owns SBR syntax/history, native PS parameters, the shared
QMF stage, full PS DSP and one queued frame. First input returns no PCM;
subsequent input renders the prior frame with six actual future QMF slots.
`frame_index` identifies the original input frame, not the later packet that
supplied lookahead. EOF renders the final frame with zero lookahead once;
repeated EOF returns None, and new input requires reset. All readers, history,
queue and DSP states commit together. Output is normalized unclipped f64.
Rate/slot/output-mode changes require reset. Missing/multiple PS elements remain
explicit refusals; native AAC dispatch and startup/timeline trimming remain open.

Four original three-packet synthetic MP4s cover 960/1024 silent native LC cores,
30/32 PS slots, phase retention/toggles and 20/34/20 grid changes. Actual SCE
bits extracted from the authored packet are decoded by the native AAC-LC core;
the corresponding original SBR payload is then supplied to the bridge. Numeric
QMF references independently use normative noise and Decimal envelope gain.
Full stereo PCM references use direct allpass convolution, weighted transients,
independent matrix coefficients and direct QMF synthesis contributions. Both
output rates compare every QMF/PCM value, replay/reset, actual lookahead and EOF.
The 960 MP4 wrapper uses correct 1920-tick packets and 5760-tick duration.
CRC, malformed PCM, missing PS and format failures reproduce specific errors
without changing reader position or pending state. All generation is explicit,
offline and separate from tests; original 1024 wrappers remain byte-identical.

This accepts the streamed SCE/core + SBR/PS bridge, not the combined native AAC
packet API or general HE-AAC v2 conformance. Nonzero-core independent PCM,
missing/late PS, native buffering/checkpoints/memory/timestamps and seek still
require qualification before removing the native PS refusal.

Validation: 829 tests passed offline (root lib 347, owned media lib 432,
11 HE-AAC integration suites 50); one existing owned test remains ignored.
The player build check, rustfmt, Python syntax and diff checks passed. Seven
new/current reference assets regenerate identically, and both original
1024-sample PS MP4 wrappers remain byte-identical.

### Native complete AAC-LC/SBR/PS packet API (2026-10-09)

`fvid::codec::aac_ps_native::NativePsAacDecoder` now accepts complete original
raw_data_blocks, without extracting SCE bits or supplying core PCM externally.
It uses the canonical owned channel parser, spectral/noise/TNS reconstruction
and IMDCT/window synthesis, then the transactional SBR/PS bridge through the
owned bounded FIL reader. ID_END and remaining-byte validation occur before
committing any core/extension/queued state. Both explicit AOT29 and sync-extension
SBR/PS signalling are accepted for one mono AAC-LC core and single/double output
rate. The API returns interleaved finite f32 stereo with original `frame_index`;
initial decode returns None, subsequent decode emits the prior packet's frame,
and `finish()` emits the last frame once. Output sample rate/mask/channels are
explicit. Opaque checkpoints include core overlap/noise, SBR/PS, pending frame
and EOF status; another core/output format is rejected without state changes.

Four original MP4 packet sequences qualify 960/1024 core samples, 30/32 PS
slots, both rates and both signalling forms (16 cases). Every stereo PCM sample
matches the independently saved SBR/PS reference after final f32 conversion.
Replay, reset, EOF restoration, frame identities, incompatible checkpoints and
all strict packet truncations are covered. Six short original malformed MP4s
reproduce trailing bytes, missing fill, duplicate SCE, a late element after the
SBR/PS DSP advanced, fill before SCE and truncated fill. Each test asserts its
specific error and verifies that the next valid packet and EOF match the saved
baseline, including core/extension and queued PCM. Eight newly generated assets
regenerate byte-identically; ordinary tests neither generate nor access network
or FFmpeg.

These fixtures use silent native LC cores with nonzero authored SBR noise.
Independent nonzero-core PS PCM conformance remains open. The general
`NativeAacDecoder`/production factories still refuse PS until they can carry
delayed frame identity, EOF, startup trimming, checkpoints/memory and seek
correctly. This packet API does not by itself qualify production playback or
close missing/late PS, coupling/PCE or other codec/profile gaps.

Validation: 832 tests passed offline (root lib 347, owned media lib 432,
12 HE-AAC integration suites 53); one existing owned test remains ignored.
The player build, formatting, Python syntax and diff checks passed. The native
PS regression binary links only libiconv/libSystem, with no FFmpeg/libav codec
library. All eight new native packet fixture assets regenerate identically.

### Delayed PS playback timing adapter and bounded thread stack (2026-10-09)

`fvid::codec::aac_ps_playback::PsAacDecoder` associates each input packet's
signed source timestamp and own presentation duration with its native frame
identity. Returned `DecodedFrame` carries complete interleaved PCM together
with original `source_pts`/`source_duration`, rather than metadata of the later
lookahead packet. Negative preroll timestamps remain signed for container edits;
the PCM packet timestamp follows existing nonnegative playback conventions.
EOF, opaque checkpoints and reset retain/clear the pending timing consistently.
Container sample rate/channels must agree. Codec failures poison the adapter
until reset or a valid checkpoint restore; incompatible restores preserve state.
The adapter does not trim PCM itself: presentation uses the returned source window.

Two short original MP4s provide unequal packet durations for 960/1024 cores.
Tests verify actual MP4 PTS/duration, every stereo PCM sample, delayed identity,
EOF replay, reset/preroll, signed source metadata and recovery after late packet
failure. Generator support for unequal `stts` entries preserves prior uniform
wrappers byte-identically. The worker still needs to consume these source windows,
drain EOF before range resets/termination, and carry timing in cached checkpoints
before the production factory can route PS through this adapter.

The adapter tests exposed stack overflow on the normal 2 MiB test/playback stack.
The same original video/PCM cases passed on 8 MiB, confirming stack pressure
rather than an unrelated media refusal. Native PS now keeps its large fixed
SBR/QMF/PS histories in a Box, reducing checkpoint/transaction temporaries on
stack without sharing mutable histories. Native packet and timing tests pass on
2 MiB; a dedicated explicitly sized 2 MiB thread runs complete timing/recovery
regressions even when an ambient test runner uses a larger default stack.

This accepts the timing adapter and stack regression. Production worker/factory,
startup trimming, full player seek/edit scheduling, nonzero-core independent PS
PCM and other codec/profile gaps remain open.

Validation: 836 tests passed offline (root lib 347, owned media lib 432,
13 HE-AAC integration suites 57); one existing owned test remains ignored.
The dedicated 2 MiB thread regression, player build, formatting, Python syntax
and diff checks passed. Three timing assets regenerate identically, and all
four prior 960/1024 PS wrappers remain byte-identical.

### Production player PS factory, source windows and EOF/edit scheduling (2026-10-09)

The player `make_audio_decoder` factory now selects the owned PS playback
adapter when ASC explicitly signals PS (including sync-extension signalling).
Ordinary AAC retains its current adapter. `AudioDecode::decode_packet` carries
PCM with the original signed source PTS/duration; immediate codecs have a
compatible default, while PS returns queued metadata. `finish_packet` drains
EOF. Typed PS checkpoints include queued timing and reject cross-codec restores.

The audio worker now applies packet-tail trim/container presentation to that
original source window, and drains the decoder before declaring Ended. MP4
edit completeness is checked after delayed PCM presentation. This allows the
last queued source packet to satisfy a range before its reset or stream end.
Completed ranges/silence do not receive an out-of-range lookahead frame at EOF;
actual remaining source gaps still produce `AAC edit extends beyond source
packets`. Existing immediate-decoder, PCM-step and reset paths are preserved.

Original unequal-duration MP4s accept production factory decoding with correct
trimmed PCM/source PTS and last output before Ended. Four additional original
edited MP4s cover silence, repeated source ranges, an offset source window,
960/1024 cores and deliberate source gaps. The 960 range needs PCM from the
last queued source packet before the next reset. End-to-end worker tests compare
continuous presentation timestamps and every byte to independently qualified
native source PCM sliced by the authored edit windows. Seek into both repeated
ranges and trailing silence, seek to zero, rewind and checkpoint restoration
preserve the exact expected presentation suffix. Gap cases prove the specific
remaining-source error AFTER all three native PCM packets have drained.
All five new worker assets regenerate identically; ordinary tests run no
fixture generation, network or FFmpeg.

This accepts the tested production MP4 PS player/factory/worker path, not full
HE-AAC v2 conformance. Independent nonzero-core stereo PCM, general startup
trimming, unhinted PS, coupling/PCE, export/other-container routing
and other codec/profile gaps still remain. The legacy immediate native AAC API
continues to refuse PS; delayed callers use the explicit PS API/adapter.

Validation: complete player-feature library suite passed 905 tests with 23
existing ignored tests. All 13 HE-AAC integration suites passed 57 tests with
player features; no-player core library passed 347 tests. Player build, Python
syntax and diff checks passed; all five worker fixture assets regenerate
byte-identically. These counts describe separate feature/configuration runs.


## Declared PS with absent and late elements

The owned SBR/PS bridge accepts a mono SBR frame without a PS element and
maps its QMF rows to both synthesis channels. Hybrid input history advances
through these frames; matrix and phase history retain the last PS element.
Returning PS resets decorrelation when the previous frame lacked PS. Separate
left/right synthesis tails remain continuous and settle to dual mono.

Eight original three-packet MP4 fixtures cover all-mono, late PS, missing-middle
and trailing-mono at 960/1024 core samples. `he_aac_ps_absence` compares every
PCM sample against independent synthesis/decorrelation references at both core
and double output rates, plus packet identity, EOF, checkpoint and reset.
Generation is explicit and offline; tests do not invoke FFmpeg or network.
Dedicated startup acceptance now covers headerless PS, a zero-envelope header
and a temporal first envelope at the unchanged initial mode. Each stays dual
mono until a subsequent independent header starts stereo. Missing whole SBR FIL, unhinted discovery, nonzero-core
startup trimming and export/other-container routing remain separate gaps.

Validation: owned media library 432 passed / 1 existing ignored; player library
905 passed / 23 existing ignored; all 14 HE-AAC suites 58 passed. The 11 new
assets regenerate byte-identically; Python syntax, Rust format and diff checks pass.


## PS startup acceptance before independent headers

Six more original three-packet MP4s exercise headerless PS, an enabled header
with zero envelopes, and a same-mode temporal first envelope. The stream remains
uninitialized in the first frame; all normalized PCM samples match independent
dual-mono synthesis. The next independently coded header starts stereo and the
last packet retains it. Both 960/1024 frames and core/double output rates are
covered, with exact packet count, nonzero mono, distinct stereo, checkpoints,
reset and EOF. The extended absence suite covers 14 videos / 28 rate cases.
A temporal envelope changing modes is not used as a startup reproducer because
it triggers the separate frequency-coded mode-transition requirement.


## PS Matroska worker and negative AAC presentation preroll

The Matroska player factory already routes declared PS through the delayed
owned decoder. Two original AVC+PS MKVs now qualify that route at 960/1024,
including late PS, original nanosecond timestamps, EOF drain and seek/rewind.
Their first AAC block starts at -10 ms. Before the fix, the player clamped its
PTS to zero while presenting all PCM (1920 instead of 1440 samples for 960).
AAC presentation now removes only negative preroll samples at time zero,
using ceil sample conversion and the delayed frame's original source PTS.
Positive seeks keep the existing block-aligned floor: earlier blocks are
discarded, including their tails from quantized timestamps. Codec histories still consume every preroll packet. Surviving samples
match independent stereo PCM and preserve both channels and synthesis tails.
This changes player presentation only. Matroska delayed export is qualified
below; owned MP4 export still needs delayed PS integration.

Validation: full offline player library 906 passed, 23 existing ignored.
The three Matroska assets regenerate byte-identically; Python syntax and
diff checks pass. The targeted regression also validates native AVC playback.


## Owned declared-PS Matroska PCM and WAV export

Matroska timeline decoders dispatch explicitly declared PS to the owned delayed
PS decoder. Pending source packet metadata supplies PTS and DiscardPadding for
the returned PCM. EOF and an accepted-input packet limit drain the pending
frame once, without reading/counting an extra source packet. Immediate PCM,
ALAC, LC/SBR AAC and Opus retain immediate behavior; the f64 PCM instantiation
uses the same timeline. Root native reader/export geometry also recognizes PS
and uses the delayed interface, while legacy immediate PS PCM calls refuse it.

Two original AVC+PS MKVs at 960/1024 core frames use non-overlapping timestamp
gaps. They reproduce the old exact PS synthesis refusal. Acceptance compares
every PCM sample to independent normalized stereo references, including gap
silence, ceil intervals, accepted two-packet EOF tail and full three-frame EOF.
The existing export policy normalizes the first source timestamp to presentation
origin, separately from player preroll at absolute zero. Root native reader PCM
matches owned output byte-for-byte. Public `decode_audio` produces WAVE with
matching rate, stereo layout and presentation sample count.

Optional controlled admission adds eight fixed PS state-copy reserves plus
four MiB for bounded hybrid/matrix/QMF/stereo vectors and their capacity growth.
The existing SBR/core/index/packet/track estimates are retained. Acceptance
checks a 64 MiB admitted export and a 10 MiB rejection before PCM output.
These are caller-selected test limits, not an unconditional player cap.

Validation: target export acceptance passed; full owned media library 432 passed
and 1 existing ignored, full offline player library 906 passed and 23 existing
ignored. Three new assets regenerate byte-identically; generation is offline
and separate from tests. MP4 PS exports are qualified below; unhinted PS, nonzero-core startup PCM
and other codec/profile gaps remain unqualified.


## Owned MP4 delayed PS PCM/WAV export with edits and checkpoints

MP4 export dispatches declared PS to the owned delayed decoder. Typed AAC/PS
checkpoints retain decoder histories and queued PCM together with pending source
window metadata in the timeline checkpoint. Restoring a repeated source range
therefore preserves the previous packet's PTS/duration as well as its lookahead.
Both root native reader and owned media export use this timeline; f64 PCM
retains its immediate decoder. PS state is heap-owned; aggregate MP4 admission
charges the existing decoder/checkpoint/restore estimate including PS reserves.

Source EOF and an accepted-input packet limit drain the pending frame. A selected
range that ends before source EOF reads one real lookahead packet when required,
presents only the original queued window, and drops out-of-range queued output
at the next range reset. Container windows can trim complete decoded packet
padding even internally; unequal-window acceptance checks every PCM sample.

Two new original short AVC+PS MP4s present silence, a source range reaching the
last queued packet, a repeated range and trailing silence. They reproduce the
old exact PS synthesis refusal. Acceptance compares full independent stereo PCM,
interval selection, accepted three-packet tail, controlled admission, root reader
byte equality and public WAVE export. Existing authored unequal-window MP4s
add full EOF and early intervals requiring real lookahead beyond selection.

This qualifies the tested declared-PS MP4 export route, not full HE-AAC v2
conformance. Nonzero-core PS startup, unhinted discovery, missing whole SBR FIL,
coupling/PCE and other codec/profile tools remain separate work.

Validation: three MP4 export regressions passed, including exact invalid-source
refusal after all pending EOF PCM. Matroska export acceptance still passed.
Full owned media library 432 passed / 1 existing ignored; full offline player
library 906 passed / 23 existing ignored. Three new MP4 assets regenerate
byte-identically; Python syntax, Rust formatting and diff checks pass.


## Native in-band PS candidate API with unspecified ASC signalling

`NativePsAacDecoder::new_with_in_band_ps(asc, output_rate)` accepts mono
AAC-LC or SBR ASC whose PS flag is unspecified. It negotiates a caller-supplied
core/double output clock and reuses the owned full packet parser and delayed
SBR/PS pipeline. `ps_detected()` reports actual accepted PS elements, including
startup elements before an independent stereo header; it is sticky until reset
and is retained by checkpoints. A malformed later packet rolls back the flag
together with core/extension/pending histories. Candidate EOF without any PS
element rejects the candidate instead of classifying ordinary mono AAC as PS.
The normal AAC decoder still accepts the corresponding mono SBR stream.

The strict `new` API remains for explicitly signalled PS. Candidate construction
honors explicit PS=false or SBR=false; checkpoint compatibility includes this
EOF/presence policy. This is the native decoding/discovery API, not automatic
container probing. Container dispatch must inspect payload and negotiate layout
before publishing stereo metadata; automatic unhinted MP4/Matroska routing is
still separate work, as are missing whole SBR FIL and nonzero-core PS startup.

Four original short AVC+AAC MP4s omit PS signalling from LC or explicit SBR ASC
and carry late PS in the original authored payload. Before the fix they reproduce
the exact explicit-signalling constructor refusal. Acceptance covers eight
960/1024 and core/double output combinations against every independent stereo
PCM sample, actual presence transitions, delayed frame identity, checkpoint
replay/reset/EOF, malformed trailing-byte rollback and explicit-disabled tools.
A no-PS candidate failure is a candidate-refusal test, separate from valid
unhinted-PS native acceptance and normal mono AAC acceptance.

Validation: all 17 HE-AAC suites passed 64 tests, full owned media library
432 passed / 1 existing ignored, full offline player library 906 passed / 23
existing ignored. Five new assets regenerate byte-identically; Python syntax,
Rust formatting and diff checks pass. No test runs generation or codec tools.

### 2026-10-09 — sole-element PCE SBR acceptance

The owned native SBR decoder now accepts a normal front PCE with one tagged SCE
or CPE and no AAC coupling elements. PCE does not require indexed configuration
1/2 to use the existing mono/stereo QMF pipeline. Tags, in-band PCE layouts and
checkpoint configuration identity remain checked. Thirty authored cases match
independent PCM for 960/1024 and 24/48 kHz; explicit and sync SBR are qualified,
as is container-hinted implicit double-rate SBR. Centered coupled/uncoupled CPE
SBR is included. Eighteen MP4 videos qualify PCM, intervals, WAV, decoder factory
source windows and actual implicit payload discovery. Four malformed videos
retain exact tag/layout refusals with state rollback. The blanket PCE/SBR refusal
is superseded only for these sole-element layouts. Multi-element/coupled AAC
PCE with SBR, height layers and implicit downsampled SBR remain open.

Full fixture and validation details are recorded in `scripts/NATIVE_VALIDATION.md`.

### 2026-10-09 — independent SBR state for multiple AAC elements

The sole-element SBR restriction above is superseded by per-element syntax and
QMF/DSP state. A FIL follows its SCE/CPE and is read with that element's channel
count; canonical PCM routing remains determined by the configured layout.
State is included in checkpoint/reset, rollback and retained memory reporting.
One hundred independent PCM cases cover two SCEs, PCE/indexed 5.1, reordered elements,
missing FIL on one pair a mixed normal/top PCE and indexed height configuration 14. Sixty MP4 videos qualify
export, intervals, WAV speaker masks and playback decoder factories. A CRC error
in a later element preserves previous histories and rolls back earlier parsed extensions. AAC
coupling with SBR and multi-element PS remain open; this is not universal codec
profile qualification. See `scripts/NATIVE_VALIDATION.md` for fixture evidence.

### 2026-10-09 — dependent AAC coupling with SBR

The blanket coupled-PCE/SBR refusal above is superseded for dependent CCE.
Nonzero spectral coupling before/after TNS now reaches target IMDCT and SBR;
80 cases and 48 synthetic MP4 videos qualify mono/stereo, both frame sizes,
core/double output clocks, target FIL present/absent and explicit/sync/hinted
implicit signalling. Core PCM is independently computed; SBR composition uses
the separately qualified own DSP. Missing-target videos verify exact refusal
and rollback. Independent coupling with SBR and CCE-owned SBR FIL remain open,
as do other unqualified AAC profiles/tools. Details: `scripts/NATIVE_VALIDATION.md`.

### 2026-10-09 — independent AAC CCE and its own SBR FIL

Independent CCE/SBR admission is superseded by per-tag CCE SBR state and final
PCM mixing after target SBR. 240 authored long-window cases and 144 acceptance
videos cover both frame sizes, mono/stereo, one/two CCEs, sparse/high tags,
changing wire order, present/absent/missing CCE FIL, core/double clocks and
explicit/sync/hinted implicit signalling. CCE states join checkpoint/reset,
transactional rollback and controlled-memory admission. The original refusal
expectation is updated; malformed video regressions retain exact error checks.
Dependent CCE FIL, coupling with PS and further profiles/tools still need
qualification. PCM evidence and validation are in `scripts/NATIVE_VALIDATION.md`.

### 2026-10-09 — dependent CCE-owned SBR FIL

The dependent CCE FIL refusal above is superseded by per-tag syntax/header/CRC
and temporal coefficient history. Dependent CCE stays in the spectral path;
its FIL does not produce another PCM contribution. 160 cases and 96 acceptance
videos require exact PCM equality to the qualified no-CCE-FIL baseline, covering
both coupling points, frame sizes, target layouts and output clocks, including
missing CCE FIL and actual in-band discovery. Four CRC/PS videos retain explicit
refusals with transactional rollback; those PS checks are not PS acceptance.
Coupling with PS and further profiles/tools remain open. Detailed evidence is
in `scripts/NATIVE_VALIDATION.md`.

### 2026-10-09 — AAC coupling with target PS

Owned PS decoding now admits dependent CCE before/after target TNS and
independent CCE with its own mono SBR, mixed into the target SCE left channel
after PS. Packet-indexed pending PCM follows PS lookahead and EOF draining;
checkpoint/reset and failed-packet rollback include CCE histories.
108 authored long-window cases and 54 synthetic acceptance MP4s cover
960/1024 samples, three coupling points, sparse tags, changing wire order,
missing FIL, output clocks and explicit/in-band signalling. Three malformed
or unsupported videos check missing targets, source CRC and source PS refusal.
Core PCM uses an independent cosine oracle; full output is stage composition
with the previously qualified owned SBR/PS DSP, not an independent full decoder.
Root/owned export, repeated ranges, WAV and playback seek/rewind are tested.
CCE-owned PS and multi-element target PS remain unqualified. Fixture generation
is separate and offline; ordinary tests need neither FFmpeg nor network.

Delivery checks: core library 908 passed / 23 ignored; owned media library
432 passed / 1 ignored. Commands used locked offline builds; no FFmpeg.

### 2026-10-09 — PS discovery belongs to the target SCE

Twelve additional own synthetic videos cover 960/1024 and coupling points
0/1/3 with CCE-owned mono SBR FIL. Without target PS, the syntax probe stays
false, the PS candidate refuses EOF specifically for missing PS, and ordinary
owned AAC playback/export accepts mono. With target PS first appearing in
packet one, negotiation accepts stereo and preserves preceding core history
and delayed PCM. These are acceptance tests for already implemented behavior,
not a claim of CCE-owned PS support. Root/owned PCM agrees; the expected PCM
uses the independent core oracle and qualified own SBR/PS stage composition.
Full playback, rewind and seek agree with export. Fixture generation remains
offline and deterministic; tests do not generate fixtures or invoke FFmpeg.

### 2026-10-09 — sequential PS elements in one extension area

The former at-most-one-PS refusal is superseded for multiple PS elements
inside one mono target SBR extension area. Syntax and native parameter history
advance in wire order; the final parameters drive one QMF/PS synthesis step.
This matches the extension-area loop in the reference parser:
https://raw.githubusercontent.com/FFmpeg/FFmpeg/master/libavcodec/aacsbr_template.c
No reference implementation is linked or invoked by production or tests.

Twelve authored 960/1024, core/double-clock, explicit/implicit cases contain
a first coarse 10-band PS followed by a distinct fine 34-band PS. PCM must
equal a single-final-PS baseline using independent core PCM and the separately
qualified owned DSP. Six MP4 acceptance videos cover root/owned export, ranges,
WAV, playback, rewind and seek. A reserved IID mode in a later PS element
requires exact refusal and transactional rollback; it is not an acceptance.
Multi-element target audio programs and CCE-owned PS remain separate gaps.

### 2026-10-09 — sequential PS temporal history

Twelve further sequential-PS cases (960/1024, core/double clock, three ASC
signallings) transmit nonzero positive/negative temporal IID/ICC and modulo-8
IPD/OPD deltas. The second PS depends on the first in the same extension area;
following packets retain both updates. Assertions check every native band index
against scalar accumulated values before PCM comparison to separately encoded
absolute-parameter packets. Six own MP4 videos qualify export/ranges/WAV and
playback seek/rewind. This extends qualification of the sequential-PS fix; it
does not introduce a new production implementation or close other AAC profiles.
The generator's default behavior preserves the existing PS fixture bytes.

### 2026-10-09 — AAC gain-control syntax and empty adjustments

The channel reader no longer refuses every gain-control flag. A shared owned
parser reads max_band, adjustment counts, levels and locations for OnlyLong,
LongStart, EightShort and LongStop. Syntax storage is bounded by wire fields
(3 bands, 8 windows, 7 adjustments), and a truncated input commits no cursor.
Empty lists are a no-op and now proceed to spectral decoding. Active lists
retain a specific unsupported-synthesis error; SSR inverse PQF and actual gain
compensation remain gaps. This is not full SSR/gain-control acceptance.
The field widths are checked against the primary reference parser:
https://raw.githubusercontent.com/FFmpeg/FFmpeg/master/libavcodec/aac/aacdec.c

32 own long-window videos cover 960/1024, LC/PS, target/CCE and max_band 0..3.
PCM equals no-gain baseline exactly, with independent cosine core PCM for LC;
root/owned export, repeated ranges and playback rewind/seek agree. Two active
adjustment videos reproduce the precise refusal and preserve overlap/history.
Separate syntax tests cover all window modes, maximum lists and every byte
truncation. Fixture generation is deterministic/offline and outside tests.
Container clock overrides were added to the own MP4 generator; existing default
fixture bytes are unchanged.

### 2026-10-09 — AVC end-of-sequence/end-of-stream NAL admission

The owned AU preparer and stateful decoder now admit NAL types 10/11 as
non-VCL markers. They do not participate in slice coverage or fabricate
a decoded picture. Decoder output ordering remains caller-owned; playback
drains its existing B-frame queue at container EOF. Reference parsing also
classifies these as non-VCL:
https://raw.githubusercontent.com/FFmpeg/FFmpeg/master/libavcodec/h264dec.c

Six short synthetic MP4s append sequence, stream or both markers to the final
AU of the existing authored eight-frame two-slice I/P/B fixture, repacked at
2/4-byte NAL lengths. Before the fix both prepare and software playback failed
exactly with unsupported in-band AVC NAL. Acceptance checks compare each
decode-order picture to the unmodified baseline and full display-order YUV
to the saved oracle after rewind/seek. Marker-only packets return no picture,
and explicit decoder reset permits replay. Fixture generation is deterministic
and requires neither FFmpeg nor network. These cases do not qualify auxiliary
slices, scalable profiles or other still-unsupported AVC reconstruction tools.

### 2026-10-09 — HEVC opaque reserved/unspecified non-VCL admission

Validated base-layer non-VCL NALs beyond the known parameter/SEI/marker types
no longer abort the complete AU. Types 41..63 are opaque to base-picture
decoding; they do not add slices, POC changes or fabricated pictures. Header
validity and unsupported-layer checks still run. This follows the reference
decoder's unknown-NAL dispatch behavior:
https://raw.githubusercontent.com/FFmpeg/FFmpeg/master/libavcodec/hevc/hevcdec.c
This is not decoding of future payload semantics, Dolby Vision RPU/EL processing
or multilayer/3D profiles, which remain separate gaps.

Six own short MP4s contain all 23 non-VCL types before/between/after the two
slices at 2/4-byte NAL lengths. Source is the existing authored three-frame
HEVC multislice fixture; config, timestamps/edits and visual samples are
preserved. Before the change, decode and playback fail specifically with
unsupported HEVC NAL type 41. Acceptance compares POC/output flags/planes
to the unmodified baseline and complete playback to saved YUV through rewind
and seek. Opaque-only packets produce no picture. Three malformed videos
require exact forbidden/temporal-bit or unsupported-layer refusals and explicit
reset before replay. Offline fixture generation is separate and deterministic.

### HEVC separate colour planes: owned reconstruction and references

The authored one-frame `hevc-separate-colour-planes-pcm-synthetic.mp4`
contains independent first slices for colour planes 0/1/2. Its parameters and
entropy payload derive only from our monochrome PCM fixture. Individual slice
headers parse and each payload reconstructs the saved monochrome pixels when
decoded with a monochrome SPS view. The old exact slice-order refusal has
become acceptance: the AU collector keeps per-plane slice order, reconstructs
each plane as monochrome and assembles full-resolution 444 samples. References
project to the corresponding plane's pixels, motion and reference POC maps.
Separate-plane retained storage includes all plane states. Completed sample
and availability buffers are shared by the assembled picture and its plane
states; assembly does not copy their contents. Mutation detaches shared
buffers, preserving held reference pixels. Accounting charges shared buffers
once and includes vector capacities and Arc control/Vec headers. Pointer-sharing,
mutation isolation, and held-frame stability after reset have direct tests.
This establishes shared pixel storage, not a measured 60 fps result.
Reordered planes, a three-frame reference/WPP video, reset/rewind, and distinct
PCM plane pixels have acceptance coverage. Missing/repeated planes retain
specific refusal tests. All seed parameters and samples are our synthetic data.
The fixtures do not qualify all separate-plane profiles, depths or tools.
Generate offline with `python3 scripts/generate_hevc_colour_plane_fixtures.py`;
ordinary tests only consume the checked-in video and YUV seed.
Generate the inter/WPP case separately with
`python3 scripts/generate_hevc_colour_plane_fixtures.py hevc-pcm-mono-reference-wpp-rext8.mp4 hevc-separate-colour-planes-reference-wpp-synthetic.mp4`.

The distinct inter-plane regression extends this to three visibly different
reference planes. Only the authored first-frame PCM sample blocks change;
subsequent predictive payloads remain intact. Plane order rotates as 2/0/1,
1/2/0 and 0/2/1 across the three AUs. Saved YUV is derived by the explicit
PCM sample transforms, not by decoding the new stream. Acceptance checks
non-I headers with nonempty reference lists, every plane's pixels after reset,
and full 444 playback after rewind and seek. This qualifies reference pixel
isolation; distinct per-plane motion histories and wider profiles remain to
be qualified.
Mutation verification replaced the plane-indexed reference projection with
plane 0 for every plane: the new test failed specifically at frame 1, plane 1.
The production projection was restored before the final regression run.
Generate with
`python3 scripts/generate_hevc_colour_plane_fixtures.py hevc-pcm-mono-reference-wpp-rext8.mp4 hevc-separate-colour-planes-distinct-reference-wpp-synthetic.mp4 distinct-reference`.

Separate-plane PCM now has 10/12-bit acceptance qualification as well. Four
own one-frame videos retain full-depth PCM syntax: baseline copies and distinct
planes containing nonzero least-significant bits. Their expected YUV comes
from the saved mono fixture and explicit PCM sample transforms. Tests compare
every 16-bit sample and packed 444 byte after reset, rewind and seek, and
require hvcC bit-depth fields to agree with the signalled SPS fields. The
generator preserves the SPS-signalled chroma-depth field rather than silently
replacing it with luma depth. These are intra PCM tests; their own scope does
not qualify inter, distinct motion histories or the remaining profiles/tools.
Use `hevc-pcm-mono-full10-rext10.mp4` / `hevc-pcm-mono-full12-rext12.mp4`
as generator inputs and `hevc-separate-colour-planes-full10-synthetic.mp4` /
`hevc-separate-colour-planes-full12-synthetic.mp4` as outputs. For distinct
low-bit samples, use output `hevc-separate-colour-planes-distinct-full10-synthetic.mp4`
or `hevc-separate-colour-planes-distinct-full12-synthetic.mp4` and the third
argument `distinct-depth`.

High-depth inter now has separate acceptance coverage in
`hevc_separate_colour_planes_high_inter`: three own videos derived from
`hevc-monochrome-filtered-rext10`, `hevc-monochrome-wpp-rext12` and
`hevc-monochrome-parallel-rext12`. It requires predicted pictures with active
references and nonzero SAO offsets, compares every plane against saved mono
YUV, and checks full packed 444 playback through reset/rewind/seek. The 128x96
case requires multiple entropy substreams, so WPP qualification goes beyond
an enabled PPS flag. These three components deliberately have identical
pixels/motion histories; distinct high-depth plane histories remain a gap.
Generate each using the existing generator with the corresponding mono MP4
input and output `hevc-separate-colour-planes-filtered10-synthetic.mp4`,
`hevc-separate-colour-planes-wpp12-synthetic.mp4` or
`hevc-separate-colour-planes-parallel12-synthetic.mp4`. Ordinary tests consume
checked-in video/YUV only and do not invoke a generator or external codec.

The high-depth target also covers a fourth, tiled 12-bit synthetic video from
`hevc-monochrome-mixed-tiles-rext12`. Each colour plane contains multiple
independent slices and dependent segments. The generator now walks complete
AU headers, inserts plane IDs after the address syntax in independent slices,
and preserves dependent NALs unchanged so their plane identity/context inherit
from the preceding segment. Acceptance requires tiles/dependent syntax and
independent/dependent headers for every plane, then compares saved pixels
through decoder reset and playback rewind/seek. Source samples and parameters
remain our authored fixtures. These plane histories are still identical;
distinct histories and remaining codec profiles/tools are not qualified here.
Generate with
`python3 scripts/generate_hevc_colour_plane_fixtures.py hevc-monochrome-mixed-tiles-rext12.mp4 hevc-separate-colour-planes-mixed-tiles12-synthetic.mp4`.


### HEVC sequence-end state

EOS/EOB NALs now validate trailing bits and temporal layer zero and close
the sequence after the accompanying picture. The next CRA starts with a fresh
DPB/POC state and suppresses leading RASL pictures. The own synthetic
`hevc-eos-before-cra-*` fixtures reproduce the formerly emitted RASL picture
and malformed markers; `tests/hevc_eos.rs` checks acceptance and exact errors.
Generate separately with `python3 scripts/generate_hevc_eos_fixtures.py`;
ordinary tests require neither FFmpeg nor network access. This qualifies the
base-layer sequence transition, not remaining HEVC profiles or multilayer tools.

The EOS acceptance target additionally verifies standalone EOS/EOB packets,
repeated end markers, reset/replay, and poison-until-reset recovery after
malformed EOS. Subsequent CRA pictures are compared against a fresh decoder,
including every reconstructed plane; the three leading RASL pictures must be
discarded. These checks reuse our synthetic fixture and require no generation
or external codec during test execution.


### MP4 output durations across suppressed pictures

The EOS synthetic video also reproduced a player timing failure: the picture
before the post-EOS CRA ended at tick 3584 while the next displayed picture
started at 5120. A decode-only RASL sample had been used as its endpoint.
MP4 reordering now uses future sample PTS only as a sorting bound and waits
until the next actual output group is complete before deriving duration.
This adds output lookahead inside the existing bounded reorder queue. The
regression checks all retained sample indices, saved-source pixels, exact PTS,
continuous intervals, EOF and rewind; native playback additionally checks seek.
The fixture is our existing short authored `hevc-eos-before-cra-valid-synthetic`
video, not private media. Ordinary tests remain offline and generation-free.

EOS native seek qualification now probes 20 positions on both sides of output
frame boundaries, including the interval extended across suppressed RASL, in
alternating forward/backward order. It requires the requested time to lie in
the sequentially derived interval and compares both that exact interval and
pixels; matching some output frame alone is not sufficient.


### HEVC future SPS/PPS extension data

Reserved nonzero `sps_extension_4bits` / `pps_extension_4bits` now cause the
remaining opaque extension data to be consumed rather than refusing the base
picture. Known range/SCC syntax is still parsed first and RBSP termination
is still validated. Multilayer and 3D extensions remain unsupported.
H.265 section 7.4.3 requires decoders to ignore these future data flags:
https://www.itu.int/rec/dologin_pub.asp?id=T-REC-H.265-202108-S%21%21PDF-E&lang=e&type=items

Three authored `hevc-future-extension-{sps,pps,both}-synthetic.mp4` videos
reuse our short Main IPB seed with only configuration parameter tails changed.
Before the fix the SPS case failed with the exact unknown-extension refusal.
`tests/hevc_extension_tails.rs` requires every retained picture, PTS, duration
and EOF to match baseline through rewind. A fourth malformed-stop fixture
requires bounded parsing failure. Generate separately using
`python3 scripts/generate_hevc_extension_tail_fixtures.py`; neither generation
nor ordinary tests need FFmpeg or network. This is future-tail compatibility,
not implementation of the remaining layered or 3D coding tools.


### HEVC VPS parsing reaches production decode

VPS parsing previously existed only in helper/tests: the decoder ignored
configuration and in-band VPS RBSP bodies. The authored
`hevc-future-extension-vps-bad-stop-synthetic.mp4` reproduced malformed VPS
being accepted at player open. The decoder now invokes owned VPS parsing for
both paths. Annex A base decoding ignores extension data without enabling
INBLD; the parser retains base-layer checks and validates RBSP termination.
https://www.itu.int/rec/dologin_pub.asp?id=T-REC-H.265-201612-S%21%21PDF-E&lang=s&type=items

The extension-tail generator additionally produces valid/invalid VPS cases.
Acceptance compares 17 output pictures, exact timestamps/durations and rewind
with the authored Main IPB seed. Invalid configuration fails specifically on
RBSP exhaustion; in-band validation tests refusal, poison-until-reset and
first-picture recovery. No private media, FFmpeg, generation or network is
needed during ordinary tests. Cross-parameter VPS/SPS identity/ordering
consistency and actual multilayer/3D decoding still require further work.


### HEVC VPS identity and availability

The decoder now retains parsed VPS by ID, applies in-band VPS packets to
parameter state and restores initial VPS sets on reset. At picture activation
the SPS must resolve its named VPS; SPS temporal sublayers/nesting must also
agree with that VPS. Changed VPS after VCL follows the existing parameter
update ordering refusal. The wrong-ID authored fixture previously decoded
despite its SPS naming absent VPS 0; it now refuses specifically at activation.
A paired nonzero-ID fixture preserves all 17 pictures, timing and rewind.
A parameter-only packet supplying missing VPS 0 enables decoding; reset removes
that update and recovery can supply it again. Generate these using the existing
offline extension-tail fixture script.

Activation availability follows H.265 7.4.2.4.2; temporal relationships follow
7.4.3.2.1. This milestone qualifies ID binding and in-band availability; it does
not prove full CVS activation restrictions, every temporal hierarchy, layered
profiles or 3D decoding.
https://www.itu.int/rec/dologin_pub.asp?id=T-REC-H.265-201911-S%21%21PDF-E&lang=e&type=items


### HEVC active VPS remains stable inside a CVS

An authored `hevc-vps-change-inter-synthetic.mp4` changes the active VPS level
from 30 to 60 before sample 1 while keeping all original pictures/parameters
otherwise. Before this fix player lookahead accepted the changed VPS and
continued dependent reconstruction. Decoder state now retains the last active
VPS and refuses a changed binding inside the sequence. IDR/BLA and a sequence
restart after EOS provide a new CVS boundary; reset forgets activation.

Acceptance tests additionally deliver the changed VPS as a parameter-only
packet before IDR and before the existing own EOS-to-CRA fixture. Both are
compared with a fresh decoder for POC and every pixel, repeated through reset;
the CRA case still discards its three leading RASL pictures. Initial higher
VPS level in configuration also preserves all baseline frames and timing.
The generator is offline and tests consume authored files only. BLA and SEI
activation need separate qualification; this does not claim every CVS rule or
layered profile implemented. H.265 7.4.2.4.2 defines active VPS lifetime:
https://www.itu.int/rec/dologin_pub.asp?id=T-REC-H.265-201802-S%21%21PDF-E&lang=e&type=items


### HEVC BLA starts a fresh reference buffer

The own `hevc-bla-w-lp-synthetic.mp4` changes the first CRA header of the
authored open-GOP seed to BLA_W_LP while preserving slice bits and picture
samples. Its legal leading RASL pictures must be suppressed. The decoder
previously kept reference pictures with POC 0, 4 and 2 after BLA POC 8; a
private-state regression reproduced that exact DPB failure. IRAP pictures with
NoRaslOutputFlag now clear prior stored references before constructing RPS
lists, including BLA even when parsed RPS syntax lists prior unused pictures.

Acceptance compares all 14 retained pictures with the unchanged authored
source, exact sample timestamps and continuous intervals through rewind, and
checks native seek at 20 forward/backward boundary positions. VPS update
acceptance also includes BLA followed by its own remaining inter pictures.
Generate separately using `python3 scripts/generate_hevc_eos_fixtures.py`;
ordinary tests use checked-in own video only. BLA_W_RADL/BLA_N_LP, SEI activation
and layered tools remain separate qualification work. H.265 reference marking:
https://www.itu.int/rec/dologin_pub.asp?id=T-REC-H.265-202601-I%21%21PDF-E&lang=f&type=items


### HEVC active_parameter_sets SEI

Owned prefix-SEI payload type 129 parsing now reads bounded VPS/SPS IDs and
self-contained/no-update declaration flags. A maximum of 16 SPS IDs is allowed;
base-layer decoding binds the first and preserves extra IDs as guidance.
The decoder stages a valid parameter-only SEI until its associated picture,
checks the declared IDs against that picture's VPS/SPS binding and validates
new activation at a CVS boundary. Repeated guidance for already active sets
is permitted. `active_parameter_sets()` exposes the last validated declaration
within the current CVS; reset or a new CVS without guidance clears it.

Own `hevc-active-parameters-*` short videos reproduce previously ignored
wrong-VPS/SPS declarations and qualify valid/repeated/extra-ID guidance by
comparing 17 pictures, POC and exact playback timing against the authored seed,
including decoder reset. The empty and mixed-message cases are malformed
guidance refusal/opaque-playback tests, not activation acceptance: malformed
SEI does not cost the accompanying picture under the existing metadata policy.
Unit tests require SPS-count/ID bounds, payload alignment and the requirement
that active-parameter SEI occupy its own NAL. Generate separately using
`python3 scripts/generate_hevc_active_parameter_fixtures.py`; tests are offline
and do not invoke generation, external codecs or network.

The flags are exposed as declarations; full self-contained/no-update promise
verification, configuration-carried activation, multilayer layer_sps_idx and
other SEI tools remain work. H.265 D.2.21/D.3.21 and 7.4.2.4.2 are the syntax
and activation basis:
https://www.itu.int/rec/dologin_pub.asp?id=T-REC-H.265-201911-S%21%21PDF-E&lang=e&type=items


### Configuration-carried HEVC activation guidance

The decoder now reads valid `active_parameter_sets` prefix SEI from hvcC
arrays as well as packets. Initial guidance is retained separately, staged
for the first picture and restored on reset without publishing it as an
activated declaration before a picture is validated. In-band guidance can
override the pending value without mutating the initial configuration.

Own `hevc-active-parameters-config*` videos place the SEI exclusively in the
configuration and leave every packet/picture unchanged. Before the fix valid
configuration guidance disappeared and wrong-VPS guidance was ignored. Tests
now require exact picture/POC/timestamp/duration parity through reset, specific
wrong-binding refusal, and restoration of the original declaration after an
in-band override. The existing offline generator produces these cases; ordinary
tests invoke no generator, external codec or network. Full declaration-promise
validation and multilayer activation remain explicit gaps.


### HEVC reserved SEI payload-extension data

`active_parameter_sets` parsing now ignores reserved payload-extension bits
after its known fields while retaining final marker/alignment validation.
H.265 D.3.1 requires decoders to ignore this future data. Previously such a
tail made the whole valid declaration disappear, and a wrong-VPS declaration
with the same tail was silently ignored instead of triggering binding refusal.
The two own `hevc-active-parameters-extension-tail*` videos reproduce both
specific failures. Acceptance retains the declaration, all 17 pictures/POC and
exact playback timing through reset; the invalid binding gives its exact error.
A unit test verifies known fields survive opaque extension bytes and a missing
final marker is still a bounded parser error. Generate separately using the
existing active-parameter fixture script; ordinary tests need no FFmpeg,
generation or network. This qualifies the active-parameter payload, not all SEI
messages or layered syntax.
https://www.itu.int/rec/dologin_pub.asp?id=T-REC-H.265-201304-S%21%21PDF-E&lang=e&type=items

### 2026-10-09 — owned AAC SSR synthesis building blocks

The shared owned IMDCT now admits the SSR 256/32 coefficient geometries,
verified against dense direct cosine sums and individual basis vectors. New
`aac_ssr_gain::SsrGainOverlap` implements bounded gain interpolation in log2
space, previous-fragment compensation, and gain-controlled overlap for all four
window sequences. It emits 256, 368, or 144 quarter-rate rows as specified,
rather than forcing transition blocks into LC geometry. Nonempty gain tests
cover all three controlled bands and long/start/short/stop history. Invalid
locations, levels, window counts, and geometry leave output/history unchanged.

`aac_ssr_ipqf::SsrIpqf` implements the four-band 96-tap synthesis filter with a
24-row polyphase history. Nonzero streaming input and every band impulse match
an independent literal zero-insertion/convolution oracle; irregular packet
boundaries, checkpoint recovery and reset are covered. Prototype coefficients
and equations come from ISO/IEC 14496-3:2009 sections 4.6.12.3.1–4 and Table
4.164, not a foreign codec implementation:
https://csclub.uwaterloo.ca/~pbarfuss/ISO14496-3-2009.pdf

These are synthesis building blocks, not SSR playback acceptance. The packet
pipeline still rejects active gain and AOT 3. Quarter-band spectral ordering,
SSR windows, appropriate band tables/TNS, and ASC/native-decoder integration
remain required, followed by an independently qualified synthetic AOT 3 video.
The existing active-gain refusal fixtures remain refusals. Ordinary tests do
not fetch the standard or invoke FFmpeg.

### 2026-10-09 — AAC SSR raw packets and MP4 playback acceptance

AOT 3 configuration now reaches the owned SSR synthesis, with frameLengthFlag
required to be zero. Spectra retain the standard 1024/128 scalefactor geometry;
four 256/32-line transforms reverse the even one-based PQF bands. SSR-specific
TNS limits follow Table 4.157. The first frame uses its own shape on both window
halves; later frames retain the previous shape. Gain, overlap, IPQF and window
state participate in packet rollback, checkpoints and reset, including retained
allocation accounting. PS paths explicitly reject SSR instead of treating it
as an LC core. Existing LC active-gain refusal remains unchanged.

`scripts/generate_aac_ssr_fixtures.py` creates eight original short MP4 videos
with mono/stereo, sine/mixed-KBD shapes, empty/active gain in every controlled
band, and long/start/short/stop transitions. It writes a separate scalar oracle:
direct cosine IMDCT, gain interpolation/overlap and literal zero-insertion FIR
convolution. Spectra are nonzero in all four PQF bands. `tests/aac_ssr.rs`
compares every decoded sample (2e-7 absolute tolerance), verifies the MP4 packet
bytes, root/owned PCM export, exact variable output extents, rewind, seek and
repeated ranges. LongStart produces 1472 samples and LongStop 576, so the
container's authored stts matches the actual synthesis timeline. A ninth video
refuses invalid transition gain location 14 and checks complete state rollback.
The original AOT 3 configuration refusal is reproduced by restoring the prior
LC-only guard before running the new acceptance test; it fails for that exact
reason. Generation and ordinary tests stay separate, offline and FFmpeg-free.

Qualification is for these 24 kHz fixtures, not all SSR encoders/profiles.
SSR coupling, SBR/PS, independently switched channel windows with unequal output
extents, and dedicated ADTS discovery/profile integration remain explicit gaps.
Full AAC profile parity (Main/LTP/ER/ELD/USAC) is still not achieved.

### 2026-10-09 — SSR dependent and independent coupling acceptance

SSR no longer unconditionally refuses CCE. Existing dependent spectral coupling
runs before/after target TNS; SSR also requires the dependent CCE window shape
to match its target. Independently switched CCE uses a separately retained owned
SSR filterbank (including its own shape/gain/IPQF history), then scales/adds PCM.
The four-bit CCE tag domain has 16 lazy boxed synthesis slots. Checkpoints,
rollback, reset and retained heap accounting include those states. A failed
later CCE restores both already-synthesized target and earlier CCE histories.

`generate_aac_ssr_coupling_fixtures.py` authors 14 short MP4 acceptance videos:
mono/stereo, points 0/1/3, active/empty gain, all window sequences, alternating
CCE/target packet order, distinct independent source/target window shapes, and
simultaneous tags 1/15. All four source PQF bands are nonzero. Expected PCM is
computed by the separate scalar SSR oracle on source or target spectra; ordinary
tests neither regenerate assets nor invoke FFmpeg. Three original failure
videos cover missing target, dependent shape mismatch and invalid gain in a
later independent CCE. Each refusal checks the exact error and state rollback.
The prior blanket CCE guard makes the new acceptance test fail for its intended
unsupported-tool reason; the fixed path passes PCM/export/seek/rewind/ranges.

This qualification does not prove all coupling gain-list/TNS interactions.
Independent source/target windows with unequal output extents still explicitly
refuse until PCM alignment is implemented; SSR SBR/PS and ADTS remain gaps.

### 2026-10-09 — bounded SSR PCM alignment primitive and video reproductions

`aac_ssr_alignment::SsrPcmAlignment` queues independently produced mono lanes
and their per-chunk output gains. It emits the previous complete target frame
with the caller's original stamp after one packet of lookahead. Each lane is
bounded to two maximum-sized SSR blocks; arbitrary cumulative drift and
incomplete stream ends return precise errors. It preserves per-source gain
changes even where source and target frame boundaries differ. Data/gain/geometry
validation, scaled and mixed f32 overflow, clone checkpoints and reset are
transactional. Retained allocation payload can be inspected independently of
allocator headers or caller buffers.

Tests cover every sample in ahead/behind/opposite window patterns, 1024 legal
transition steps across all 16 initial state pairs, gain changes across source
chunk boundaries, numeric overflow in a later lane, and incomplete EOF recovery.
`generate_aac_ssr_alignment_fixtures.py` adds six original MP4 videos (three
coupled streams and three standalone sources). The standalone native SSR output
feeds the alignment primitive and agrees with the separate scalar PCM oracle
for all 6144 samples while retaining target frame stamps. Ordinary tests read
committed assets; they do not generate media or invoke FFmpeg/network access.

This is not native playback acceptance for unequal windows. The coupled streams
still reproduce `AAC SSR independent coupling window extents require alignment`
with exact state rollback. The native MP4/root export acceptance test is ignored
with an explicit integration reason and was run separately to confirm that
precise refusal. It must be enabled after native delayed-frame dispatch, source
PTS/checkpoints and EOF draining are integrated. A passing refusal or primitive
test does not close that remaining codec gap. SSR SBR/PS and ADTS remain separate.

### 2026-10-09 — native SSR alignment, source timing and final-frame drain

Integrated `SsrPcmAlignment` into both native AAC instantiations. Independently
switched output channels and independent CCEs synthesize their own lengths; the
first unequal extent activates one packet of lookahead. Before that point,
existing equal-extent SSR packets retain immediate output. Alignment persists
until reset and keeps per-source gain chunks, original signed packet PTS and
source duration. The full queue and pending source duration belong to opaque
checkpoints, rollback, reset and retained-payload accounting. Timed callers use
`decode_timed`; untimed `decode` returns an empty vector for a consumed delayed
packet and must be drained with `finish`. This is not malformed-packet success.

Root playback now returns the original source window through `AudioDecode` and
flushes the final original frame at EOF. Owned/root MP4 and Matroska dispatch
recognize delayed AAC; ADTS streaming drains it and excludes SSR from LC/SBR
clock-discovery caching. The former ignored MP4 acceptance is enabled and the
old exact-refusal expectation has become delayed-frame acceptance.

The five original coupled videos (source ahead, source starts ahead, source
behind, late source behind and opposite switches), plus an original stereo CPE with independently switched channels,
compare all 6144 sample frames against the scalar oracle. Root and owned full
exports agree; playback verifies checkpoint replay, rewind, seeks at 1100,
2600, 5900 and exact EOF, repeated interval exports and terminal drain. A new
short unfinished video reaches the precise `SSR PCM alignment incomplete at
stream end` error, leaves its queued samples and timestamps untouched, and can
continue after the rejected finish. No private media or codec parameters are
used, and ordinary tests require no generator, network or FFmpeg.

Qualification remains scoped to the authored 24 kHz mono/stereo switch schedules
and container durations. Changing independent CCE rosters while a queue is
active still refuses explicitly; SSR SBR/PS, other rate/profile coverage, general priming/discard-padding
qualification and changing channel/coupling topology remain separate gaps. This
does not establish complete AAC or codec parity.

Fixed-clock extension of the same regression: four MP4 variants use the usual
1024-sample AAC packet duration. Before the fix their stop windows reached
`MP4 audio packet duration disagrees with decoded samples` (verified failing
acceptance, not an unrelated parse error). Timed native SSR now selects the
standard access-unit clock when a transition requires it, checkpoints that
choice and continues using it through final packet trimming. A late-transition
video with a 600-sample final duration previously reached `SSR PCM alignment
incomplete at stream end`; its enabled acceptance compares all 5720 selected
sample frames with the untrimmed scalar oracle prefix. Raw untimed synthesis
retains its variable block API. Authored variable-duration fixtures continue
passing as well.

Four original AVC/AAC Matroska variants and a stereo SSR ADTS stream reuse the
same own packets. Root and owned full and 10–150 ms interval exports are byte
identical to MP4, including EOF drain. No external codec executable creates or
reads them. A separate original video removes an independent CCE after queue
activation and tests the precise remaining roster-change refusal and complete
rollback; that is a refusal test, not playback acceptance for changing rosters.

Final local checks for this milestone: root `cargo test --offline --features
media,player --lib --test aac_ssr --test aac_ssr_coupling --test
aac_ssr_alignment --test aac_gain_control` passed 909 core, 4 gain, 3 SSR,
3 coupling and 8 alignment tests (23 unrelated core tests remain ignored).
Owned media library passed 448 tests with one existing ignored test. These are
1375 successful test executions, not 1375 independent codec profiles. The
media-only offline build passed; regeneration reproduced all 26 alignment
fixture files byte-for-byte. The new playback acceptances are enabled.

### 2026-10-09 — independent SSR CCE first occurrence during alignment

The earlier blanket roster-change refusal also rejected adding a configured CCE
at its first occurrence. Two new own videos reproduce that precise error on
packet 2 after alignment has started: old tag 15 followed by new tag 1, and old
tag 1 followed by new tag 15. Their standalone scalar oracle combines the old
source with a newly initialized source starting at packet 2; the new source has
no contribution before that point. Six qualified variants use 24 kHz mono, active gain, alternating sine/KBD
source shapes and an existing long/start/short/stop source. The newly arriving
source starts either with long, LongStart or EightShort windows; both tag orders
are covered. Packet element order changes independently of tags.

`SsrPcmAlignment::extend_lanes` now preserves every existing lane exactly once,
permits adding lanes and canonical reordering, and prefixes a new source only
for the already pending past interval. That prefix contains no gain mapping or
source contribution. The new packet's actual PCM is retained separately. The
operation rejects duplicates, out-of-range indices, output-channel movement,
lane retirement, post-EOF changes and counts beyond the 16-tag domain before
mutating queues. Unit acceptance verifies samples, gains, stamps, checkpoint
replay and the allocation bound. Native SSR dispatch uses this only for a
superset of already active tags. Its transaction snapshot covers new source
histories, queue topology, gains and source timing.

Enabled native acceptance compares every emitted sample against the scalar
oracle and replays each packet through a checkpoint. All six new MP4s also join
full root/owned export, rewind, seek, repeated range and EOF acceptance. The same
own packets have Matroska fixtures for full/interval root/owned comparisons.
Generators remain offline and separate from tests; no private source media or
codec parameter sets are included.

This closes first occurrence of an additional configured CCE for those authored
switch schedules. It does not define concealment or retirement for a source that
disappears with queued/filter history: the separate removal fixture retains its
precise refusal and rollback regression. General CCE disappearance/reappearance,
SSR SBR/PS and the other documented codec/profile gaps remain unfinished.

Verification: the current production change passed 909 core, 4 gain-control,
3 SSR, 3 coupling and 9 alignment integration tests, plus 450 owned-media unit
tests: 1378 successful test executions (24 existing unrelated tests remain
ignored). After broadening the first-window schedules, the full 9-test alignment
suite passed again, including both MP4 and Matroska paths, every-sample scalar
comparison and playback/range coverage. The earlier failure log records the
precise roster-continuity refusal before the fix. No new acceptance is ignored.
Regeneration reproduced all 38 alignment fixture files byte-for-byte.

### 2026-10-09 — AAC GASpecificConfig extension flag and following metadata

The native ASC parser rejected any `extensionFlag=1`, although implemented LC
and SSR cores have no ER/sub-frame fields there and allow a following zero
`extensionFlag3`. Eight original short MP4 scenarios pair otherwise identical
flag-zero and flag-one ASCs and payloads: indexed LC, LC with SBR-absent sync,
indexed stereo SSR, mono SSR PCE with CCE, explicit HE-AAC, sync HE-AAC, explicit
PS and sync PS. Initial regression execution reproduced precisely
`AAC extension flag is not yet supported`, before any packet decoding.

Shared `config_impl.rs` now retains `extensionFlag` while reading the optional
PCE, then consumes `extensionFlag3` at its actual syntax position. Nonzero future
flags produce `AAC extensionFlag3 must be zero`; a separate short video with no
future bit reaches `truncated or oversized bit field`. Unsupported core object
types still refuse before reading unimplemented ER-specific syntax.

Enabled acceptance compares parsed metadata and every emitted PCM byte with the
flag-zero baseline, including SBR/PS clocks, nonzero decoded audio and SSR window
transitions. Owned/root MP4 export, packet checkpoint replay where applicable,
player decoder checkpoint replay, rewind, seeks at 1100/3000 and exact EOF, and
terminal drain are checked. All eight nonzero future-flag videos and the missing
future-bit video have exact parser refusal checks; the PCM path also refuses
those ASCs. The generator uses only own AAC packets/AVC seed video, runs offline
without a codec executable, and is not invoked by ordinary tests.

This closes the LC/SSR extensionFlag parsing gap, not AAC Main/LTP/ER tools or
other unimplemented profiles. Before broadening the fixture set with sync PS,
the same production change passed 909 core, 4 gain, 3 SSR, 3 coupling, 9 alignment
and 3 extension-flag integration tests, plus 450 owned media tests: 1381 passing
test executions, with 24 existing unrelated ignored tests.
The final eight-scenario 3-test suite passed, including sync PS; regeneration
reproduced all 27 extension-flag fixture files byte-for-byte.

## AAC Main prediction DSP foundation (2026-10-09)

`owned_aac::aac_main_predictor` implements the per-line two-stage backward
lattice predictor, reduced-precision stored state, rounded inverse variance and
estimate, continuous adaptation while prediction is disabled, post-frame
interleaved reset groups 1..30 and complete short-window reset. The spectral
bank validates its band layout and commits spectrum/history together, including
arithmetic failure rollback. It is shared with the root codec API.

The independent Python scalar oracle uses generated inverse mantissa tables and
explicit single-precision scalar stages. Thirty-six events cover changing band
flags, flags omitted above max_sfb, reset groups, signed input, short resets and
bit-exact checkpoint replay over 64 spectral lines. Five primitive unit tests
also cover rounding ties, reset timing, malformed geometry and overflow rollback.
Generation is offline and separate from tests.

`aac-main-prediction-synthetic.mp4` contains eight own mono 24 kHz AAC Main
packets with prediction enabled after three warm-up frames and a group reset.
The first three packets also decode as LC controls; the first active prediction
packet fails with the exact LC prediction refusal, establishing that the source
is not an unrelated malformed packet. Root/owned constructors still reject
AOT 1 explicitly. The playback acceptance test is **ignored** until channel
syntax, per-channel/CCE predictor histories, stereo/PNS/coupling/TNS ordering,
checkpoint accounting and playback/seek are integrated. A passing DSP oracle or
profile refusal does not qualify AAC Main playback. LTP/ER/ELD/USAC remain gaps.

Algorithm reference: ISO/IEC 13818-7:2004 clause 13, referenced for frequency-domain
prediction by ISO/IEC 14496-3. No foreign decoder implementation is incorporated.

### AAC Main native packet and playback integration (2026-10-09)

AOT 1 now uses profile-aware ICS prediction flags/reset groups and the Main
spectral limit for the selected rate; LC/SSR retain their no-prediction syntax.
Long Main TNS syntax admits order 20; LC/SSR retain order 12, and short order 7
is unchanged. The generic TNS filter's bounded history can hold 20 coefficients.
Each mapped output channel and each CCE tag has an independent predictor bank.
Parsing/reconstruction uses cloned candidate banks and commits only on successful
packet synthesis. Checkpoint/restore/reset and retained payload accounting include
all allocated banks. Prediction follows MS and precedes dependent coupling/TNS;
left prediction precedes intensity reconstruction, while right intensity bands
adapt without prediction. PNS bands suppress prediction and reset history.

The old Main profile refusal test is replaced by acceptance, and the previously
ignored MP4 acceptance test is enabled. Two own mono videos cover 24 kHz/1024
and 48 kHz/960, all window sequences, disabled warm-up, active prediction and
interleaved reset. Independent sparse direct-IMDCT/window/overlap-add PCM matches
native output within 2e-8. Root/owned packet PCM and MP4 exports match exactly;
packet checkpoints, malformed-packet rollback and reset replay are checked.
The player adapter exercises checkpoint replay, rewind, intermediate seeks and
exact EOF. Two malformed group 0/31 videos require the exact reset-group refusal
and preservation of prior predictor/synthesis history. Generator output is
reproducible offline and no generator is invoked by tests.

Qualification remains limited to these inputs. Main stereo MS/intensity/PNS,
CCE combinations, order-20 TNS signal effects, PCE bootstrap/ADTS and SBR/PS
interactions need further synthetic acceptance, rather than inheriting LC/SSR
claims. Main support does not close LTP/ER/ELD/USAC or all remaining codec gaps.

Verification for this integration: 908 root library tests, 456 owned media tests,
6 Main integration tests, 3 ASC extension-flag, 3 SSR, 9 SSR alignment and 3 SSR
coupling tests passed offline without FFmpeg (1388 selected successful test
executions; 24 existing unrelated ignored tests). The native Main acceptance
has no ignored test remaining. The eight Main fixture files regenerated
byte-for-byte. These numbers qualify the stated fixtures and affected suites,
not all production profiles or hardware performance.

### AAC Main stereo/PNS and order-20 TNS qualification (2026-10-09)

`scripts/generate_aac_main_tools_fixtures.py` writes original AAC bits and
independent scalar PCM oracles, without external encoders, private media or
network. Seven stereo cases each run at 24 kHz/1024 and 48 kHz/960: explicit MS,
full MS, independent ICS, explicit/full-mask intensity, right-only PNS transition
and correlated two-channel PNS transition. They exercise all four window
sequences, sine/KBD changes, active prediction after warm-up, periodic resets,
and ordinary/intensity/noise recoding. Explicit intensity covers both codebooks
14/15 with enabled/disabled mask inversion. Independent ICS also has different
window sequences and opposite shapes in the two channels, with separate
prediction flags/history. PNS recoding returns to ordinary spectral bands after
noise, testing that old predictor history cannot leak through.

Two additional mono videos use a genuine order-20 TNS filter with only the
last reflection coefficient nonzero, applied forwards and backwards over 32
spectral coefficients. Independent delay-20 recurrences and direct IMDCT verify
the resulting PCM. Two corresponding unfiltered videos have their own PCM
oracles; decoded audio differs by more than 1e-5, making this signal-effect
acceptance rather than a zero-filter parser test. LC refuses these packets at
its order-12 limit, with the expected profile diagnostic. Both illegal order-21
videos refuse at the Main limit while preserving prior history.

The initial new regression exposed a misleading diagnostic: invalid Main TNS
order 21 was called an AAC-LC error. `read_profile` now names Main correctly,
leaving the LC diagnostic unchanged. Passing invalid-order tests prove refusal,
not additional supported TNS orders.

Three enabled integration tests cover all sixteen valid scenarios: scalar PCM
within 2e-8, exact root/owned packet and MP4 export equality, checkpoints, reset,
truncated-packet rollback, playback checkpoints, rewind, seeks at 1100/3000/6100
and exact EOF. All forty fixture files regenerate identically offline; ordinary
tests only read committed artifacts. Remaining Main gaps include explicit CCE
and PCE/ADTS/SBR/PS qualification and wider normative conformance; LTP/ER/ELD/USAC
remain unimplemented. This milestone does not claim all codec gaps are closed.

Final verification for the same production change: 908 root library tests,
456 owned media tests, 3 Main tools tests, 6 Main prediction tests, 3 extension
flag tests, 3 SSR tests, 9 SSR alignment tests and 3 SSR coupling tests passed
offline without FFmpeg (1391 selected successful executions). Twenty-four
existing unrelated tests remain ignored. All new Main tools acceptance tests
are enabled. These results qualify the saved scenarios, not unrestricted codec
conformance or throughput.

## AAC Main/LC/SSR explicit PCE bootstrap

PCE serialization now preserves the configured Main/LC/SSR object instead of
writing LC; ADTS bootstrap requires PCE profile/rate to match its header. Twelve
authored mono/stereo cases cover these profiles and Main CCE tags 1/15 at
coupling points 0/1/3. Independent PCM references, MP4/ADTS export, intervals,
checkpoint replay, rewind and seeks are covered by enabled acceptance tests.
Main coupling uses unity gains and no target TNS; wider interactions remain
unqualified. Four profile/rate mismatches and one truncated Main CCE END are
separate refusal regressions, including decoder history rollback.

All sixty artifacts regenerate offline without FFmpeg or network access. Tests
read saved fixtures. CRC-framed ADTS variants contain placeholder CRC bytes and
qualify framing offsets only, not checksum validation or CRC conformance.

Delivery validation: 908 root library, 456 owned library, 5 new PCE profile,
13 existing PCE, 6 Main prediction, 3 Main tools, 3 extension flag, 3 SSR,
9 SSR alignment and 3 SSR coupling tests passed offline (1409 executions).
Twenty-four existing tests remain ignored; no new acceptance is ignored.

## Owned single-block ADTS CRC protection

`owned_aac::adts_crc` selects normative protection spans with the existing owned
channel, pair, coupling and PCE/DSE readers. CPE parsing additionally returns the
second ICS span transactionally; that span intentionally overlaps the first
192-bit CPE span when needed. The CRC uses polynomial 0x8005, initial all ones,
MSB-first, no final inversion, and zero padding of short audio regions. FIL and
element IDs are excluded. Indexed and streaming ADTS verify before decoding;
CRC failure poisons the streaming reader. Multi-raw-block frames remain refused.

Eighteen new authored companion videos and CRC ADTS streams cover Main/LC/SSR,
mono/stereo, shorter/longer-than-192-bit elements, CPE overlap and independent
Main CCE. Twelve use explicit PCE; six use standard header layouts. DSE and unprotected
FIL occur in every raw block. Their
authored boundaries and independent Python GF(2) long division qualify the
checksums. Corrupted-checksum fixtures test first-frame refusal, and later-frame
mutations test stream poisoning. Complete MP4/ADTS nonzero PCM must match.

Earlier PCE-profile placeholder checksums have been replaced by real checksums:
the offline generator uses the owned region export helper and independent Python
polynomial division. Ordinary tests read committed artifacts; no generator,
FFmpeg, network access or helper compilation is required during execution.
This supersedes the earlier CRC-framing-only limitation; it does not qualify
multi-block protection or all AAC profile/tool combinations.

Final validation: 1418 selected tests passed offline without FFmpeg: 908 root
library, 456 owned library, 3 CRC, 5 PCE profiles, 13 existing PCE, 6 streaming,
6 Main prediction, 3 Main tools, 3 extension flag, 3 SSR, 9 alignment and
3 coupling. Twenty-four existing tests remain ignored. All 116 CRC/PCE-profile
artifacts regenerate identically; no new acceptance test is ignored.

## Owned ADTS multiplexed raw blocks

ADTS transport headers now admit one to four raw blocks. Indexed and sequential
readers separate them into logical AAC packets; each advances 1024 samples.
Unprotected frames use the owned syntax readers through ID_END and byte
alignment to find variable-length blocks. Protected frames validate the position
table relative to the first raw block, header CRC and every block CRC before
exposing any block. An invalid transport poisons the streaming reader without
publishing its first block. Transport counts may vary while configuration and
protection presence remain fixed. The indexed packet limit counts logical blocks.

One hundred twenty authored companion videos have 240 ADTS variants: Main/LC/SSR,
mono/stereo, explicit PCE or standard header layouts, short/long CPE and Main
CCE, in two/three/four-block groups and mixed one/two/four-block groups.
Independent Python polynomial division writes separate header/raw CRCs.
Acceptance tests compare exact packet bytes, 1024-sample timestamps, complete
nonzero MP4/ADTS PCM, intervals, playback checkpoint/replay, rewind and seeks.
Six separate malformed transports retain valid authored AAC and isolate header
CRC, raw CRC, invalid position, position/syntax disagreement and block count.
Their passing refusal tests do not replace playback acceptance. A late raw CRC
fixture checks whole-transport refusal after delivering the preceding frame.
Forty-eight additional videos reuse explicit-PCE transition packets: all window
sequences, sine/KBD changes, Main prediction and coupled source histories.

The Frame index continues describing a logical AAC packet. Single-block frames
retain existing header/offset metadata; multiplexed entries point directly to
each raw payload and have header_bytes=0. This supersedes the prior multi-block
framing refusal, without extending the supported AAC core tools or profiles.

The remux acceptance exposed a production assumption that the final logical
packet ended at the source EOF. For protected multiplexed frames the final CRC
follows that packet. Indexed ADTS now records the complete transport span; the
MP4 writer uses it to reject unrepresented data while preserving all logical
packets. Indexed/sequential MP4 and Matroska remux PCM equals source ADTS.

Final validation: 1422 selected successful test executions (908 root library,
456 owned library and 58 integration tests), with 24 existing ignored tests.
The expanded four multiblock tests passed for all 120 companion videos and
240 ADTS variants. All 368 multiblock artifacts reproduce identically offline.
No new acceptance test is ignored. This qualifies the saved core configurations,
not all AAC profiles, SBR/PS combinations or hardware throughput.


### ADTS multichannel and implicit SBR clock qualification

The authored `adts-layout-sbr` matrix contains 28 companion MP4 videos and
56 protected/unprotected ADTS streams: indexed and PCE 5.1/7.1 with LFE,
implicit mono/stereo SBR, and SBR arriving after the first core packet.
Each stream multiplexes two, three, four, or varying raw blocks per transport
frame. Independent scalar PCM qualifies the four LC layouts; own MP4 PCM
qualifies transport equivalence, output length, intervals, rewind and seek.

A regression reproduced the player reporting 24 kHz for 48 kHz SBR output.
The reader now locates and validates an SBR candidate before publishing its
output rate, and rescales packet timestamps and seek/preroll boundaries into
that output clock. This is selected fixture qualification, not complete AAC
profile or SBR/PS conformance.

Generator: `scripts/generate_adts_layout_sbr_fixtures.py`; sources are authored
synthetic AAC and AVC/container seeds. No private media or parameters are
copied. CRC values use independent polynomial division and the own offline
region helper. Ordinary tests only read saved fixtures; generation and tests
use neither FFmpeg/libav nor network access.


The same authored matrix now covers indexed/sequential ADTS-to-MP4 and
ADTS-to-Matroska remux. A red acceptance reproduced SBR rejection after remux
at a wrongly declared core rate. Writers validate the first SBR candidate and
publish the negotiated output rate. MP4 sample durations scale with that rate;
Matroska preserves nanosecond timing and rewrites its fixed-size track header
before finalization. Packet payloads are not transcoded or buffered as a whole
stream. Acceptance verifies exact full PCM and container output clocks, including
delayed SBR, protected/unprotected multiplexing and multichannel LC.


### Implicit PS in mono-core ADTS

`adts_implicit_ps` reproduces the old ordinary AAC decoder refusal on a late
in-band PS element. Six authored single/multiplexed ADTS streams, protected
and unprotected, share a short synthetic AVC+AAC companion video and existing
independent scalar stereo PCM. The core configuration stays mono AAC-LC at
24 kHz; selected PS playback/export yields stereo at 48 kHz.

Mono ADTS discovery now scans the selected encoded prefix with the own
transactional syntax probe before publishing output geometry. Encoded records
are spooled to a private temporary file; retained RAM does not grow with prefix
length. Replay selects the own PS decoder and drains its delayed frame at EOF.
The native player uses the same verified PS choice and rescales source timing.
Acceptance verifies companion packet identity, independent full stereo PCM,
exact intervals, packet-limit geometry, rewind, seek and repeated EOF drain.
A one-packet prefix stays mono when its future PS packet was not selected.

Generator: `scripts/generate_adts_ps_fixtures.py`. Only existing authored
packets, scalar PCM and AVC/container seeds are used; no private media or
parameters are copied. CRC values use the own offline region helper and
independent polynomial division. Tests read saved fixtures without generation,
FFmpeg/libav or network access. This qualifies the tested 1024-sample late PS
route; first-packet PS remux, broader PCE/coupling combinations and complete
HE-AAC v2 conformance still require separate qualification.


### First-packet PS ADTS remux and source geometry

`adts_ps_remux` reproduces two failures: the mux probe passed an initial PS
payload to the ordinary AAC synthesis decoder, and `aac_source_info` reported
mono/core speaker geometry for verified in-band PS. Six new authored ADTS
streams (single/multiplexed, CRC/plain) and a short AVC+AAC companion cover
first-packet PS, missing-middle PS and restart, alongside the prior late-PS
fixtures. Independent scalar stereo PCM qualifies the new video and ADTS.

Mono implicit SBR/PS mux candidates are now validated by the own transactional
syntax probe. Indexed/sequential MP4 and streaming Matroska preserve original
raw packets, negotiated rate, sample clock/duration and complete stereo PCM.
ASC/core channel declarations remain mono; verified in-band PS selects stereo
output. Source inspection now probes MP4/Matroska payloads before returning
decoded rate, channel count and speaker mask. Tests cover companion source
inspection and both remux outputs, including late PS.

Generator: `scripts/generate_adts_ps_first_fixtures.py`, sharing the existing
own ADTS PS generator. Only authored packets, independent scalar PCM and
AVC/container seeds are used. No private media or parameter sets are copied.
Ordinary tests read saved fixtures and use neither FFmpeg/libav nor network
access. This supersedes the first-packet PS remux gap; broader mono PCE/CCE
combinations and complete HE-AAC v2 conformance still need qualification.


### Mono PCE / PS / CCE ADTS transport qualification

The new `adts-pce-ps` companion video and six protected/plain ADTS variants
reproduce the prior SBR signalling refusal for an explicitly configured normal
front mono PCE. The own PS syntax parser already supported this program and SCE
tag; discovery incorrectly excluded every PCE. Selection now uses the parser's
validated mono-program shape in export, native playback and mux rate probing.
Other AAC layouts remain on the ordinary decoder path.

Independent scalar stereo PCM qualifies the mono PCE video and ADTS. The
`adts-pce-ps-cce` matrix adds nine previously qualified authored PS/CCE programs:
coupling points 0/1/3, tags 1/15/both, reordered sources, source SBR and missing
target FIL. Fifty-four ADTS variants cover single, three-block and varying
transport grouping, with CRC/plain protection. Core direct-cosine and separately
qualified PS/SBR composition remain covered by `he_aac_ps_coupling`; the new
matrix qualifies transport and playback rather than claiming a new independent
whole-decoder oracle for CCE.

Acceptance checks complete PCM, exact raw packet preservation in MP4/Matroska,
output clocks/durations, source geometry, EOF drain, rewind and seek. Generators:
`scripts/generate_adts_pce_ps_fixtures.py` and
`scripts/generate_adts_pce_ps_cce_fixtures.py`, sharing the own offline ADTS PS
generator and region helper. No private media or parameter sets are copied.
Tests use saved files without generators, FFmpeg/libav or network access.
These selected mono PCE/CCE combinations do not establish complete AAC profile,
gain/tool or HE-AAC v2 conformance.


### Stereo PCE SBR in ADTS

`adts_pce_sbr` reproduces the extension-signalling refusal caused by excluding
every explicit PCE from ordinary ADTS SBR discovery. The underlying own
AAC decoder already supported a tagged sole normal-front CPE program. The
discovery gate now uses the resolved mono/stereo channel count and leaves
supported SBR program-shape validation to the native decoder.

Two short authored companion videos and twelve ADTS variants qualify coupled
and uncoupled SBR with CPE tag 3, single/three/mixed raw-block grouping and
CRC/plain protection. Both channels are compared directly with the existing
independent scalar SBR PCM. Acceptance verifies complete PCM, exact interval
selection, output rate/count/duration, raw companion packet identity, indexed
and streaming MP4 and Matroska remux, output-clock timestamps, rewind and seek.

Generator: `scripts/generate_adts_pce_sbr_fixtures.py`, sharing the own offline
ADTS fixture writer and CRC region helper. Only authored PCE/SBR payloads,
independent scalar PCM and AVC/container seeds are used. No private media or
parameters are copied. Ordinary tests read saved fixtures and invoke no
generator, FFmpeg/libav or network access. This qualifies the sole normal-front
stereo CPE at the implicit double-rate clock; arbitrary multichannel SBR
programs and other AAC tools/profiles still require separate implementation
and qualification.


### Multi-element ADTS SBR discovery and bounded PCM spool

`adts_multi_sbr` separately reproduces the former multichannel extension
signalling refusal and, after enabling discovery, the fixed mono/stereo spool
record rejection. LC ADTS discovery now admits the native decoder's supported
resolved layout instead of limiting its channel count to two. Private PCM
records are bounded by 2048 frames times the admitted channels (at least stereo
for PS); push/read enforce that extent. Encoded records remain at most 8191
bytes and retained RAM does not grow with prefix length. Optional controlled
memory admission now reserves possible SBR state for all discovered channels.

Nine authored short companion videos and 54 CRC/plain ADTS variants qualify
two-SCE, height two-SCE, 5.1 PCE/indexed, element reordering, silent LFE and
missing-element SBR. Every output lane is compared with independent scalar
SBR or missing-SBR PCM using the canonical channel mapping. Acceptance checks
full PCM, exact intervals, packet identity, output geometry/clocks, indexed
and sequential MP4 and Matroska remux, timestamps, rewind and seek. A controlled
budget rejection is tested before publishing PCM.

Generator: `scripts/generate_adts_multi_sbr_fixtures.py`, sharing the own ADTS
writer and offline CRC region helper. Only authored payloads, independent PCM
and AVC/container seeds are used; no private media or parameters are copied.
Ordinary tests read saved files without generation, FFmpeg/libav or network
access. This qualifies the tested implicit double-rate programs, not arbitrary
multichannel SBR, nonzero LFE extension processing or complete AAC conformance.
The 14-valued indexed height layout cannot be directly represented by the
three-bit ADTS channel configuration and is not included in this matrix.

### 2026-10-09 — Main + SBR discovery

Removed LC-only SBR discovery gates for supported Main cores in native decoder,
ADTS admission/export, player and mux clock probe. Own six-frame active Main
prediction + SBR videos reproduce the previous ADTS-specific refusal while
explicit/container-clock forms already decode. Acceptance compares all signalling
forms, six protected/plain raw-block layouts, both MP4 mux paths, Matroska and
player seek/rewind. The fixed authored six-frame composition also has an
independent scalar Main/IMDCT/QMF/SBR PCM reference (1e-9 tolerance), with a
no-prediction control. Broader Main/SBR tools and geometries remain unqualified;
this change does not claim complete AAC or other codec conformance.

Validation: 931 root/core and selected integration tests passed (23 existing
ignored), plus 456 owned-media tests (one existing ignored); the expanded
Main+SBR acceptance pair also passed. Offline, no FFmpeg/network. All 11 fixture
artifacts regenerated identically; `git diff --check` passed.


### 2026-10-09 — independent Main + SBR composition PCM

`generate_aac_main_sbr_oracle.py` evaluates the existing authored nonzero Main
program without calling any decoder. Main predictor + sparse direct IMDCT feed
direct analysis/synthesis QMF convolutions, fixed patch geometry, per-frame
energy/gain limiting and cross-frame smoothing/noise phases. A prediction-off
control differs by 1.04749e-4. Committed reference and control cover 12288 samples;
ordinary tests remain offline and do not invoke generators. This closes the
whole-composition oracle gap for this fixture only; arbitrary SBR tools,
profiles and channel layouts remain separate qualification work.

Validation: 11 Main/prediction/SBR/tools acceptance tests passed offline with no
FFmpeg/network. Measured maximum absolute full-composition PCM difference is
4.27283e-8 over all 12288 samples, with fixed acceptance tolerance 1e-7.
The stricter 1e-9 and 2e-8 trials failed; they are not claimed as passed.
All 14 existing/new encoded and oracle artifacts regenerated identically;
`git diff --check` passed. Production code did not change in this oracle step.

### 2026-10-09 — SSR + SBR alignment gap reproduced

Own six-frame SSR window transitions with explicit/sync/implicit-clock SBR now
have companion videos and a valid SSR-only control. The control produces 6144
silent samples. All SBR variants reproduce the exact `AAC SSR SBR synthesis is
not implemented` diagnostic; ordinary offline reproduction passes, intended
playback acceptance remains ignored and explicitly fails when requested.
This is diagnostic progress, not implementation or successful SSR/SBR playback.

The next implementation must preserve SBR frame metadata through SSR's bounded
PCM alignment queue: SSR long/start/short/stop synthesis outputs variable internal
extents, while a timed SBR access unit consumes exactly 1024 core samples.
Pending SBR metadata must follow the aligned frame stamp, including checkpoint,
rewind, delayed return and EOF drain, before broadening discovery/admission.
Simply removing the profile guard or feeding unaligned PCM is insufficient.
Six authored artifacts regenerate identically; no FFmpeg/network/test-time generator.

### 2026-10-09 — bounded SSR packet metadata transport

Native SSR alignment now stores packet durations in a two-slot source-stamped
metadata queue rather than an untagged optional previous duration. A delayed
output consumes only its matching metadata; wrong stamps and excess lookahead
refuse without changing the queue. Checkpoint/restore/reset and decode rollback
retain this state. The shared implementation uses each frontend's own error type
in both root codec and owned-media decoder. The queue itself allocates no heap.
This is the packet-metadata foundation for associating SBR frames with aligned
SSR PCM; SBR frame storage and synthesis are not connected yet, and the SSR/SBR
acceptance remains ignored/red rather than being represented as supported.

Validation: 457 owned-media unit tests passed (one existing ignored), 13 selected
SSR/alignment/reproduction integration tests passed (one intended SSR/SBR
acceptance ignored), and the root metadata queue test passed. All offline with
no FFmpeg/network; `git diff --check` passed. Queue tests cover bounded capacity,
negative stamps, mismatch atomicity, wraparound and checkpoint replay.


### 2026-10-09 — SSR + SBR synthesis after aligned core PCM

SSR packet metadata now retains parsed/dequantized SBR frames and layout through
PCM alignment. The matching source frame drives SBR only after 1024 core samples
are available. Both immediate and delayed paths use the same synthesis helper;
EOF commits alignment, metadata and DSP together only after success. Checkpoint,
reset and decode rollback include the queue. Retained payload counts all pending
frame vectors; optional memory admission reserves bounded metadata copies.

The former SSR/SBR refusal and ignored acceptance have been replaced by passing
container playback acceptance. Six-frame mono programs cover all window
transitions, silent and nonzero spectra with active SSR gain control, explicit,
sync and implicit container clocks, full export, malformed packet rollback,
checkpoint/EOF replay, reset, player rewind/seek and intervals.

The active case revealed an old independent SBR oracle error: its fixed patch
map wrongly included QMF band25 for a width-two final master interval. Correct
normative patch construction discards the final two-band patch. The reference
now derives its patches from independent Decimal master borders; production
patching was already correct. Prior 1e-7/4e-6 tolerance trials are superseded,
not used as acceptance. SSR core matches the scalar IPQF reference bit for bit;
SSR/SBR maximum absolute difference is 2.26381e-10 (nonzero) / 1.81536e-12
(silent). Corrected Main/SBR maximum is 7.26431e-12. Both use 1e-9 tolerance.

Remaining scope: SSR/SBR independent CCE per-source synthesis (explicit refusal),
ADTS discovery, wider layouts/tools, downsampled SSR/SBR and broader codec gaps.
This is qualified container composition, not complete SSR or AAC conformance.

Validation on final production code: 909 root/core + 457 owned-media unit tests,
3 existing SSR + 9 SSR alignment tests, and the final 5 SSR/SBR + 2 Main/SBR
acceptance tests passed: 1385 distinct tests, 24 existing ignored. The SSR/SBR
acceptance is no longer ignored. All run offline without FFmpeg/network.
All 27 SSR/SBR and Main/SBR artifacts regenerate identically; `git diff --check`
passes. No hardware/real-time throughput or full profile conformance is inferred.


### 2026-10-09 — SSR ADTS discovery and delayed core drain

ADTS discovery admits SSR alongside Main and LC. Timed core decoding preserves
SSR window alignment; core-only discovery caches the final delayed PCM at EOF.
The owned synthetic corpus covers silent and active SSR, implicit SBR and core
controls, six-packet programs in 111111/33/123 framing with and without CRC.
Acceptance compares complete PCM and 20–200 ms intervals with the already
qualified companion MP4, and checks indexed/streaming MP4 and Matroska remux.
This does not qualify arbitrary late SBR changes, incomplete SSR transition
prefixes, SSR PS, independent coupling, or all SSR profiles/tools.
Ordinary tests use checked-in fixtures without FFmpeg or network access.

Delivery checks: 909 root unit tests and 457 owned-media unit tests passed;
24 pre-existing tests remain ignored. Both new ADTS tests and all four native
AAC remux tests passed. All 25 ADTS artifacts regenerate identically.
Changed Rust files pass formatting; repository-wide formatting still reports
pre-existing differences outside this change. `git diff --check` passes.


### 2026-10-09 — ADTS player EOF seek and SSR playback acceptance

The ADTS adapter formerly clamped a seek at/beyond stream duration to the
start of the final packet, replaying its audio. The owned six-frame SSR/SBR
fixture reproduced 10240 instead of the requested EOF 12288 output samples.
The adapter now lands at the exact stream duration with no pending packet.
The former last-packet unit expectation is replaced with EOF acceptance.

The SSR ADTS playback regression covers all 24 existing owned files: clock,
duration, complete PCM, rewind, midstream seeks, exact EOF and beyond-EOF seek,
and repeated decoder drain. PCM is compared with the previously qualified
companion synthetic video. This qualifies the player path in addition to
export/remux; broader SSR tools and other codec gaps remain open.

A neighboring late implicit-PS fixture exposed a second EOF-path issue: a
new playback decoder with no input required an in-band PS element on drain.
The playback bridge now returns empty only when no source frame is pending.
A separate regression verifies checkpointed empty drain while a consumed
mono/SBR candidate still refuses EOF without PS. Native presence validation
is retained for every consumed candidate.

Broader multiblock checks retained exact transport errors, but an old test
expected publication of two warm LC packets before a malformed frame.
Clock negotiation deliberately buffers that prefix; the regression now
requires zero published PCM on negotiation failure and separately verifies
that an explicit two-packet limit exports the valid prefix (8192 bytes).

Final checks: 909 root unit tests plus 24 distinct integration tests passed
(ADTS SSR/SBR 3, implicit PS 3, layout SBR 3, multiblock 4, PS in-band 6,
PS playback 5); 23 existing root tests remain ignored. The updated multiblock
refusal/prefix assertion passed separately after the other three tests.
All checks were locked/offline without FFmpeg. Changed-file formatting and
`git diff --check` pass. These results do not establish complete codec parity.


### 2026-10-09 — source-preserving SSR alignment for SBR coupling

The SSR alignment queue now offers `submit_sources`/`finish_sources` alongside
its existing mixed-output API. Each output lane retains separate PCM chunks
and their original output gains when a 1024-sample aligned interval crosses
1472/576-sample synthesis submissions. Packet stamps, bounded lookahead,
checkpoint cloning, EOF completeness and transactional errors are preserved.
Source admission defers gain multiplication/mixing to the extension consumer;
it does not reject a finite source solely because an unused core-domain mix
would overflow. Existing mixed output retains its finite-mix checks.

Three new unit regressions cover opposite drift/gain boundaries and replay,
incomplete EOF/error rollback, and deferred gain overflow. This is the required
source-alignment substrate, not CCE acceptance: SSR/SBR independent coupling
remains refused until per-source SBR history and rendering are integrated and
qualified against synthetic nonzero CCE fixtures and independent PCM.

Validation: all 460 owned-media unit tests passed (one existing ignored),
as did 9 SSR alignment and 5 SSR/SBR integration tests. Locked offline checks
used no FFmpeg; changed-file formatting and `git diff --check` pass.


### 2026-10-09 — valid SSR/SBR independent CCE regression

An authored six-packet mono SSR program with silent SCE0 and active CCE1
now isolates the per-source SSR/SBR coupling gap. Its nonzero core-only
video decodes bit-exact to the independent SSR/IPQF control reference.
Adding the owned SBR FIL to CCE1 reproduces precisely
`SSR SBR independent coupling requires per-source aligned synthesis`,
without a syntax/configuration failure.

The regression includes an explicitly ignored PCM acceptance against the
independent SSR/SBR reference; enable it and replace the refusal expectation
when source-aligned synthesis is connected. Two reproduction/control tests
pass offline without FFmpeg. All four artifacts regenerate identically.
This qualifies the reproducer, not SSR/SBR CCE playback.


### 2026-10-09 — per-source SSR/SBR independent CCE synthesis

The previous CCE refusal/ignored acceptance is superseded. SSR/SBR programs
with independent CCEs now preserve aligned target and coupling PCM separately.
Pending metadata retains each CCE SBR frame by source tag. Target elements and
CCE sources run through their own SBR DSP; original chunk gains apply after
source extension synthesis and before final interleaving. EOF and packet
rollback clone alignment, pending metadata and DSP histories transactionally.
Checkpoint/reset preserve the selected alignment mode. Optional admission and
retained-payload accounting include configured CCE metadata allocations.

Four authored nonzero programs qualify mono/stereo targets with one/two CCE
sources, tags 1/15, explicit 24→48 kHz SBR and unit independent gain. The core
control remains bit-exact; every SBR program matches the independent reference
within 1e-9. Direct native checkpoints/error/reset/EOF and player rewind/seek
acceptance run for all four cases. No CCE acceptance remains ignored.

Remaining scope includes non-unit/time-varying CCE gain, unequal independent
SSR/SBR switching, broader SBR signalling/ADTS transport, source retirement,
transitions between core mixed alignment and SBR source alignment, downsampled
SSR/SBR, PS and wider codec tools/profiles. This is not full codec conformance.

Final validation: 909 root unit tests, 460 owned-media unit tests and
18 SSR integration tests passed, 24 existing ignored outside CCE acceptance.
All seven CCE artifacts regenerate identically. Checks were locked/offline
without FFmpeg; `git diff --check` passes.


### 2026-10-09 — SSR/SBR separate target gain qualification

Two additional authored stereo SSR/SBR CCE programs exercise target selection
3, separate left/right gain lists, constant right gain 0.5 and positive
time-varying gain 1/0.5/2. An independent expectation scales the previously
qualified scalar SBR reference at the original SSR source-chunk boundaries,
including the 1472/576 transition. All six SBR CCE programs now run scalar
PCM, native checkpoint/error/reset/EOF and player rewind/seek acceptance.
The four integration tests pass locked/offline without FFmpeg; all nine
artifacts regenerate identically. Production code needed no further change.

This supersedes the earlier unqualified non-unit/time-varying gain note for
these stated positive-gain programs only. Signed dependent coupling, more target lists,
other gain scales, unequal source switching, signalling/transport variants
and wider codec tools/profiles still require qualification or implementation.


### 2026-10-09 — SSR/SBR common gain scales and sign semantics

Eight more authored stereo CCE programs cover all four gain scales and both
`gain_element_sign` values with nonzero positive/negative common-gain deltas.
The expected multiplier remains positive: ISO/IEC 13818-7:2004 §12.3.3
specifies common gain for independent CCE and positive common-gain sign,
while signed differential gain belongs to dependent coupling. This corrects
the earlier planned “negative independent gain” gap rather than inventing
an unsupported bitstream combination.
Source: https://ossrs.net/lts/zh-cn/assets/files/ISO_IEC_13818-7-AAC-2004-67b015c6ddfc9a4af83665738477124a.pdf

Fourteen SBR CCE programs now run independent scalar PCM comparison, native
checkpoint/error/reset/EOF and player rewind/seek acceptance. A dedicated
regression requires identical PCM for sign-flag pairs at every common-gain
scale. Wider target lists, signed dependent spectral coupling, unequal
SSR/SBR switching and broader codec tools/profiles remain separate work.

Validation: five distinct integration tests pass locked/offline without
FFmpeg; all seventeen artifacts regenerate identically. Changed Rust test
formatting and `git diff --check` pass. Production code is unchanged.


### 2026-10-09 — downsampled SSR/SBR target and CCE qualification

The independent scalar oracle now evaluates either 64-band full-rate or
32-band core-rate QMF synthesis from authored SSR/IPQF core PCM. Four new
explicit/sync target videos cover silent/nonzero spectra and gain control;
fourteen more explicit CCE videos cover mono/stereo targets, one/two sources
and all existing common-gain examples at 24 kHz output. Raw syntax is owned
and reused without private input. Container packet clock is 1024 output
samples rather than 2048, preserving the same 256 ms stream duration.

The native/player regressions now negotiate rate/ticks from each authored
case, compare scalar PCM at 1e-9, replay checkpoints/reset/errors, drain EOF
and check rewind/seek/intervals in the proper output timescale. Gain-boundary
expectations use source-row extents with the appropriate output/core ratio.
A sign-flag comparison runs at both clocks. Production code needed no change.
Existing three full-rate Main oracle artifacts regenerate identically.

Remaining scope includes implicit SBR with unchanged output clock, broader
SSR/SBR switching and retirement, PS, more CCE target layouts and other
codec profiles/tools. This is bounded fixture acceptance, not full codec parity.

Final checks: 6 SSR/SBR and 5 SSR/SBR CCE integration tests pass
locked/offline without FFmpeg; all 50 combined SSR/SBR artifacts regenerate
identically through both generators, including the shared active 32-band
reference. Changed Rust test formatting and `git diff --check` pass.


### 2026-10-09 — implicit SBR at a fixed core-rate output clock

Two target and fourteen CCE videos now isolate implicit SBR at 24 kHz input
and output. Before the fix a valid first-packet FIL refused with
`AAC fill extension tool SBR requires extension-aware stream signalling`.
A matching output/core clock is now a fixed clock hint, not an implicit
assertion that SBR is absent. When ASC leaves SBR unspecified, the decoder
can discover valid FIL without changing that hint. Explicit disable flags
remain honored. Reset preserves the hint; checkpoint compatibility rejects
restoring an automatic dual-rate candidate into a fixed-core candidate, even
when their initial sample rates are equal. Optional controlled-memory
admission covers unknown-signalling SBR candidates before their first FIL.

The direct target and CCE regressions exercise independent scalar PCM,
checkpoint/error/reset/EOF and player rewind/seek. The exact refusal is now
replaced by enabled acceptance. Core-only controls and existing Main/SBR/ADTS
regressions stay green. This qualifies FIL present from the first packet;
late discovery, mixed/source-alignment mode transitions, source retirement,
SSR PS and other codec profiles/tools remain open. No default memory cap,
FFmpeg fallback, foreign codec or network dependency was introduced.

Final validation: 909 root and 460 owned-media unit tests plus 17
distinct integration tests passed (7 SSR/SBR, 5 CCE, 2 Main/SBR, 3 ADTS SSR).
Twenty-four existing unit tests remain ignored. All 66 combined SSR/SBR
artifacts regenerate identically. Checks were locked/offline without FFmpeg;
changed-test formatting and `git diff --check` pass.


### 2026-10-09 — mixed-to-source SSR queue transition on late SBR

An owned six-frame video reproduced precisely
`SSR SBR alignment mode changes require source history continuity` when
first SBR FIL appeared on CCE1 at packet 3. Its valid core control stays
silent; authored SBR noise produces a nonzero suffix with an independent
32-band QMF expectation.

The existing alignment queue already retains unmixed source chunks/gains.
It now upgrades to source output without rebuilding or discarding those
lanes. A pending descriptor with no SBR still emits its original core PCM
and duration; it is not retroactively rendered using the next packet's SBR.
The selected source mode persists and is restored/reset transactionally.
The former refusal/ignored acceptance has been replaced by enabled PCM,
checkpoint/error/reset/EOF and player rewind/seek acceptance.

This closes the alignment-mode refusal for the stated zero-prefix case.
Nonzero pre-FIL QMF history, broader late signalling/rate transitions, source
retirement, SSR PS and other codec tools/profiles still require work.

Validation: 909 root and 460 owned-media unit tests, 9 SSR alignment,
7 SSR/SBR, 5 CCE and 4 late-FIL integration tests passed: 1394 distinct
passes, 24 existing ignored outside the new enabled acceptance. All four
new artifacts regenerate identically. Locked/offline checks used no FFmpeg;
changed-test formatting and `git diff --check` pass.


### 2026-10-09 — preserve nonzero SSR QMF history before first FIL

The owned late-FIL generator now includes nonzero direct SCE and independent
CCE programs, each with a core-only control. Before the fix the CCE program
failed at sample 3072: native output was zero versus the independent scalar
expectation 0.0003223548776518015. The core control remains bit-exact.

Timed implicit SSR discovery now carries warm-only metadata and uses the
source-preserving queue before the first FIL. QMF analysis, low-delay and
synthesis advance for every aligned source, with the candidate output clock;
only original core PCM and original gains are published until FIL arrives.
Header initialization therefore retains nonzero pre-FIL history. Target-only
streams use the same aligned warm-up path. The untimed variable-length core
API remains unchanged outside timed discovery.

The reference extends the independent direct convolution oracle to 32 low
bands during the three-frame prefix, followed by authored 10/27-band SBR
with noise phase starting at FIL. Prefix PCM is original core output, not
QMF output. Four prior Main/SSR scalar references regenerate bit-identically.
Enabled acceptance compares the full waveform and additionally covers
checkpoint/invalid-packet rollback, reset, EOF, player rewind and seek on
both sides of the FIL boundary. The nine late-FIL artifacts regenerate
identically without any external codec, FFmpeg or network access.

This qualifies the authored six-frame, 24 kHz, 1024-tick direct SCE and unit
independent CCE programs. Arbitrary late rate changes, untimed variable-length
discovery, source retirement, SSR PS and wider codec profiles/tools remain
open; this fixture qualification is not complete codec conformance.

Validation: 909 root and 460 owned-media unit tests plus 22 integration tests
passed (2 Main/SBR, 7 SSR/SBR, 5 CCE, 5 late-FIL, 3 ADTS SSR): 1391 distinct
passes, 24 existing ignored unit tests. The late-FIL acceptance is enabled,
not a passing refusal. Checks were locked/offline without FFmpeg. Changed-test
formatting and `git diff --check` pass.


### 2026-10-09 — Main/LC late SBR at a fixed core output clock

Eight owned six-frame programs cover Main/LC and direct SCE0/independent
unit-gain CCE1, each with a core-only control and a first valid FIL at packet
3. The new acceptance initially reproduced exactly
`SBR output rate changed without reset`, after the valid core prefix.

Before presence was established, Main/LC selected Double for warm-up even
when `new_with_output_rate` retained a fixed Core output hint. First FIL
then selected Core, invalidating the retained synthesis state. Target and
CCE warm-up now use the same immutable output hint as discovery. The
pre-FIL published output remains core PCM; QMF history is preserved.

The scalar reference independently combines authored integer spectra, Main
prediction (disabled for LC), direct IMDCT, 32-band prefix QMF and the
existing authored SBR geometry/noise. Enabled acceptance checks all samples,
bit-identical published prefixes, checkpoint/invalid-packet rollback, reset,
EOF and player rewind/seek across the transition. Fourteen generated
artifacts are independent of production decoding, FFmpeg and network access.

This qualifies these 24 kHz, 1024-sample, six-frame programs. Arbitrary
late rate switches, wider layouts/tools and dependent coupling remain
separate gaps; this does not establish complete AAC conformance.

Validation: 909 root and 460 owned-media unit tests, plus 18 integration
tests (3 new fixed-clock Main/LC, 2 Main/SBR, 5 SSR CCE, 5 SSR late-FIL,
3 ADTS SSR) passed: 1387 distinct passes, 24 existing ignored unit tests.
All fourteen fixture/reference artifacts regenerate identically. Checks
were locked/offline without FFmpeg; formatting and diff checks pass.


### 2026-10-09 — independently switched SSR/SBR target and CCE windows

Five unequal window schedules now have synthetic SBR acceptance: source
ahead, source starting ahead, source behind, late target drift, and opposite
long/start/short/stop switching. The target remains silent with sine windows;
the independent CCE has authored nonzero spectra/gain and alternating
sine/KBD windows. Packet element order alternates while FIL remains attached
to its own CCE. Each schedule has a bit-exact nonzero core-only control and
explicit/sync/implicit SBR programs at both 24 and 48 kHz output clocks:
35 six-frame MP4 videos in total, of which 30 contain SBR.

The source-preserving SSR queue and per-source SBR DSP already accept these
schedules; no decoder change was required. Complete PCM is checked against
independent scalar SSR/IPQF core values from the authored alignment oracle,
followed by direct QMF/HF/noise convolutions at 32 and 64 bands. No production
decoder produces the reference. Native checkpoint/invalid-packet rollback,
reset and delayed EOF, and player rewind/seek/EOF cover every program.

All four enabled integration tests passed (15.77 seconds). No refusal or
ignored test is counted as playback acceptance. The 47 generated artifacts
regenerate identically offline, without FFmpeg, private media or network
access. Ordinary tests read committed artifacts and do not run generators.
Formatting and `git diff --check` pass.

This qualifies the five authored window/gain schedules, mono target and one
independent unit-gain CCE with stable tag roster. Dynamic source retirement,
SSR PS, arbitrary wider layouts, untimed late discovery and other codec
profiles/tools remain open. The complete codec goal remains unproven.


### 2026-10-09 — explicit SSR CCE absence and return

The authored `ahead-core` program initially reproduced exactly
`AAC SSR aligned coupling roster changes require lane continuity` when
CCE1 disappeared after a long-start packet. Four schedules now cover source
ahead, behind, exactly at the frame boundary, and return after an absent
packet. Each has nonzero independent scalar core output and SBR at both
24 and 48 kHz: twelve six-frame video programs.

Established lanes now remain in a bounded canonical tag roster. A missing
CCE contributes a zero, gainless input only for the timeline not already
covered by retained source PCM. Queued chunks retain their old gains and
are consumed first; missing input never truncates or repeats them. The
source's coded SSR synthesis state remains keyed by tag for a later return.
Per-source SBR receives the existing pure-upsampling path when its FIL is
absent, preserving QMF history without inventing coded SSR windows.

`absent_input_rows` is read-only and calculates coverage from the original
pending frame plus the current requested frame. Explicit zero/gainless
source spans may have noncoded extents (including 128 samples when 448
queued samples already cover part of a 576-row interval). Ordinary coded
geometry and incomplete EOF remain strict; a missing final coded block
is not silently padded. A queue unit regression covers ahead/behind/exact
extents, original gains, replay and invalid-input rollback. The old roster
refusal is replaced by enabled PCM/checkpoint/EOF acceptance.

The independent scalar reference now supports absent middle FIL frames:
all 32 low bands for pure upsampling, retained analysis/synthesis history,
noise advancement only for SBR frames, and unchanged gain smoothing history
across the gap. Original source-chunk gains are applied after QMF synthesis.
Core controls compare every byte; SBR programs compare every sample at
1e-9 absolute tolerance. Native checkpoint/error/reset/EOF and player
rewind/seek cover every program, including resumed coded source history.

The new fixtures are authored by
`scripts/generate_aac_ssr_cce_absence_fixtures.py` from the existing original
SSR scalar alignment oracle. No private media, foreign codec, FFmpeg or
network is used; ordinary tests do not run generators. The 134 combined
alignment, absence, drift, late SSR and late Main/LC artifacts regenerate
identically. This qualifies these stable-PCE, unit independent CCE programs;
PCE replacement, SSR PS, dependent coupling and wider codec profiles/tools
remain separate work.

Validation: 909 root and 461 owned-media unit tests plus 42 integration
tests passed: 1412 distinct passes, 24 existing ignored unit tests. The
new absence tests and replacement roster acceptance are enabled. All checks
were locked/offline without FFmpeg; four original Main/SSR scalar references
also remain bit-identical after the oracle extension. Changed-test/alignment
module formatting and `git diff --check` pass.


### SSR/PS syntax preparation prerequisite (2026-10-09)

The owned mono SBR/PS stage now separates validated packet syntax from DSP:
`prepare` / `prepare_upsampling` retain at most two unprocessed frames;
`process_prepared` consumes them in wire order when aligned core PCM arrives.
The existing immediate `read` and absent-FIL API remain transactional wrappers.
EOF refuses to discard unprocessed syntax. The retained queue compares the full
opaque prepared frame, so a foreign same-index payload cannot substitute for
accepted syntax. Failed CRC, truncated or nonfinite PCM, duplicate/out-of-order
frames and excess lookahead leave the relevant reader/history/DSP unchanged.
Checkpoint replay and reset preserve that contract. Independent authored scalar
stereo references qualify delayed submission at core and doubled output clocks.

`scripts/generate_aac_ssr_ps_fixtures.py` authors three silent SSR sine/KBD window
schedules with existing original SBR/PS payloads, explicit/sync signaling and
both clocks: three core controls and twelve combined videos. Stereo gold comes
from the independently authored PS/QMF scalar references, not this decoder.
Ordinary tests read committed artifacts; no FFmpeg, foreign decoder, network or
private media is used. Enabled tests prove valid SSR core and PS stage output
and reproduce the exact native SSR/PS profile gate. Combined native waveform
acceptance is explicitly ignored until SSR alignment, PS lookahead and the two
EOF frames are integrated. This change does not claim native SSR/PS playback
acceptance or complete codec conformance.

Validation for the preparation prerequisite: locked/offline root unit tests
(909 passes), owned-media unit tests (461 passes), and the SSR/PS, native PS,
SBR/PS, PS absence and PS coupling integration suites. The ignored combined
SSR/PS acceptance was explicitly run separately and fails at the exact native
profile gate; this is a red acceptance check, not a passing playback test.
The 19 new artifacts regenerate identically. Changed Rust formatting and
`git diff --check` pass.


### Native mono SSR/PS alignment and transport acceptance (2026-10-09)

Mono AAC-SSR now uses its own four-band gain/window/IPQF synthesis before a
fixed 1024-core-sample alignment queue. SBR/PS syntax validates in the original
packet; retained prepared frames are consumed with matching aligned core PCM.
Alignment and PS hybrid lookahead retain two original packet identities.
EOF feeds the final aligned PCM and returns both remaining PS frames on
successive calls (one-input streams still produce their single complete frame).
Native packet transactions, checkpoint/replay and reset retain syntax, gain,
overlap, aligned PCM and PS state together. SSR does not allocate the unused
LC filterbank. Configured SSR PS coupling remains an explicit unsupported tool.

The playback bridge now retains up to two signed source timestamps/durations.
MP4 and Matroska export queues retain source windows/packets and drain every
EOF frame. ADTS owned export and playback discover mono SSR PS in-band,
provide negotiated stereo at twice the core clock and drain both frames.

The old native SSR/PS profile refusal test has been removed and its waveform
acceptance enabled. The 12 mono MP4 cases cover explicit/sync signalling,
24/48 kHz output, sine/KBD and long/start-stop/start-short-stop schedules.
Their nonzero stereo PCM matches the independent authored scalar gold.
Twelve Matroska and three unprotected ADTS streams qualify the same PCM;
twelve additional MP4s add silence and repeated source ranges. Enabled tests
cover full/range export, both EOF identities, source timing including negative
PTS and short durations, malformed packet rollback, checkpoint replay, reset,
rewind and player seek. Repeated-range playback uses the actual AudioStep
reset/silence scheduling contract. The intermediate missing EOF/window bug
reproduced `audio edit extends outside available samples` before the queue fix.

All artifacts come from `scripts/generate_aac_ssr_ps_fixtures.py`, original
silent SSR core syntax and existing original independent SBR/PS scalar gold.
No private source, foreign decoder, FFmpeg or network is used. Ordinary tests
read committed artifacts and do not run generators. This qualifies these mono
programs; nonzero SSR spectral/gain tools composed with PS, SSR PS CCE, broader
layouts/profiles and general codec conformance remain separate requirements.
This section supersedes the earlier preparation-only native refusal status.

Validation: 909 root and 461 owned-media unit tests plus 36 selected integration
tests passed (1406 distinct passes, 24 existing ignored unit tests). All five
SSR/PS tests, including the formerly ignored native waveform acceptance, are
enabled. Checks were locked/offline without FFmpeg. All 46 artifacts regenerate
identically, and the 18 original binary fixtures/references remain unchanged.
Changed Rust module/test formatting and `git diff --check` pass.


### Native SSR/PS dependent and independent CCE composition (2026-10-09)

The configured SSR PS coupling gate is removed. Points 0/1 use spectral mixing
and the target SSR gain/window/IPQF pipeline, with the same mandatory dependent
window-shape check as the ordinary SSR decoder. Point 3 synthesizes each CCE
with its own tag-keyed SSR filterbank/gain history. A source-preserving alignment
queue retains raw PCM and original output gain boundaries, then applies that
source's original SBR FIL (or pure upsampling) at the fixed PS core clock.
Only channel 0 selected by the mono SCE target receives independent coupling
after PS, consistently with the existing owned LC/PS dispatch contract.
Canonical source lanes preserve prior chunks when tags are added; explicitly
absent lanes have no output gains. Native packet transactions, checkpoint,
reset and both EOF frames retain the source syntax, PCM and coupling identity.
SSR CCEs do not allocate an unused LC filterbank.

`scripts/generate_aac_ssr_ps_coupling_fixtures.py` authors 14 nonzero SSR core
controls, 36 combined PS programs and three malformed videos. The matrix
covers points 0/1/3, tag 1 alone and tags 1/15 together, active/inactive SSR gain,
sine/KBD and long/start/short/stop transitions, both 24/48 kHz PS clocks, eight
independent-source SBR programs and four standalone nonzero mono SSR/PS programs.
Controls use the original independently evaluated scalar SSR/IPQF PCM. Combined
acceptance uses that scalar core plus separately qualified owned SBR/PS DSP
composition; it is not a new independent end-to-end PS numerical oracle.
Malformed shape, absent target and source CRC reproduce their specific errors
and preserve queued audio through a complete valid continuation and EOF replay.
All native waveform and export/range/rewind/seek tests are enabled. The old
profile-gate reproduction passed before the implementation and was removed
when the intended acceptance was enabled.

No private media, foreign codec, FFmpeg or network is used. Ordinary tests
read committed artifacts and do not run the generator. This qualifies the
listed stable configured programs, not all coupling or codec conformance.
Distinct source spectra, independently switched source windows, live source
appearance/absence with PS, target TNS composition, PCE replacement and broader
profiles/layouts remain further qualification or implementation requirements.
This section supersedes earlier SSR/PS coupling and nonzero-core refusal notes.

Validation: 909 root and 461 owned-media unit tests plus 36 selected integration
tests passed (1406 distinct passes, 24 existing ignored unit tests). All five
new SSR/PS coupling tests are enabled. The 56 new artifacts regenerate
identically. Checks were locked/offline without FFmpeg; changed native/test
formatting and `git diff --check` pass.


### Distinct SSR/PS coupling source histories (2026-10-09)

The coupling fixture matrix now contains 20 core controls and 48 PS programs.
Six new core controls and twelve new PS videos use independent scalar SSR/IPQF
PCM for tags 1 and 15, different sine/KBD histories and a 2:1 spectral amplitude
ratio. Source wire order reverses on alternating packets. The PS cases cover
inactive/active gain, 24/48 kHz output and absent, shared or asymmetric source
SBR payloads. Native waveform, rollback, rewind, seek, ranges and delayed EOF
acceptance use each source's own reference PCM and extension history.

These are authored synthetic fixtures; ordinary tests need neither FFmpeg nor
network access. The combined PS reference composes independently evaluated SSR
PCM with the qualified owned SBR/PS stage, rather than providing a new fully
independent PS oracle. This qualification does not cover independently switched
source window sequences, source absence/return under PS, or PCE replacement.
No production decoder change was required for this extension.


### SSR/PS independent CCE absence and return (2026-10-09)

Sixteen further authored videos qualify final disappearance and coded-history
pause/return for CCE1 under PS. Source-ahead, source-behind, exactly aligned
absence and return schedules run at 24/48 kHz with and without source SBR.
Target packets remain long-window silent SSR with the original PS payloads.
Source coded ordinals pause during absence; the original scalar SSR alignment
PCM is preserved, including queued samples beyond the last present packet.
Per-chunk gain intervals distinguish those samples from zero absent lanes.

The reference warms source QMF with aligned PCM even when output gain is absent,
then applies the original gain intervals after extension DSP. It composes the
independent scalar SSR source oracle with the already qualified owned SBR/PS
stage; it is not a new independent full PS numerical oracle. Native acceptance
checks checkpoint/retry, reset, packet indices, both delayed EOF frames and
MP4 export. The existing playback test also covers ranges, rewind and seek.
The matrix now has 20 core controls and 64 PS programs. No production decoder
change was needed. PCE replacement, multiple disappearing sources and wider
profiles/tools remain separate work. No private media, FFmpeg or network is
used by the generator or ordinary tests.


### SSR/PS in-band PCE coupling roster changes (2026-10-09)

Twelve synthetic programs reproduce the former exact refusal
`PS AAC in-band PCE changed the configured layout` when only the CCE roster
changes. The native PS decoder and syntax-only PS probe now accept that roster
change transactionally while keeping profile, clock, target SCE tag and layout
fixed. Existing source synthesis/QMF history remains keyed by tag; queued gain
intervals and PCM survive removal and a returning tag resumes its history.

The initial ASC program is retained separately from the current in-band PCE.
Checkpoint restore compares the initial configuration and restores current PCE
state; reset returns to ASC. A checkpoint from another initial roster remains
incompatible. The videos cover aligned/ahead sources, final removal and return,
with/without source SBR at 24/48 kHz. Their waveform reference equals the original
absence/return programs, using independent scalar SSR core and qualified owned
SBR/PS composition. An incompatible stereo PCE companion retains the exact
layout refusal and verifies queued-frame rollback.

This supersedes the fixed-CCE-roster restriction for native PS. General PCE
layout/target-tag replacement, ordinary non-PS AAC roster changes and wider
profiles remain separate work. The matrix has 20 core controls, 76 PS programs
and four malformed/unsupported companions. Generation and ordinary tests use
no private media, foreign decoder, FFmpeg or network.


### Ordinary AAC in-band PCE coupling roster changes (2026-10-09)

NativeAacDecoder now treats the in-band CCE roster as transactional packet
state, retaining its initial ASC program separately. The fixed profile, sample
clock, tagged output elements, height and PCM layout remain checked. Checkpoints
restore current PCE together with synthesis and queued PCM; reset restores ASC.
SBR reserves the four-bit CCE slot domain for explicit PCE programs even when
the current roster is empty, preserving histories through removal/return.

Twelve additional SSR core/SBR videos reproduce the former exact layout refusal
for a valid roster-only change. Their accepted PCM equals the original scalar
SSR/QMF absence/return oracle. They cover source-ahead/behind/exact/return with
core-only and 24/48 kHz SBR output. Tests cover timing, checkpoint replay, reset,
rewind, seek and EOF. Two malformed companions retain exact layout and absent
CCE errors and prove configuration/PCM rollback before continuing the baseline.

Fixtures are authored and generated offline without FFmpeg, foreign decoders
or private media. Dynamic Main/LC roster acceptance, initially empty roster
arrival, output element/tag/layout changes and broader profiles/tools remain
separate qualifications; this does not claim those cases are complete.


### Main/LC dynamic CCE PCE roster qualification (2026-10-09)

Twelve authored mono Main/LC videos qualify the ordinary decoder's roster
changes: static/dynamic PCE controls for mixed-window absence/return, long-only
source absence/return, and two independently arriving sources after an empty
initial roster. CCE1/15 have distinct residual spectra and window histories,
wire order alternates, and each source's coded ordinal pauses during absence.
Target PCM is also nonzero; Main prediction warms, activates and resets on the
authored schedule. Each program has twelve packets (512 ms at 24 kHz).

`scripts/generate_aac_pce_roster_fixtures.py` evaluates independent scalar Main
prediction and direct IMDCT/window overlap. Static and dynamic variants must
produce identical PCM and agree with that oracle. Incorrect scalar controls
that discard source histories on absence differ measurably; the long-window
Main program additionally detects discarding predictor alone while preserving
overlap. No production decoder is used by generation.

Enabled tests cover full native export, exact dynamic/static PCM identity,
checkpoint replay around every PCE, malformed trailing syntax rollback, reset
to empty ASC roster, rejection of a foreign initial roster checkpoint, playback
ranges, rewind and seek. All three tests passed on all twelve videos; fixture
generation and ordinary tests require no private media, FFmpeg or network.
No production change was needed beyond the preceding roster-state fix.

This qualifies Main/LC core independent coupling at point 3. Dynamic dependent
coupling, source SBR across initially empty rosters, multi-channel roster changes
and actual output layout/profile transitions remain separate work.


### Main/LC dependent CCE roster qualification (2026-10-09)

Thirty-two further videos qualify dependent coupling points 0/1 with changing
CCE1/15 rosters in Main and LC. Each has nonzero target and source spectra,
static/dynamic PCE controls, source removal/return or initially empty roster
arrival. Window-transition programs use matching target/source ICS geometry.
Long-window companions carry first-order target TNS: point 0 mixes sources
before TNS, while point 1 mixes after TNS. The scalar predictor, spectral sum,
TNS recurrence and direct IMDCT/overlap oracle computes those stages explicitly.

An oracle-sensitivity test checks eight paired TNS programs and proves the two
points produce different PCM. Native export for both agrees with the oracle,
and static/dynamic variants remain exactly equal. All four roster tests passed
on the resulting 44-video matrix, including trailing-syntax rollback around
PCE changes, checkpoint/reset, empty-initial-roster rejection, playback ranges,
rewind and seek. No production decoder change was required.

Existing twelve videos and scalar/packet prefixes remain unchanged. The
generator and ordinary tests use no private media, FFmpeg or network. These
programs qualify core mono dependent coupling and first-order target TNS;
source SBR across dynamic rosters, stereo/multichannel changes, changing actual
output layouts/profiles and wider codec tools remain separate work.


### SSR source SBR after initially empty PCE roster (2026-10-09)

Six authored videos qualify CCE1 arrival after two complete target packets.
Static-roster controls and initially empty dynamic PCE variants cover core-only
SSR and source SBR at 24/48 kHz. On packet two the dynamic program announces
CCE1 before its coded source/FIL. Independent scalar SSR alignment provides
2048 silent samples followed by four source-window chunks totaling 4096 samples;
the direct QMF/SBR oracle evaluates extension output at each negotiated clock.

Native PCM must agree with that oracle and static/dynamic exports must match
exactly. Tests additionally check the initial silence, checkpoint/replay, reset,
rewind, seek and EOF. Reset restores the empty initial roster and rejects a CCE
without its new PCE; foreign initial-roster checkpoints remain incompatible.
This qualifies the fixed four-bit SBR slot domain added by the earlier roster
fix: an empty current roster does not remove storage for a later CCE tag.
No additional production decoder change was needed.

The absence/roster matrix now has 30 positive videos and two negative companions.
Generation and ordinary tests use no private media, FFmpeg or network. Dynamic
Main/LC source SBR, source PS, multiple late SSR sources, actual target layout
and profile changes and wider codec tools remain separate work.


### Main/LC source SBR under dynamic CCE PCE rosters (2026-10-09)

Thirty-six authored videos qualify independent CCE1 with Main/LC core, source
SBR at 24/48 kHz, and static/dynamic PCE rosters. Six-packet programs cover
source absence/return, initially empty roster arrival and final retirement.
Twelve core controls retain nonzero spectra, alternating window shapes and
Main prediction activation/reset. Target is silent mono SCE0.

The generator evaluates scalar prediction and direct IMDCT/overlap per coded
source ordinal, then direct QMF/SBR convolution on that source clock. When a
complete CCE is omitted, its coded synthesis/extension history pauses while
target presentation continues with zero coupling. This ordinary Main/LC contract
is distinct from SSR's variable-length chunk alignment and absent-lane padding.
Static and dynamic PCM must be identical and both agree with the scalar oracle.

Four enabled tests passed: complete PCM and silence intervals, trailing-syntax
rollback/checkpoint/reset/EOF, playback ranges/rewind/seek, and reference
sensitivity to discarding source extension DSP at return. Incorrect numerical
controls retain core predictor/overlap and reset only extension DSP; all eight
paired SBR programs differ measurably. No production decoder change was needed.
Generation and ordinary tests use no private media, FFmpeg or network.

Dependent coupling with dynamic target/source SBR, source PS, simultaneous
multiple late SBR sources, actual output-layout/profile changes and wider codec
tools remain separate work. This qualification does not claim full codec parity.


### Native nonzero AAC Main + PS acceptance (2026-10-09)

The native PS core-profile gate now admits Main (AOT 1). Target spectrum passes
through the existing owned Main predictor before TNS, coupling, synthesis and
SBR/PS. One predictor bank belongs to the target; configured CCE banks are keyed
by tag and updated before source TNS. Packet clones and opaque checkpoints
retain those banks; reset clears target and source histories. Syntax-only PS
probe admits the same Main mono configuration. Existing LC/SSR behavior remains
covered by adjacent regression suites.

Three authored videos reproduce the former exact profile refusal: a nonzero
Main core control and explicit PS at 24/48 kHz. They exercise sine/KBD, long,
start, short and stop windows, prediction warm-up/activation and group reset.
The core control agrees with independent scalar prediction/IMDCT. Combined
acceptance composes that core PCM with the already qualified owned PS stage;
it is not a new fully independent PS numerical oracle. The refusal expectation
is replaced by enabled waveform acceptance with original indices 0–11 and EOF.

Tests verify malformed trailing syntax rollback, checkpoint replay, reset,
MP4 export, PS probe, playback ranges, rewind and seek. Four integration suites
passed (14 tests), together with all 461 owned-media unit tests (one existing
ignored). Fixture generation and ordinary tests require no private media,
FFmpeg or network. Main PS configured CCE/source SBR, PNS/TNS compositions,
in-band discovery/transports and wider layouts/profiles need separate acceptance
matrices; this mono qualification does not claim those tools are complete.

### AAC Main PS dependent CCE prediction qualification (2026-10-09)

Four authored MP4 videos cover two distinct Main CCE sources (tags 1 and 15),
coupling points 0/1, 24/48 kHz PS output, long/start/short/stop sequences,
sine/KBD windows, distinct predictor reset groups and alternating wire order.
The scalar oracle maintains separate source predictors and combines spectra
before direct IMDCT; the already qualified PS stage supplies the extension
composition reference. No private media, FFmpeg, network or foreign decoder is
used. Generator: `scripts/generate_aac_main_ps_cce_fixtures.py`; acceptance:
`tests/aac_main_ps_cce.rs`. Generation is separate from ordinary tests.

Acceptance checks waveform, per-packet rollback/checkpoint replay, delayed EOF
indices, reset, MP4 export, probe replay, intervals, rewind and seek. A numerical
control discarding predictor history differs from the oracle. Temporarily
omitting the production CCE Main prediction hook makes waveform acceptance fail
at sample 5317 for point 0 / 24 kHz; the production hook is restored byte-for-byte.
All ten generated artifacts reproduce deterministically.

This matrix has no target TNS, source SBR, CCE absence/roster transitions or
independent point 3 coupling. Points 0/1 therefore share PCM in this matrix;
TNS-sensitive ordering and those other combinations still need separate Main PS
acceptance. This is bounded evidence, not full AAC or codec parity.

Validation: 12 integration tests passed across `aac_main_ps_cce`,
`aac_main_ps` and `he_aac_ps_coupling`, locked/offline with production
`media,player` features and no FFmpeg. Production code is unchanged from
`ec338f53a`; the temporary mutation was fully restored.

### AAC Main PS target TNS/coupling ordering (2026-10-09)

The Main PS CCE matrix now includes four additional long-window MP4 videos
(points 0/1, output 24/48 kHz) with first-order target TNS and the same distinct
source predictors/reset groups. An independent scalar all-pole recurrence is
applied after spectral coupling for point 0; point 1 adds the source after the
silent target's TNS. The full PS PCM reference composes the independent core
with the previously qualified PS stage. Acceptance requires observable point
0/1 PCM differences at both clocks and continues waveform, transactional
checkpoint, reset, delayed EOF, probe, interval, rewind and seek checks.

A temporary mutation reversing production point selection passed the original
non-TNS cases but failed `0-24000-tns` at PCM sample 887, proving sensitivity to
this specific stage-order error. Production source was restored byte-for-byte.
The generator deterministically reproduces 18 artifacts; all eight previously
committed videos/core/control files remain byte-identical.

This supersedes the previous target-TNS qualification gap for this first-order,
long-window mono Main PS matrix only. Source TNS, higher orders, independent
point 3, source SBR and CCE absence/roster transitions still need qualification;
full AAC/all-codec parity remains incomplete.

Validation: 11 integration tests passed across `aac_main_ps`,
`aac_main_ps_cce` and `aac_pce_roster`, locked/offline with `media,player`.
Production code remains unchanged; fixture generation stays outside tests.

### AAC Main PS forward/reverse source TNS acceptance (2026-10-09)

Eight additional authored MP4 videos extend the two-source Main PS matrix to
16 videos. CCE tag 1 has forward first-order TNS and tag 15 has reverse TNS;
they retain distinct predictor histories and reset groups. Cases combine
source TNS with and without target TNS at coupling points 0/1 and output
24/48 kHz. The independent scalar core oracle applies Main prediction before
source TNS, then spectral coupling and target TNS at the appropriate point,
followed by direct IMDCT. The previously qualified PS stage remains the
extension composition reference rather than a new independent PS oracle.

Enabled tests verify waveform, checkpoint/rollback, reset, delayed EOF,
MP4 export, probe, intervals, rewind and seek across all 16 cases. An explicit
numerical control omitting source TNS changes the final PS PCM in all eight
new cases. A temporary production mutation omitting source TNS passes the
prior eight cases, then fails `0-24000-source-tns` at sample 851. Production
source has been restored byte-for-byte. All 38 generated artifacts are
deterministic, and the 16 prior videos/core/control files remain unchanged.
Generation is separate from ordinary tests and uses no private media,
FFmpeg, foreign decoder or network.

This covers first-order source TNS with long sine/KBD windows in this mono
Main PS dependent CCE matrix. Higher orders, short-window source TNS,
point 3/source SBR, PNS and CCE absence/roster transitions remain separate
qualification gaps; no complete AAC or all-codec parity claim is made.

Validation: 12 integration tests passed across `aac_main_ps`,
`aac_main_ps_cce` and `aac_pce_roster`, locked/offline with `media,player`
and no FFmpeg. Production code is unchanged.

### AAC Main PS independent point-3 sources (2026-10-09)

Four authored MP4 videos cover independent CCE tags 1/15 at PS output
24/48 kHz. Each source has distinct residuals, Main prediction history,
window-transition schedule and sine/KBD shapes. Two cases carry source SBR
on tag 1 while tag 15 uses upsampling; two use upsampling for both. Wire
order alternates, including CCE placement before and after the target SCE.
The scalar oracle independently computes each source's Main prediction and
IMDCT/overlap. Qualified SBR/PS stages then render source/core composition,
adding independent coupling only to the left SCE output after PS, aligned
with the delayed frame index. This is composition evidence, not a new
independent numerical SBR/PS implementation.

Acceptance checks waveform, source-history sensitivity, packet rollback and
checkpoint replay, reset, all 12 source frame indices including delayed EOF,
MP4 PCM export, syntax probe replay, intervals, rewind and seek. A temporary
production mutation omitting CCE Main prediction fails waveform acceptance
at sample 7196 for 24 kHz upsampling. The original source was restored
byte-for-byte. Generator `scripts/generate_aac_main_ps_independent_fixtures.py`
reproduces ten artifacts deterministically and stays separate from ordinary
tests. No private media, FFmpeg, foreign decoder or network is used.

This qualifies continuously coded Main PS point-3 sources and one authored
source-SBR geometry. Source absence/arrival, PCE roster transitions, multiple
source-SBR geometries, source PS, PNS and wider layouts/profiles remain open;
full AAC/all-codec parity has not been demonstrated.

Validation: 12 integration tests passed across `aac_main_ps`,
`aac_main_ps_cce` and `aac_main_ps_independent`, locked/offline with
`media,player` and no FFmpeg. Reassigning source SBR FIL to the other
source changes the composition reference at both clocks, guarding binding
sensitivity. Production code is unchanged.

### AAC Main PS point-3 source absence and PCE roster transitions (2026-10-09)

Sixteen additional authored MP4 videos extend point-3 qualification to 20
videos: tag 1 disappears for three target frames and returns, or first appears
after three frames, while tag 15 remains coded. Each schedule has static and
dynamic PCE variants, source-SBR on/off and 24/48 kHz output. Dynamic PCE
removes/re-adds the coupling tag while preserving the mono target layout.
Source packet windows, prediction and SBR payloads follow a compact coded
ordinal clock that pauses on absence; the target PS presentation clock
continues. The reference selects independent scalar Main/IMDCT core PCM by
that coded ordinal and retains tag-keyed qualified SBR DSP/history state.

Waveform, packet rollback/checkpoints, reset, delayed EOF, export, probe,
intervals, rewind and seek acceptance cover all 20 videos. Static and dynamic
roster variants must produce byte-identical PCM. A numerical control resets
only the absent source's DSP and observably changes return PCM at both rates,
with source-SBR enabled and disabled. A temporary production mutation clearing
CCE states on every PCE update passes static cases but fails waveform at
sample 8192 in `0-24000-return-dynamic`. Production source is fully restored.
All 26 artifacts reproduce deterministically; the eight prior video/core/
control files remain byte-identical. Generation remains separate from tests,
with no private media, FFmpeg, foreign decoder or network.

This supersedes the tag-1 absence/arrival and compatible PCE roster gap for
this mono Main PS point-3 matrix. Initial empty rosters, simultaneous absence
of both sources, source TNS/PNS combinations, multiple source-SBR geometries,
source PS and wider layouts/profiles remain unqualified; all-codec completion
is not established.

Validation: 13 integration tests passed across `aac_main_ps`,
`aac_main_ps_cce` and `aac_main_ps_independent`, locked/offline with
`media,player` and no FFmpeg. Production code is unchanged.

### AAC Main PS empty rosters and simultaneous source absence (2026-10-09)

Sixteen authored MP4 videos extend the independent point-3 matrix to 36
videos. Both CCE sources disappear together for three target frames and
return, or first appear after three target-only frames. Static and dynamic
PCE variants cover source-SBR on/off and 24/48 kHz output; the dynamic late
arrival ASC begins with an empty coupling roster. Source clocks remain
compact coded ordinals, while target PS advances throughout absence.

The existing scalar Main/core plus qualified SBR/PS composition checks now
cover every case, including waveform, packet rollback/checkpoints, reset,
all target frame indices and delayed EOF, export, probe, intervals, rewind
and seek. Static/dynamic roster PCM must remain identical. The discarded-DSP
numerical control also covers simultaneous source absence. A dedicated test
compares frames with no CCE against a target-only PS reference, and requires
the right channel to remain target-only for every frame even when sources
return. This guards leaked stale coupling PCM and target-clock disruption.

All 42 generated artifacts are deterministic; the 24 prior videos/core/control
files remain byte-identical. Fixture generation is separate from ordinary
tests and uses no private media, FFmpeg, foreign decoder or network.
This covers empty initial rosters and simultaneous absence for this mono
Main PS point-3 matrix. Source TNS/PNS combinations, multiple source-SBR
geometries, source PS, broader profiles/layouts and all-codec parity remain
unqualified. The SBR/PS reference uses qualified owned stages rather than a
new independent numerical SBR/PS oracle.

Validation: 14 integration tests passed across `aac_main_ps`,
`aac_main_ps_cce` and `aac_main_ps_independent`, locked/offline with
`media,player` and no FFmpeg. Production code is unchanged.

### AAC Main PS point-3 source TNS/SBR matrix (2026-10-09)

A separate 36-video authored matrix qualifies forward first-order source TNS
on tag 1 and reverse TNS on tag 15 for independent Main PS coupling point 3.
Long sine/KBD source windows, distinct predictor histories/reset groups,
source-SBR on/off and 24/48 kHz output combine with continuous sources,
single-source absence/arrival, simultaneous absence/arrival, initially empty
rosters and static/dynamic PCE variants. The scalar oracle predicts each
source before directional TNS and direct IMDCT/overlap; qualified owned
SBR/PS stages supply the extension composition reference. A separate source
core control omits TNS, preserving prediction and overlap.

Enabled acceptance checks waveform, packet rollback/checkpoints, reset,
frame indices/delayed EOF, export, syntax probe, intervals, rewind and seek.
Static/dynamic roster PCM stays identical; absent-source DSP discard,
predictor-history discard, source FIL reassociation and source-TNS omission
controls all change final PCM. Empty frames equal target-only PS PCM and
independent sources do not change the right target channel. A temporary
production mutation omitting source TNS fails waveform at sample 968 for
24 kHz upsampling; original production source is restored byte-for-byte.

Generator `scripts/generate_aac_main_ps_independent_tns_fixtures.py` reproduces
44 artifacts deterministically. It stays separate from test execution and
uses no private media, FFmpeg, foreign decoder or network. Prior fixture sets
are unchanged. This qualifies first-order long-window source TNS combinations
in this mono Main PS point-3 matrix only. Short-window/higher-order source TNS,
PNS, additional SBR geometries, source PS and broader profiles/layouts remain
open; neither a new independent numerical PS oracle nor all-codec parity is
claimed.

Validation: 15 integration tests passed across `aac_main_ps`,
`aac_main_ps_cce` and `aac_main_ps_independent_tns`, locked/offline with
`media,player` and no FFmpeg. Production code is unchanged.

### AAC Main PS independent source PNS/TNS/SBR acceptance (2026-10-09)

Eight authored MP4 videos qualify PNS-band switching in two independent Main
PS CCE sources (tags 1/15), with source TNS on/off, source-SBR on/off and
24/48 kHz output. Both sources switch a second band from ordinary residuals
to PNS and back, with distinct noise energies and alternating wire order.
A single scalar noise generator follows actual CCE parse order; per-tag
scalar Main predictors reset the noise-band lines before subsequent ordinary
prediction. Optional first-order TNS runs forward on tag 1 and reverse on
tag 15, followed by direct IMDCT/overlap. Qualified owned SBR/PS stages
provide the extension composition reference rather than a new numerical PS
oracle.

The explicit wrong core control advances predictor state on zeroed PNS input
but omits per-line reset, matching the targeted production mutation. Its
final PS PCM observably differs in every case. Temporarily removing
`bank.reset_lines(range)` from production Main/PNS reconstruction causes
waveform acceptance to fail at sample 21544 after ordinary coefficients
return. Production source is restored byte-for-byte. Other acceptance checks
cover packet rollback/checkpoints (including noise RNG), reset, delayed EOF
indices, MP4 export, syntax probe, intervals, rewind and seek; source FIL
reassociation remains numerically observable.

Generator `scripts/generate_aac_main_ps_pns_fixtures.py` reproduces 18 artifacts
deterministically, separately from tests, with no private media, FFmpeg,
foreign decoder or network. Prior fixture sets remain unchanged. This matrix
uses continuously coded long-window mono Main PS point-3 sources. PNS with
short windows, source absence/PCE changes, broader band geometry, additional
SBR profiles, source PS and wider codecs remain separate gaps; full AAC or
all-codec completion is not claimed.

Validation: 10 integration tests passed across `aac_main_ps_pns`,
`aac_main_tools` and `aac_main_ps`, locked/offline with `media,player`
and no FFmpeg. The existing Main TNS order-21 check remains a refusal
regression, not order-21 acceptance. Production code is unchanged.

### AAC Main PS PNS source absence and PCE changes (2026-10-09)

48 additional authored MP4 videos extend the Main PS PNS matrix to 56 cases.
Single-source return, simultaneous-source return and initially empty-roster
arrival combine with static/dynamic PCE, source TNS on/off, source-SBR on/off
and 24/48 kHz output. New programs have 16 target frames, allowing ordinary
coefficients to return after the compact source-clock PNS interval despite
three-frame source absence. The scalar noise RNG consumes only coded PNS
bands in actual wire order; per-tag prediction and IMDCT states pause while
that source is absent. Dynamic PCE preserves target layout and histories.

Six enabled integration tests pass for every case: independent core plus
qualified SBR/PS waveform composition, omitted-PNS-reset control, source FIL
binding sensitivity, rollback/checkpoints, reset, all target frame indices
including EOF, export, syntax probe, interval, rewind and seek. Static/dynamic
roster PCM is identical. Discarding absent source DSP is numerically observable,
and frames with no sources equal target-only PS PCM; the right target channel
remains unaffected by point-3 coupling. The dynamic/static comparison also
matches source-TNS mode, so different tool programs cannot be confused.

All 90 artifacts reproduce deterministically and the 16 prior video/core/
control files remain byte-identical. Generator execution remains separate
from tests, with no private media, FFmpeg, foreign decoder or network.
Command: `cargo test --locked --offline --no-default-features --features
media,player --test aac_main_ps_pns` (6 passed, no failures). Production code
is unchanged. This qualifies long-window PNS across the selected compatible
roster schedules only. Short-window PNS, dependent-coupling PNS, wider band
geometry/profiles, additional SBR tools and all-codec parity remain open;
SBR/PS uses previously qualified owned stages, not a new independent oracle.

### AAC Main PS short-window PNS point-3 acceptance (2026-10-09)

A separate 56-video matrix qualifies PNS in grouped eight-short windows and
long/start/short/stop source transitions, with distinct schedules for CCE
tags 1/15. Source clocks pause on single/simultaneous absence and late
arrival; initially empty, static and dynamic PCE variants combine with
source-SBR on/off and 24/48 kHz output. Optional directional first-order TNS
runs on non-short windows only; this does not qualify short-window TNS.
A scalar global noise generator follows actual source wire order and generates
one independently normalized noise band per short window. Scalar per-tag
Main prediction resets all history on short windows before subsequent long
prediction; direct IMDCT/overlap supplies the independent core reference.
Qualified owned SBR/PS stages remain the extension composition reference.

The wrong core control retains old long-window predictor state throughout
short windows, without applying prediction to short PCM. It changes final
PCM in every case. A temporary production mutation omitting `bank.short_window()`
fails waveform at sample 12122 for 24 kHz upsampling. Production source is
restored byte-for-byte. Acceptance also covers checkpoint/rollback including
noise RNG, reset, all target frame indices/delayed EOF, export, syntax probe,
intervals, rewind and seek; static/dynamic PCM equality, absent-DSP discard,
source FIL binding and target-only empty frames remain checked.

Generator `scripts/generate_aac_main_ps_pns_short_fixtures.py` reproduces 90
artifacts deterministically, separately from ordinary tests, without private
media, FFmpeg, foreign decoders or network. Prior fixture sets are unchanged.
This covers one grouped short-window geometry for mono Main PS independent
coupling point 3. Other grouping patterns, short-window TNS, dependent-coupling
PNS, broader band/profile/layout geometry and all-codec parity remain open.

Validation: 12 integration tests passed across `aac_main_ps_pns_short`,
`aac_main_tools` and `aac_main_ps`, locked/offline with `media,player`
and no FFmpeg. Existing TNS order-21 coverage remains a refusal test,
not order-21 acceptance. Production code is unchanged.

### AAC Main PS dependent-coupling PNS/TNS acceptance (2026-10-09)

16 authored MP4 videos qualify two PNS-switching Main CCE sources at dependent
coupling points 0/1, source TNS on/off, target TNS on/off and 24/48 kHz PS
output. Sources use distinct residuals and noise energies, alternating wire
order and long sine/KBD windows. Independent scalar reconstruction applies
wire-order PNS and per-tag Main prediction/reset, directional source TNS,
f32 spectral addition, target TNS at the selected point and direct IMDCT.
The qualified owned PS stage supplies extension composition reference.

A first draft used exactly opposite residuals, causing omitted-reset errors
to cancel in the mixed core. The distinct-source fixture replaces that weak
control: skipped PNS line reset now changes every core oracle. Acceptance
verifies all 16 waveforms, checkpoint/rollback/reset, delayed EOF indices,
export, probe, interval, rewind and seek. Target TNS distinguishes points 0/1
at both rates with source TNS enabled and disabled. A temporary production
mutation reversing point selection fails waveform at sample 751 in
`0-1-0-24000`; production source has been restored byte-for-byte.

Generator `scripts/generate_aac_main_ps_dependent_pns_fixtures.py` reproduces
34 artifacts deterministically, separately from ordinary tests, without
private media, FFmpeg, foreign decoders or network. Prior fixture sets are
unchanged. This matrix covers continuously coded, long-window mono Main PS
dependent sources with first-order TNS and one two-band PNS geometry.
Dependent short-window/grouping PNS, source absence/PCE changes, wider
profiles/layouts and all-codec parity remain open. It does not establish a
new independent numerical SBR/PS oracle.

Validation: the offline media/player integration run passed 10 tests across `aac_main_ps_dependent_pns`, `aac_main_ps`, and `aac_main_tools`; no failures. Production source was restored byte-for-byte after the mutation check.

### AAC Main PS dependent PNS source absence/arrival (2026-10-09)

The owned generator `generate_aac_main_ps_dependent_pns_absence_fixtures.py`
authors 16 short synthetic MP4 videos: dependent coupling points 0/1,
24/48 kHz output, optional directional source TNS and target TNS.
Tag 15 first appears on packet 2; tag 1 is absent on packets 6–8 and returns
on packet 9 after its earlier PNS bands. Target and source wire order alternate.
The scalar oracle advances global noise only for actually coded sources and
preserves independent Main prediction histories across absent packets. It
computes source/target TNS, f32 spectral sums and direct IMDCT overlap itself.
The target PS composition uses the separately qualified owned PS stage; this
is not a new independent numerical oracle for PS. Generation is separate from
ordinary offline tests and uses no FFmpeg, network or private media.

The acceptance covers waveform, packet checkpoint/rollback, reset/EOF,
MP4 export, probe replay, interval crop, rewind and seek. Short-window dependent
PNS, grouped bands, target layout transitions and wider codec profiles remain
separate qualification work; this matrix does not prove full AAC parity.

Sensitivity control: temporarily clearing absent CCE states in the production
PS decoder failed waveform acceptance in case `0-0-0-24000`, sample 19275
(`0.0001914204` versus `0.00019121692`), after tag 1 returned. The source was
restored byte-for-byte. All 34 generated artifacts reproduced identical SHA-256
hashes on regeneration.

Validation: restored offline media/player run passed 14 tests across dependent PNS absence, dependent PNS, Main/PS and Main tools; zero failures.

### AAC Main PS dependent short-window PNS acceptance (2026-10-09)

The owned `generate_aac_main_ps_dependent_pns_short_fixtures.py` authors
16 synthetic MP4s with shared long/start/eight-short/stop transitions for target
and two dependent CCE sources. It spans coupling points 0/1, 24/48 kHz output,
and directional source/target TNS on non-short packets. Both sources use distinct
spectral residuals; global PNS consumes each coded short window in wire order.
The scalar oracle computes Main prediction, per-source short-window history
reset, PNS, f32 spectral mixing, optional TNS, direct short IMDCT/windowing and
overlap. Its negative control omits short-window predictor reset and resumes
with the old long-window history. Target PS uses the separately qualified owned
PS composition stage; this is not independent numerical PS qualification.

Acceptance checks waveform, checkpoint/rollback, reset/EOF, root export,
probe replay, interval crop, rewind and seek. The short windows form one group;
other grouping patterns, short-window TNS, source absence during dependent
window transitions and wider profiles remain separate qualification work.
No private source media, foreign decoder, FFmpeg or network is used, and
fixture generation remains separate from ordinary tests.

Sensitivity control: temporarily omitting `bank.short_window()` in the owned
Main channel decoder failed dependent waveform acceptance in case
`0-0-0-24000`, sample 13834 (`-0.0009294358` versus `-0.00092922425`),
after long-window prediction resumed. Shared production source was restored
byte-for-byte before final verification.

Validation: 34 generated artifacts reproduced identical SHA-256 hashes. The restored offline media/player run passed 14 tests across dependent short PNS, dependent PNS absence, Main/PS and Main tools; zero failures.

### AAC Main PS dependent grouped short PNS (2026-10-09)

`generate_aac_main_ps_dependent_pns_grouped_fixtures.py` authors 16 synthetic
MP4 videos with unequal short-window groups in target and dependent CCEs.
The first short packet uses tag 1 groups [1,3,4], tag 15 [2,1,2,3] and target
[3,2,3]; the next uses eight singleton windows, [4,4], and [8], respectively.
Each source alternates ordinary/PNS and PNS/ordinary band layouts between
groups. The own writer serializes group/band/window order and independent
scalar noise reconstruction maps each group into physical window spectra.
Main reset, optional non-short source/target TNS, f32 spectral mixing and direct
IMDCT overlap compose the mono core oracle. Target PS uses the separately
qualified owned stage; the oracle is independent for the core, not PS DSP.

The matrix spans coupling points 0/1 and 24/48 kHz, with checkpoint/rollback,
EOF/reset, export, probe replay, interval crop, rewind and seek acceptance.
These specific nonuniform layouts do not qualify all 128 grouping masks or
short-window TNS. Fixture generation remains separate from tests; no FFmpeg,
foreign decoder, network or private media is used.

Sensitivity: reversing all physical windows was rejected by a different special
band layout and is not waveform evidence. Reversing windows within each group
kept band layouts valid and failed PCM acceptance in case `0-0-0-24000`,
sample 8466 (`0.00026189064` versus `0.0002621318`). Production ICS source was
restored byte-for-byte. Regeneration reproduced identical hashes for all 34
artifacts.

Validation: restored offline media/player run passed 14 tests across grouped dependent PNS, dependent short PNS, Main/PS and Main tools; zero failures.

### AAC Main PS dependent PNS all grouping masks (2026-10-09)

The owned `generate_aac_main_ps_dependent_pns_all_groups_fixtures.py` writes
four synthetic MP4s spanning coupling points 0/1 and 24/48 kHz. Each has 136
core packets: long/start, 128 consecutive short packets, stop and long-window
prediction return. Tag 1 visits masks 0–127, tag 15 their complement, and target
a cyclic permutation. Alternating ordinary/PNS band layouts make group order
visible in PCM. A separate Rust test reads ICS at each actual coded bit offset,
reconstructs its grouping mask, and asserts all 128 masks for each element.

An independent scalar oracle reconstructs noise in group/band/window order,
Main short-window reset, f32 dependent spectral addition and direct IMDCT
window overlap. The target PS stage remains a composition of the separately
qualified owned DSP rather than an independent PS numerical oracle. Acceptance
covers whole-flow PCM, checkpoints/rollback, EOF/reset, MP4 export, probe replay,
interval cropping, rewind and seek. The negative core control retains old Main
prediction during short windows. Fixture generation is separate from tests and
uses no private media, foreign decoder, FFmpeg or network.

This supersedes the grouping-mask count gap for this two-band, two-source Main/PS
dependent PNS setup. It does not qualify every band count, source absence/window
transition combination, short TNS, profile or layout, or complete codec parity.

Sensitivity: reversing window order within each physical group in production
ICS deinterleave failed waveform acceptance in case `0-0-0-24000`, sample
8466 (`0.00026192036` versus `0.00026214647`). The shared source was restored
byte-for-byte. All 10 generated artifacts reproduced identical SHA-256 hashes.

Validation: restored offline media/player run passed 14 tests across all grouping masks, nonuniform groups, Main/PS and Main tools; zero failures.

### AAC Main PS dependent short-window TNS/PNS (2026-10-09)

The owned `generate_aac_main_ps_dependent_pns_short_tns_fixtures.py` authors
16 synthetic MP4s spanning source/target TNS on/off, coupling points 0/1 and
24/48 kHz. Each of eight short windows carries a first-order filter with a
three-bit coefficient and direction alternating by window; tag 15 reverses
tag 1's direction pattern. Nonuniform target/source groups and alternating
ordinary/PNS band layouts remain independent of TNS windows. The scalar core
oracle computes separate recurrence histories per physical short window,
source TNS before coupling, target TNS at the correct coupling stage, f32 sums,
Main reset, direct IMDCT and overlap. Target PS still uses the separately
qualified owned PS DSP; this is not an independent numerical PS oracle.

Waveform, checkpoint/rollback, EOF/reset, MP4 export, probe replay, interval
crop, rewind and seek acceptance cover the matrix. Generation remains separate
from tests with no private media, FFmpeg, foreign decoder or network. Higher
short TNS orders/resolutions/compression, all group/TNS combinations and wider
AAC profiles/layouts remain separate qualification work.

Sensitivity: bypassing only short-window TNS in production channel reconstruction
failed target-only TNS case `0-1-0-24000` at sample 8442 (`0.00020383533`
versus `0.00020408037`). Non-short TNS remained enabled. Shared production
source was restored byte-for-byte. All 34 artifacts reproduced identical hashes.

Validation: restored offline media/player run passed 14 tests across short-window TNS/PNS, grouped dependent PNS, Main/PS and Main tools; zero failures.

### AAC Main PS short TNS orders/resolution/compression (2026-10-09)

The owned `generate_aac_main_ps_dependent_pns_tns_orders_fixtures.py` writes
16 short synthetic MP4s with four consecutive short packets covering TNS
orders 1–7, resolutions 3/4 and compression off/on: all 28 combinations.
Coefficients use both signs, with direction alternating by physical window
and opposite patterns in two dependent CCE sources. Source/target TNS on/off,
coupling points 0/1 and 24/48 kHz remain in the matrix, together with nonuniform
window groups and alternating PNS bands. The independent scalar core oracle
converts authored reflection coefficients to LPC, runs per-window recurrence,
performs f32 spectral addition and direct IMDCT/window overlap, with Main reset.
Target PS uses separately qualified owned DSP; this does not independently
qualify PS numerics.

A syntax companion and Rust test read the actual order/resolution/compression
bits and assert all 28 combinations, finite parsed LPC and both coefficient
signs. Waveform, checkpoint/rollback, EOF/reset, export, probe replay, intervals,
rewind and seek cover the whole flow. All fixture generation stays separate
from ordinary tests and uses no private media, foreign decoder, FFmpeg or
network. Broader coefficient magnitudes, band lengths, profile/layouts and
all grouping/TNS Cartesian combinations remain separate qualification work.

Sensitivity: truncating short-window LPC to one coefficient after fully reading the syntax failed waveform acceptance: `"0-1-0-24000" sample 8451: -0.000034607547 vs -0.00003488504`. Production TNS source was restored byte-for-byte. All 36 generated artifacts reproduced identical hashes.

Validation: restored offline media/player run passed 15 tests across short TNS orders/resolution/compression, first-order short TNS/PNS, Main/PS and Main tools; zero failures.

### AAC LTP syntax foundation and playback gap (2026-10-09)

The owned `aac_ltp_syntax` module parses ordinary LTP side information after
its presence flag: lag/coefficient index and long-band or short-window usage.
It validates frame geometry and lag, and commits the bit cursor only on success.
It does not implement ER/LD lag-update syntax or change profile admission.
The independent authored fixture generator supplies 240 unaligned cases across
960/1024 frame lengths, eight coefficient indices, lag boundaries, band counts,
and short-window optional lag fields, plus malformed geometry/lag/truncation.

Two synthetic MP4s reproduce the existing exact AOT4 configuration refusal,
with active and inactive LTP flags. A passing refusal test is not playback
acceptance. The intended playback test is explicitly ignored until LTP signal
history, prediction, forward transform/TNS integration and profile admission
are implemented; independent PCM qualification will also be required.
Fixtures are generated separately from ordinary offline tests with no private
media, FFmpeg or external decoder execution.

Bit-field reference consulted: [FAAD2 Table 4.4.28 implementation](https://github.com/knik0/faad2/blob/master/libfaad/syntax.c)
and [40-band LTP syntax bound](https://github.com/knik0/faad2/blob/master/libfaad/structs.h).
These are syntax references, not code or runtime dependencies.

Validation: all five artifacts regenerate identically. Owned-library tests: 461 passed, 0 failed, 1 pre-existing ignored. Offline media/player integration: 8 passed, 0 failed, 1 explicitly pending LTP playback acceptance. Syntax acceptance and configuration-refusal reproduction do not establish LTP playback.

### AAC LTP owned time-domain history foundation (2026-10-09)

`aac_ltp_history::LtpHistory` owns a bounded four-frame signed-16-bit history
for 960/1024 core samples. Updates consume raw synthesis PCM/overlap before
normalization, using saturation and nearest-even integer rounding. Estimate
preparation uses lag and one of eight ordinary LTP gains to produce two-frame
long-window analysis input. The future tail remains zero. Reset and matching
checkpoint restore are allocation-free; invalid updates, estimates and restores
leave state/output intact. Short-window analysis explicitly refuses.

`SynthesisHistory::overlap_raw` exposes a borrowed raw overlap snapshot for
future decoder integration. It does not advance synthesis or normalize samples.
The independent Python oracle uses piecewise physical time rather than a
production buffer shift and covers past PCM, overlap, zero future, all gains,
clipping/ties and lag boundaries including 2047. A separate test feeds actual
owned synthesis PCM/overlap into the history. Generation is separate from tests
and uses no private media, foreign codec execution, FFmpeg or network.

This is a required LTP computation stage, not LTP playback acceptance. Forward
analysis/window selection, TNS analysis, band application, transactional decoder
history and AOT4 admission remain unintegrated; the two existing synthetic LTP
videos still reproduce configuration refusal and playback acceptance is ignored.
Gain/history layout reference consulted: [FAAD2 LTP reference](https://github.com/knik0/faad2/blob/master/libfaad/lt_predict.c).
No external decoder code or dependency is imported.

Sensitivity: replacing the previous-PCM carry with zeros failed the independent history oracle at n=960, lag=1920, coefficient=0. Source was restored. All three artifacts reproduce identical hashes and cover 216 full two-frame estimates, including the separately encoded 2047 lag boundary.

Validation: owned-library tests passed 461 with zero failures and one pre-existing ignored. Offline media/player integration passed 11 with zero failures and one pending LTP playback acceptance ignored. The chosen raw-signal scale and integer rounding still require end-to-end LTP PCM qualification when analysis/synthesis is integrated.

### AAC LTP owned FFT forward analysis foundation (2026-10-09)

`Imdct::forward_with_scratch` now folds the 2N input into the DCT-IV domain
and reuses the owned chirp convolution/FFT tables for an unnormalized cosine-sum
MDCT. It is O(N log N), accepts caller-owned scratch, and validates geometry,
finite input and transformed output before publishing any output samples.
An independent dense direct-cosine unit test covers all six supported transform
lengths (32,120,128,256,960,1024) and preserves output on bad scratch geometry.

`aac_ltp_analysis::LtpAnalysis` applies previous/current sine/KBD windows for
only-long, long-start and long-stop at 960/1024 geometry, then runs the forward
transform with retained scratch and no per-call allocations. It does not
advance decoder history; short-window analysis explicitly refuses. A separate
own generator creates 24 sparse boundary-signal full-spectrum references and
four dense harmonic selected-bin references using scalar windows/direct cosine
sums. Acceptance checks repeated calls, invalid input, overflow, short refusal
and recovery without corrupting caller output.

This remains a required computation stage, not LTP playback. TNS analysis,
selected-band application, profile/ICS admission and transactional decoder state
remain unconnected, as does final end-to-end scale/rounding qualification. The
existing synthetic LTP videos still reproduce the explicit AOT4 refusal and
playback acceptance remains ignored. Generation is separate from tests and uses
no private media, foreign decoder execution, FFmpeg or network.
Window-sequence reference consulted: [FAAD2 LTP filterbank](https://github.com/knik0/faad2/blob/master/libfaad/filtbank.c).
No external decoder code/dependency is imported.

Sensitivity: substituting current shape for previous shape failed the independent analysis oracle: `n=960 seq=OnlyLong bin=0: -1024.3450776386292 vs -1026.0142568988708`. Source was restored. All three generated artifacts reproduce identical hashes.

Validation: restored owned-library tests passed 462 with zero failures and one pre-existing ignored. Offline media/player integration passed 13 with zero failures and one pending LTP playback acceptance ignored. All six forward-transform sizes passed dense direct-cosine comparison.

### Owned AAC TNS analysis for LTP prediction spectra (2026-10-09)

`TnsData::analyze_owned` applies the all-zero/FIR analysis counterpart of the
existing TNS synthesis filter to an owned f64 spectrum. Each filter keeps
original-input history, resets at its own band/window boundary and respects
direction plus clipped spectral-band intervals. Geometry/finite LPC validation
precedes processing; overflow refuses. The buffer is consumed and reused with
no second frame allocation. This is a computation stage, not AOT4 admission.

The own generator supplies 96 full-spectrum direct-convolution references over
960/1024 geometry, long/eight-short windows, two filter intervals, both directions,
clipping limits 0/2/5 and orders through 20 (generic Main-capable TNS data;
profile-specific limits still belong to syntax admission). Tests compare FIR
results independently, check synthesis reversibility within f32 precision,
retained pointer/capacity and malformed/overflow refusal. They do not prove all
possible LPC coefficients or geometry. Generation is separate from tests and
uses no private source media, foreign decoder execution, FFmpeg or network.

LTP still needs selected-band application, profile/ICS admission, decoder state
integration and end-to-end scale/rounding/PCM acceptance. Existing synthetic LTP
videos retain exact configuration refusal; intended playback remains ignored.
Analysis recurrence reference consulted: [FAAD2 TNS analysis](https://github.com/knik0/faad2/blob/master/libfaad/tns.c).
No external code/dependency is imported.

Validation: 462 owned-library tests and 15 offline media/player integration tests passed, with zero failures. One pre-existing library test and one pending LTP playback acceptance remain ignored. All three generated artifacts retained identical SHA-256 hashes after regeneration. A temporary incorrect feedback-history mutation failed the independent FIR comparison at n=960, sample=0; the original implementation was restored before these final checks.

### Owned AAC LTP selected-band spectral addition (2026-10-09)

`apply_long_prediction` adds a previously TNS-analyzed prediction to only the
signaled long-window spectral bands. It checks frame geometry, strictly ordered
band offsets, finite operands and the ordinary LTP 40-band limit. All selected
sums are checked for f32 range before any residual write; a late overflow leaves
the caller's complete residual unchanged. No frame allocation occurs. Inputs
must already share normalization; this helper does not establish that contract
for the decoder's eventual LTP integration.

The own offline generator supplies 512 per-bin interval-union references: all
256 masks over eight unequal bands for both 960 and 1024 samples. Exact binary
fractions avoid rounding ambiguity. Tests include the first/last bin, untouched
bands, the 40-band ceiling, malformed geometry, nonfinite values, a late overflow
and reuse after refusal. These are spectral-stage acceptance tests, not video
playback acceptance. The existing own active/inactive LTP MP4 reproducers remain
configuration refusals and the playback acceptance remains pending.

Reference for ordering (analysis filterbank, TNS analysis, selected-band addition):
[FAAD2 LTP](https://github.com/knik0/faad2/blob/master/libfaad/lt_predict.c).
No external implementation or runtime dependency is imported. Profile admission,
channel-pair syntax, transactional decoder history, normalization/rounding and
independent end-to-end PCM qualification remain required.

Validation: 462 owned-library and 17 offline media/player integration tests passed with zero failures. One pre-existing library test and one pending LTP playback acceptance remain ignored. The own 512-case band fixture regenerates with an identical SHA-256 hash.

### Owned ordinary LTP ICS and common-window pair syntax (2026-10-09)

`LtpIcsInfo::read` parses the complete ordinary AOT4 ICS header transactionally,
including independent left/right LTP presence and data in a common-window pair.
Short windows consume only grouping, without long-window predictor flags.
Main predictor state remains separate. Geometry, reserved-bit, band-limit or
nested predictor failures leave the caller cursor unchanged. ER/LD syntax and
production configuration admission are not enabled by this parser.

The own generator authors 896 ICS cases over 960/1024, long/start/stop, sine/KBD,
0/1/40/63 bands, independent pair presence combinations, and every one of 128
short grouping masks for single/common-window modes. Tests verify exact cursor
and a following section sentinel, predictors, groups, all truncated byte prefixes
that end within the ICS, and band-limit rollback. Existing active/inactive own
LTP video packet ICS headers also parse correctly and leave the following
spectral-section codebook aligned. Their decoder playback remains a configuration
refusal; the ignored PCM acceptance is not enabled prematurely.

Syntax reference consulted: [FAAD2 ICS Table 4.4.6](https://github.com/knik0/faad2/blob/master/libfaad/syntax.c).
No external source implementation was copied or linked. Decoder channel-state
integration, normalization/rounding and independent end-to-end PCM qualification
remain required before AOT4 admission.

Validation: 462 owned-library and 20 offline media/player integration tests passed with zero failures. One pre-existing library test and one pending LTP playback acceptance remain ignored. Both ICS artifacts regenerate with identical SHA-256 hashes.

### Composed owned LTP spectral prediction pipeline (2026-10-09)

`LtpAnalysis::predict_long` composes the owned i16 history estimate, shaped forward
MDCT, optional TNS FIR analysis and selected-band addition. History is borrowed
and never advanced; all intermediate spectra are packet-local. A nested TNS or
geometry failure leaves the caller residual unchanged. This initial composition
allocates bounded 2N and N temporary arrays; retained scratch/accounting must be
handled at production decoder integration. The API explicitly uses matching raw
synthesis/MDCT units, not a claim of normative AAC PCM scale or AOT4 admission.

The own generator supplies 24 independent full-spectrum references across both
frame sizes, long/start/stop and all previous/current sine/KBD combinations.
It authors sparse physical previous/current PCM and overlap, quantizes independently,
uses direct cosine sums, original-input FIR convolution in two opposite-direction
intervals and an interval-union selection oracle. Tests check the combined result,
repeated-call identity, unchanged history, nested TNS refusal and mismatched
history geometry. Fixtures regenerate separately, without private media, external
codecs, FFmpeg or network. These spectral-stage tests do not replace the pending
video PCM acceptance or prove the history normalization/rounding contract.

Ordering reference: [FAAD2 LTP/filterbank](https://github.com/knik0/faad2/blob/master/libfaad/filtbank.c).
No external implementation is copied or linked. Production channel/pair state,
profile dispatch, memory accounting and independent end-to-end PCM qualification
remain required before enabling the existing LTP playback acceptance.

Validation: 462 owned-library and 22 offline media/player integration tests passed with zero failures. One pre-existing library test and one pending LTP playback acceptance remain ignored. All three composite-pipeline artifacts regenerated with identical SHA-256 hashes.

### Owned LTP channel histories and scalar PCM chain (2026-10-09)

`LtpChannel` joins spectral prediction, TNS synthesis, window synthesis and LTP
history updates. Checkpoints contain both synthesis overlap and quantized history
plus the previous window shape. Reset restores zero histories and sine shape;
mismatched checkpoint geometry refuses before mutation. Processing advances
histories only after all stages succeed, using temporary cloned state in this
initial adapter. It returns the existing owned 1/65536-normalized PCM convention.
This is not yet production AOT4 profile admission or proof of normative LTP scale.

The independent own scalar generator authors 12 sequential PCM frames across
960/1024, alternating sine/KBD, inactive then active LTP, signed large residuals,
clipped/rounded history and both TNS directions. It uses direct cosine sums and
scalar original-input FIR/feedback AR recurrences, distinct from the FFT adapter.
Tests compare full PCM, checkpoint replay, reset, refusal rollback and mismatched
restore. Generation is separate from ordinary offline tests and uses no private
media, FFmpeg, network or foreign decoder. The existing two own LTP MP4s remain
production configuration refusals, with intended video playback still pending.

Before decoder admission, normative history/PCM scaling and rounding require
external-reference qualification, followed by production channel/pair dispatch,
CCE/PCE interactions and memory accounting. The adapter currently clones channel
state and allocates temporary buffers; it does not claim production performance.

Validation: 462 owned-library and 24 offline media/player integration tests passed with zero failures. One pre-existing library test and one pending LTP playback acceptance remain ignored. Both channel artifacts regenerate with identical SHA-256 hashes.

### LTP floating-history external-reference correction (2026-10-09)

An explicit FFmpeg reference benchmark of the existing own inactive/active LTP
MP4s found inactive PCM peak error 2.6021e-11, but active error 5.2731e-6 with the
initial integer-history channel. The former validates base synthesis scaling on
this fixture; the latter is a specific active-prediction reproduction, not a
claim of general codec acceptance. The same own videos reproduce the issue;
no private source was added.

`LtpHistory::new_float` preserves fractional raw synthesis values without i16
rounding/saturation and is now used by `LtpChannel`. The original fixed-history
constructor and its independently qualified integer behavior remain separate.
Restore refuses a precision-mode mismatch. Nonfinite updates and floating
estimate overflow refuse before state/output mutation. The own channel scalar
oracle was updated explicitly to floating history, rather than retaining the
old integer convention as expected behavior. Fixed-history stage references
remain unchanged.

`scripts/benchmark_aac_ltp_reference.py` is the explicit external benchmark;
ordinary tests only read its saved f32 PCM and version/source/reference SHA-256
manifest. The acceptance covers the channel chain with the known authored
residuals of these two 24k/1024 mono long-window videos, not production packet
parse/dispatch, other tools/layouts or all LTP profiles. Full root playback
acceptance remains pending. External implementation details consulted:
[FFmpeg LTP float DSP](https://github.com/FFmpeg/FFmpeg/blob/master/libavcodec/aac/aacdec_dsp_template.c).
No external source code was copied or runtime decoder dependency added.

Validation: after the floating-history fix, active fixture PCM peak error fell to 3.37394e-10 (inactive remains 2.60203e-11), passing the unchanged 1e-7 tolerance. 462 owned-library and 26 offline media/player integration tests passed with zero failures. One pre-existing library test and one pending root LTP playback acceptance remain ignored. Reference source/PCM hashes were verified against the benchmark manifest; both own channel artifacts regenerated identically.

### Owned ordinary LTP individual-channel packet reader (2026-10-09)

`ChannelData::read_ltp` consumes global gain, ordinary AOT4 ICS/LTP, sections,
scalefactors, pulse, TNS, gain-control and spectral Huffman payload through the
shared channel-body parser. Whole-channel failures restore the original cursor;
non-AOT4 calls refuse. Geometry tables now accept AOT4 for this explicit reader
and reconstruction; production ASC/profile dispatch remains unchanged and gated.
The refactor shares the existing Main/LC/SSR body rather than duplicating tools.

The offline acceptance reads all 24 channel packets from the existing own active
and inactive LTP video fixtures, verifies element/tag and the following END code,
reconstructs the actual coded spectrum and runs owned channel synthesis against
the saved external PCM. Truncated channel prefixes and incorrect profile calls
must preserve the cursor. This replaces metadata-authored residuals with actual
packet spectral parsing for this path. Tests do not execute FFmpeg or use network.

This is individual-channel adapter acceptance, not root-container/player AOT4
acceptance. Common-window/non-common pairs, CCE/PCE state mapping, production
checkpoint dispatch and memory accounting remain required at integration. The
existing root LTP refusal and ignored root playback acceptance remain explicit.

### SSR regression helper EOF-drain correction (2026-10-09)

The expanded channel-body regression found the SSR playback helper returning
20480 bytes against the 24576-byte export: all common bytes were identical, with
only the final queued frame absent. The failure reproduced with the previous
channel/body implementation restored, so it was not introduced by LTP parsing.
Production `audio_thread` already calls `finish_packet` at EOF. The regression
helper now drains that same API and checks repeated EOF is empty before comparing
full, rewind and seek output. The existing short own SSR video supplies the exact
reproducer; no private media or replacement tolerance was introduced.

Validation: 462 owned-library tests, four root channel unit tests and 51 offline integration tests across 13 suites passed with zero failures. One pre-existing library test and 1 integration test(s) remain ignored, including pending root LTP playback acceptance. The previously failing SSR helper now passes full/rewind/seek/ranges with exact PCM equality after EOF drain.
