# Что перенять: предметные наблюдения

Просмотрены выбранные архитектурные участки 33 файлов из 18 проектов. Это не полный аудит этих репозиториев. Наблюдение отделено от предлагаемого решения. Все ссылки ниже закреплены на commit; копии и хеши доступны в source-index.json. Чужие исходники не включены в сборку Fvid.

## 1. FFmpeg/FFmpeg: Базовая линия, которую нельзя упрощать

**Наблюдение.** Scheduler описывает DAG с проверкой циклов и ограничениями синхронизации. Обычный crop сдвигает plane pointers, vflip меняет stride; AVFrame использует reference-counted storage.
**Для Fvid.** Перенять представления кадров и дисциплину drain; улучшать проверяемость контрактов, диагностику и explain.
**Граница вывода.** Не утверждать, что FFmpeg копирует каждый фильтр или не имеет планировщика. Наличие этих механизмов ещё не доказывает оптимальность каждого сценария.

- [fftools/ffmpeg_sched.h](https://github.com/FFmpeg/FFmpeg/blob/72e5fc4b801f55228a429307eff9dbd0fa633120/fftools/ffmpeg_sched.h) — commit `72e5fc4b801f`, 513 строк в сохранённом файле.
- [libavfilter/vf_crop.c](https://github.com/FFmpeg/FFmpeg/blob/72e5fc4b801f55228a429307eff9dbd0fa633120/libavfilter/vf_crop.c) — commit `72e5fc4b801f`, 398 строк в сохранённом файле.
- [libavfilter/vf_vflip.c](https://github.com/FFmpeg/FFmpeg/blob/72e5fc4b801f55228a429307eff9dbd0fa633120/libavfilter/vf_vflip.c) — commit `72e5fc4b801f`, 137 строк в сохранённом файле.
- [libavutil/frame.h](https://github.com/FFmpeg/FFmpeg/blob/72e5fc4b801f55228a429307eff9dbd0fa633120/libavutil/frame.h) — commit `72e5fc4b801f`, 1221 строк в сохранённом файле.

## 2. GStreamer/gstreamer: Пулы и многомерные лимиты очередей

**Наблюдение.** gstbufferpool описывает предварительное выделение и возврат буферов; gstqueue ограничивает bytes, buffers и time, блокирует producer или применяет выбранный drop policy.
**Для Fvid.** Единый бюджет памяти плюс лимиты на каждую очередь; в runtime нужны события EOS/flush и backpressure.
**Граница вывода.** Локальные лимиты сами по себе не доказывают глобального потолка памяти и отсутствия deadlock.

- [subprojects/gstreamer/gst/gstbufferpool.c](https://github.com/GStreamer/gstreamer/blob/f3eda2bee81af74fb7b5417d581637d3c42f184a/subprojects/gstreamer/gst/gstbufferpool.c) — commit `f3eda2bee81a`, 1423 строк в сохранённом файле.
- [subprojects/gstreamer/plugins/elements/gstqueue.c](https://github.com/GStreamer/gstreamer/blob/f3eda2bee81af74fb7b5417d581637d3c42f184a/subprojects/gstreamer/plugins/elements/gstqueue.c) — commit `f3eda2bee81a`, 2003 строк в сохранённом файле.

## 3. vapoursynth/vapoursynth: Контракты зависимостей фильтра

**Наблюдение.** API содержит VSFilterDependency, request patterns, режимы параллельности, запросы конкретных кадров и настройки cache history/size.
**Для Fvid.** Пусть temporal-фильтры объявляют диапазон зависимостей; preview запрашивает нужное поколение и кадр.
**Граница вывода.** Не выводить из API эффективность внутреннего scheduler: просмотрен интерфейс, не весь исполнитель.

- [include/VapourSynth4.h](https://github.com/vapoursynth/vapoursynth/blob/2b621093c383b97d639457706e1e06a6b9bced56/include/VapourSynth4.h) — commit `2b621093c383`, 569 строк в сохранённом файле.

## 4. pdeljanov/Symphonia: Раздельные временные типы и буферизация I/O

**Наблюдение.** units.rs различает Timestamp, Duration, Delta и timebase; MediaSourceStream документирует ограниченный read-ahead и возможность backtracking для nonseekable источника.
**Для Fvid.** Разделить типы времени, сделать возможности источника явными, подключать codecs/formats выборочно.
**Граница вывода.** Полный codec corpus не запускался. Заявления README о скорости не переносим в свои результаты.

- [symphonia-core/src/io/media_source_stream.rs](https://github.com/pdeljanov/Symphonia/blob/ee35874b571a35a9a6e15d3bc9a3aaf8f11fbeee/symphonia-core/src/io/media_source_stream.rs) — commit `ee35874b571a`, 708 строк в сохранённом файле.
- [symphonia-core/src/units.rs](https://github.com/pdeljanov/Symphonia/blob/ee35874b571a35a9a6e15d3bc9a3aaf8f11fbeee/symphonia-core/src/units.rs) — commit `ee35874b571a`, 1480 строк в сохранённом файле.

## 5. MetalPetal/MetalPetal: Оптимизация графа с учётом зависимостей

**Наблюдение.** Optimizer строит node table, учитывает dependents и cache policy, вызывает специализированные объединения color matrix/compositing, перестраивает dependencies. Promise имеет dependencies, dimensions и alpha type.
**Для Fvid.** Небольшие доказуемые rewrite rules; отличать transient и persistent результаты; учитывать fan-out.
**Граница вывода.** Это специализированные правила, а не произвольное безопасное объединение всех GPU-операций.

- [Frameworks/MetalPetal/MTIRenderGraphOptimization.m](https://github.com/MetalPetal/MetalPetal/blob/f9b78897bd4214bb097f352a1bde0a4f4a1e2ddb/Frameworks/MetalPetal/MTIRenderGraphOptimization.m) — commit `f9b78897bd42`, 125 строк в сохранённом файле.
- [Frameworks/MetalPetal/MTIImagePromise.h](https://github.com/MetalPetal/MetalPetal/blob/f9b78897bd4214bb097f352a1bde0a4f4a1e2ddb/Frameworks/MetalPetal/MTIImagePromise.h) — commit `f9b78897bd42`, 195 строк в сохранённом файле.

## 6. BabitMF/bmf: Устройство как часть контракта кадра

**Наблюдение.** VideoFrame имеет device, copy_ и to(device, non_blocking); документация to различает передачу устройства и разделение уже находящихся там данных. SchedulerQueue предоставляет очередь задач и pause/resume.
**Для Fvid.** Явные upload/download и события готовности; стоимость переноса входит в placement.
**Граница вывода.** Производственные масштабы и скорость из README — заявления авторов, здесь не проверенные. Из header очереди не следует global memory bound.

- [bmf/engine/c_engine/include/scheduler_queue.h](https://github.com/BabitMF/bmf/blob/5c8d302a468085b853613f026876c39d920ee20e/bmf/engine/c_engine/include/scheduler_queue.h) — commit `5c8d302a4680`, 135 строк в сохранённом файле.
- [bmf/sdk/cpp_sdk/include/bmf/sdk/video_frame.h](https://github.com/BabitMF/bmf/blob/5c8d302a468085b853613f026876c39d920ee20e/bmf/sdk/cpp_sdk/include/bmf/sdk/video_frame.h) — commit `5c8d302a4680`, 209 строк в сохранённом файле.

## 7. software-mansion/smelter: GPU-декодирование и порядок показа

**Наблюдение.** WgpuTexturesDecoder выдаёт NV12 textures; FrameSorter использует metadata PTS/picture order и отдельный flush.
**Для Fvid.** Разделить ready/decode/display order; GPU storage проходит через граф без обязательной CPU-материализации.
**Граница вывода.** Не проверяли на устройстве ни этот backend, ни возможность совместного использования с VideoToolbox.

- [gpu-video/src/decoders/wgpu_api.rs](https://github.com/software-mansion/smelter/blob/4afebd308e46e931db03938f798d5217d7d0ffbb/gpu-video/src/decoders/wgpu_api.rs) — commit `4afebd308e46`, 86 строк в сохранённом файле.
- [gpu-video/src/frame_sorter.rs](https://github.com/software-mansion/smelter/blob/4afebd308e46e931db03938f798d5217d7d0ffbb/gpu-video/src/frame_sorter.rs) — commit `4afebd308e46`, 115 строк в сохранённом файле.

## 8. chromeos/cros-codecs: Surface lifetime и смена формата

**Наблюдение.** FramePool учитывает минимальное число кадров; события различают FrameReady/FormatChanged; DecodedHandle имеет is_ready/sync. PooledVaSurface возвращается в существующий совместимый pool при Drop.
**Для Fvid.** Учитывать DPB и поколение формата; держать ресурс до завершения fence и последнего пользователя.
**Граница вывода.** Проект ориентирован на Linux. Нельзя автоматически перенести его API и гарантии на macOS.

- [src/decoder.rs](https://github.com/chromeos/cros-codecs/blob/5ff6d693ffae0b36935b8fc13092c733b4c2646f/src/decoder.rs) — commit `5ff6d693ffae`, 266 строк в сохранённом файле.
- [src/backend/vaapi/surface_pool.rs](https://github.com/chromeos/cros-codecs/blob/5ff6d693ffae0b36935b8fc13092c733b4c2646f/src/backend/vaapi/surface_pool.rs) — commit `5ff6d693ffae`, 225 строк в сохранённом файле.

## 9. Vanilagy/mediabunny: Обратное давление до encoder и writer

**Наблюдение.** media-source.ts ожидает dequeue при заполнении encoder queue и lastMuxerPromise для writer backpressure; явно закрывает кадры. Conversion API различает transforms, codec options и сохранение rotation metadata.
**Для Fvid.** Состояния жизненного цикла и backpressure входят в публичный контракт; высокоуровневый API строит тот же план.
**Граница вывода.** WebCodecs — внешняя реализация кодеков; чистый TypeScript не означает, что сами кодеки написаны на TypeScript.

- [src/conversion.ts](https://github.com/Vanilagy/mediabunny/blob/c67c5e4072cf834743498c45a5a5bdf058947aec/src/conversion.ts) — commit `c67c5e4072cf`, 2042 строк в сохранённом файле.
- [src/media-source.ts](https://github.com/Vanilagy/mediabunny/blob/c67c5e4072cf834743498c45a5a5bdf058947aec/src/media-source.ts) — commit `c67c5e4072cf`, 3056 строк в сохранённом файле.

## 10. membraneframework/membrane_core: Demand вместо неограниченного производства

**Наблюдение.** ManualFlowController обрабатывает demand/redemand, откладывает повторные запросы; InputQueue хранит размер и единицы demand.
**Для Fvid.** Явное количество разрешённых данных и метрики очереди; ограничить циклы перепланирования.
**Граница вывода.** Не копировать процессную/actor модель буквально для каждого Rust-фильтра; supervision ещё требует отдельного дизайна.

- [lib/membrane/core/element/manual_flow_controller.ex](https://github.com/membraneframework/membrane_core/blob/3aca29d537ae7f49f2bc792c48fd68d16420ec61/lib/membrane/core/element/manual_flow_controller.ex) — commit `3aca29d537ae`, 219 строк в сохранённом файле.
- [lib/membrane/core/element/manual_flow_controller/input_queue.ex](https://github.com/membraneframework/membrane_core/blob/3aca29d537ae7f49f2bc792c48fd68d16420ec61/lib/membrane/core/element/manual_flow_controller/input_queue.ex) — commit `3aca29d537ae`, 373 строк в сохранённом файле.

## 11. BillyDM/Firewheel: Расписание и буферы готовятся до обработки

**Наблюдение.** CompiledSchedule хранит назначения buffers и возможность их переиспользования; sync_new_buffers документирует отсутствие allocation при безопасном reuse. process.rs обрабатывает звук блоками.
**Для Fvid.** Скомпилированный план и preallocation; control plane отдельно от realtime callback.
**Граница вывода.** Исходники используют raw pointers и unsafe в горячих участках. Это источник архитектуры, не свидетельство полностью safe-Rust исполнения.

- [crates/firewheel-graph/src/graph/compiler/schedule.rs](https://github.com/BillyDM/Firewheel/blob/5e557f164fd2d5ee942ecc2bdaf2763c07fe6d53/crates/firewheel-graph/src/graph/compiler/schedule.rs) — commit `5e557f164fd2`, 1380 строк в сохранённом файле.
- [crates/firewheel-graph/src/processor/process.rs](https://github.com/BillyDM/Firewheel/blob/5e557f164fd2d5ee942ecc2bdaf2763c07fe6d53/crates/firewheel-graph/src/processor/process.rs) — commit `5e557f164fd2`, 754 строк в сохранённом файле.

## 12. rust-av/Av1an: Параллелизм по частям задания

**Наблюдение.** Broker распределяет chunks через bounded channel, обрабатывает ошибки encoder и записывает завершённые части в done.json; диапазоны chunk используют exclusive end.
**Для Fvid.** Checkpoint по завершённым независимым частям, ограничение worker concurrency, явные границы.
**Граница вывода.** Нужны отдельные проверки швов, GOP, идентичности настроек и аудиосинхронизации. README-ускорения не воспроизведены.

- [av1an-core/src/broker.rs](https://github.com/rust-av/Av1an/blob/805dad69143fa0a81cfe2fb89c0b9e90a828ea72/av1an-core/src/broker.rs) — commit `805dad69143f`, 382 строк в сохранённом файле.
- [av1an-core/src/chunk/mod.rs](https://github.com/rust-av/Av1an/blob/805dad69143fa0a81cfe2fb89c0b9e90a828ea72/av1an-core/src/chunk/mod.rs) — commit `805dad69143f`, 99 строк в сохранённом файле.

## 13. cool-japan/oximedia: Матрица зрелости важнее числа функций

**Наблюдение.** codec_status.md различает фактические режимы и историю их проверок. В просмотренном crop_plane выделяется новый Vec и копируются pixels, с fallback 0 при отсутствующем sample.
**Для Fvid.** Вести capability matrix по профилям; Unsupported не должен становиться фальшивым успехом. Crop-view в Fvid следует проверить отдельно.
**Граница вывода.** Не считать большой README или число tests доказательством замены FFmpeg. Наблюдение о crop относится только к указанному файлу/версии, а не всему проекту.

- [docs/codec_status.md](https://github.com/cool-japan/oximedia/blob/5263510374761ec8ef02dace5d26d62fb20e405b/docs/codec_status.md) — commit `526351037476`, 1102 строк в сохранённом файле.
- [crates/oximedia-graph/src/filters/video/crop.rs](https://github.com/cool-japan/oximedia/blob/5263510374761ec8ef02dace5d26d62fb20e405b/crates/oximedia-graph/src/filters/video/crop.rs) — commit `526351037476`, 699 строк в сохранённом файле.

## 14. rust-av/rust-av: Общая модель данных отдельно от codec API

**Наблюдение.** data/src/frame.rs содержит общие frame types и errors; codec/src/lib.rs разделяет common/decoder/encoder/error модули.
**Для Fvid.** Переиспользуемые core primitives без зависимости от CLI; единый контракт для нескольких backend.
**Граница вывода.** По структуре модулей нельзя заключить о полноте кодеков или скорости; дополнительно требуется поддержка нужных profiles.

- [data/src/frame.rs](https://github.com/rust-av/rust-av/blob/d44bb083ffc543d80397c8244f2e8c534f609308/data/src/frame.rs) — commit `d44bb083ffc5`, 652 строк в сохранённом файле.
- [codec/src/lib.rs](https://github.com/rust-av/rust-av/blob/d44bb083ffc543d80397c8244f2e8c534f609308/codec/src/lib.rs) — commit `d44bb083ffc5`, 12 строк в сохранённом файле.

## 15. mozilla/mp4parse-rust: Отказоустойчивое выделение памяти парсера

**Наблюдение.** MP4 parser использует fallible_collections/TryVec/TryRead и проверяемые числовые преобразования; источник явно отделяет типы для fallible allocation.
**Для Fvid.** Ограничить размеры, вложенность и объём parse work; ошибки allocation выражать в результате.
**Граница вывода.** Просмотр выбранных участков не является полным security-аудитом и не доказывает отсутствие panic.

- [mp4parse/src/lib.rs](https://github.com/mozilla/mp4parse-rust/blob/215e238e633fa0df42cc25bfcf43a0f902692691/mp4parse/src/lib.rs) — commit `215e238e633f`, 6688 строк в сохранённом файле.

## 16. halide/Halide: Алгоритм и способ вычисления — разные решения

**Наблюдение.** Учебный исходник на одном producer/consumer показывает inline, compute_root, compute_at, store и tiling.
**Для Fvid.** LogicalPlan хранит семантику, PhysicalPlan выбирает материализацию и порядок вычисления.
**Граница вывода.** Сам Halide не внедрён и его скорость здесь не измерялась. Собственный JIT не нужен в первом срезе.

- [tutorial/lesson_08_scheduling_2.cpp](https://github.com/halide/Halide/blob/8cfbcdb0a6e718eff70f6dfd04f37dcc78ede5ca/tutorial/lesson_08_scheduling_2.cpp) — commit `8cfbcdb0a6e7`, 709 строк в сохранённом файле.

## 17. libvips/libvips: Обработка нужного региона

**Наблюдение.** generate.c описывает start/generate/stop sequence и per-thread input regions; генератор заполняет requested output region.
**Для Fvid.** Тайлы/регионы и ленивое вычисление там, где это поддерживает операция; bounded cache.
**Граница вывода.** Это библиотека изображений. Stateful video codec нельзя произвольно превратить в региональный декодер.

- [libvips/iofuncs/generate.c](https://github.com/libvips/libvips/blob/22331a771c76407e2ac4616be86c82c55f3f9f16/libvips/iofuncs/generate.c) — commit `22331a771c76`, 788 строк в сохранённом файле.

## 18. AcademySoftwareFoundation/OpenTimelineIO: Временная шкала монтажного проекта

**Наблюдение.** RationalTime хранит value/rate и умеет rescale, но оба значения представлены double.
**Для Fvid.** Явная time scale и диапазоны на уровне timeline; адаптация в точное внутреннее время двигателя.
**Граница вывода.** Не приписывать OTIO целочисленную рациональную точность. Timeline interchange не заменяет media runtime.

- [src/opentime/rationalTime.h](https://github.com/AcademySoftwareFoundation/OpenTimelineIO/blob/bc5fe2d78dc3f8b2a8feb7e04483d85a12e80072/src/opentime/rationalTime.h) — commit `bc5fe2d78dc3`, 428 строк в сохранённом файле.

## Дополнительные первичные документы

- [GStreamer: общая архитектура](https://gstreamer.freedesktop.org/documentation/additional/design/overview.html): negotiation, clock, события и push/pull.
- [GStreamer: buffer pool](https://gstreamer.freedesktop.org/documentation/additional/design/bufferpool.html).
- [FFmpeg: send/receive и drain](https://ffmpeg.org/doxygen/trunk/group__lavc__encdec.html).
- [VapourSynth API](https://www.vapoursynth.com/doc/apireference.html).

Документация дополняет исходники; live-страницы могут изменяться. Структурные гипотезы не заменяют измерений.
