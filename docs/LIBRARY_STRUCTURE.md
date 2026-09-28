# Структура библиотек FVid

Статус: целевая схема и ближайшие шаги, 28 сентября 2026. Это план границ, а не предложение создать набор пустых crates. Реальный код и подтверждённые ограничения важнее красивой диаграммы.

## Что уже есть

В Cargo-пакете `fvid` живут и библиотечный API, и CLI. В нём соседствуют собственные контейнеры/кодеки, Y4M-конвейер, playback/UI и интеграции. На верхнем уровне часть playback-файлов плоская (`playback_mp4.rs`, `playback_wav.rs` и т. п.), хотя уже организована по доменам в `codec/`, `container/`, `color/`.

Отдельные crates появились по техническим причинам: `fvid-cuda`, `fvid-vt`, `fvid-platform` и `fvid-cpu` имеют особые системные зависимости, FFI или требования к `unsafe`. `fvid-media` — отдельный, но legacy-адаптер к libav, который объединяет несколько задач. Его не следует считать целевой моделью собственного media core.

Первые безопасные шаги уже выполнены: Y4M API и потоковая обработка вынесены в `src/y4m.rs`; общая ошибка и общие механизмы получили отдельные `error.rs`, `memory.rs`, `parallel.rs`. `fvid::y4m::*` — явное доменное пространство; прежние имена `fvid::{Header, Plan, process, ...}` оставлены как фасад совместимости. Playback-файлы физически сгруппированы в `src/playback/{audio,video,subtitles,runtime}/`; новые пути доступны через `fvid::playback::{audio,video,subtitle_tracks,runtime}`, а прежние `fvid::playback_*` пути сохранены. Это границы модулей, не новые Cargo crates: playback-адаптеры всё ещё используют соседние root re-export'ы. Тесты, проверяющие только legacy `fvid-media`, перенесены из `lib.rs` в отдельный feature-gated integration test.

## Предлагаемая структура модулей

Сначала привести в порядок границы внутри существующего crate и только затем извлекать стабильные Cargo crates:

```text
src/
  lib.rs                 # публичный фасад и совместимые re-export'ы
  error.rs               # типы ошибок и контекст диагностики
  media/                 # общие типы времени, stream/frame/audio, цвет и buffer views
  y4m/                   # Y4M reader/writer, проверки геометрии, трансформации
  graph/                 # описание операций и проверка графа (когда появится реальный граф)
  planner/               # выбор форматов/backend и объяснение конверсий (после вертикального среза)
  runtime/               # offline/preview/live scheduling, backpressure, cancel и flush
  format/                # контейнеры; модуль на формат, без логики UI
  edit/                  # намерение редактирования и выбор execution plan
    trim.rs              # временные диапазоны
    concat.rs            # модель склейки
    transform.rs         # crop / flip
    plan.rs              # выбор packet-copy или transcode
  codec/
    video/               # avc, hevc, vp9, av1
    audio/               # pcm, aac, ac3, flac, ...
  filter/                # color, geometry, audio filters
  backend/               # контракты выбора backend и общие адаптеры
  playback/              # открытие источников, декодирование и playback policies
  app/                   # CLI/MCP/UI как входные адаптеры
crates/
  fvid-cpu/              # SIMD/unsafe kernels, только при реальной границе зависимости
  fvid-cuda/             # CUDA adapter
  fvid-vt/               # VideoToolbox FFI
  fvid-platform/         # OS-specific services
  fvid-media/            # временно, изолированный legacy libav adapter
```

`graph/`, `planner/`, `runtime/` — будущие модули, не повод заранее вводить абстракции без потребителей. Сейчас не нужно ни выносить каждый codec в crate, ни создавать общий plugin ABI.

## Будущие границы crates

Когда зависимости и циклы можно будет выразить тестами, разумна такая последовательность:

| Crate | Владеет | Не зависит от |
|---|---|---|
| `fvid-core` | типы media/time, error, capabilities/ports, leases и бюджеты | UI, CLI, конкретный codec/container, wgpu, Tokio, libav |
| `fvid-formats` | demux/mux конкретных контейнеров и их metadata mapping | player UI, hardware backend |
| `fvid-codecs` | собственные codec implementations, codec-specific state | контейнеры и UI |
| `fvid-processing` | CPU filters и позднее graph/planning APIs | файловая система, UI |
| `fvid-runtime` | исполнение выбранного плана, очереди, cancel/flush/backpressure | конкретные контейнерные форматы и окна |
| `fvid-backend-*` | CPU/GPU/platform adapters и device interop | детали пользовательского интерфейса |
| `fvid` / бинарь | удобная сборка API, features, CLI; отдельное приложение player при необходимости | внутренние детали других слоёв сверх их публичных API |

Ports/capabilities определяются со стороны потребителя. Например, runtime просит узкий `PacketSource`/`FrameDecoder`, а не передаёт огромный `MediaBackend` со всеми функциями. `fvid-formats` и `fvid-codecs` работают с общими типами `fvid-core`; container не декодирует кадр, а codec не открывает окно.

Практический критерий извлечения crate: он имеет одного владельца ответственности, полезную самостоятельную сборку/тесты и действительно более узкие зависимости. Если выделение добавляет циклические path-dependencies или только перемещает файлы, оставляем модулем.

## SOLID и DRY: правила для этого репозитория

- **S — Single Responsibility.** Не помещать парсинг контейнера, codec decode, playback policy и рендер окна в один тип. Разделять сначала по изменяющейся причине: формат, codec, устройство, режим выполнения.
- **O — Open/Closed.** Новый codec/container/backend добавляется отдельным модулем и capability-реализацией; не раздуваем один центральный `match` во всех слоях. Где dispatch обязан быть явным, держим его в одном registry/entrypoint.
- **L — Liskov Substitution.** Интерфейсы описывают реальные обещания: drain/flush, timestamps, format-change, cancellation и ownership. Не объявлять backend взаимозаменяемыми, если один теряет кадры, не поддерживает формат или имеет другие гарантии памяти.
- **I — Interface Segregation.** Раздельные узкие контракты для demux, decoder, filter, encoder, mux, device transfer и telemetry. Не вводить монолитный trait, который вынуждает простой CPU filter реализовывать работу с файлами и GPU.
- **D — Dependency Inversion.** Внутреннее ядро зависит от доменных типов/контрактов; platform, GPU, libav, UI и CLI — внешние адаптеры. Feature flags управляют сборкой адаптеров, но не должны менять семантику core API.
- **DRY без преждевременной унификации.** Общими делать проверяемые механизмы (checked allocation, rational/timebase, buffer lease, диагностический контекст) только при совпадающей семантике. Не сводить codec-specific state machines или различающиеся container правила к универсальной «магической» абстракции. Не создавать общий `utils.rs` как бесхозную свалку.

## Порядок изменений

1. **Сделано частично.** Корневой фасад `fvid` сохранён; добавлены `fvid::y4m`, `fvid::edit` и сгруппированные `fvid::playback::{video,audio,subtitle_tracks,runtime}`. `fvid::edit` пока содержит только модель операций и объяснимый выбор между packet-copy и decode/process/encode; выполнение будет подключено после стабилизации format/runtime контрактов. Старые module paths остаются рабочими. Следующее — так же упорядочить `format` API и сократить случайную публичность implementation modules.
2. **Сделана физическая группировка.** Playback reader-файлы разложены в `playback/{video,audio,subtitles,runtime}/`; legacy root declarations указывают на эти пути. Внутри групп API пока реэкспортирует root-модули для совместимости; дальнейшее сужение visibility требует отдельного migration plan.
3. Разделить верхние уровни API/CLI: библиотечные команды и файловая публикация — в application/binary слое, не в codec или core.
4. Изолировать legacy `fvid-media` за feature/адаптером и постепенно заменить необходимую функциональность собственными format/codec путями. Не смешивать его типы с `fvid-core` без явного mapping.
5. Извлечь `fvid-core` тогда, когда появится второй независимый consumer или адаптер и контракт типов стабилизируется; после этого — processing/runtime. Добавлять crates по одному, со сборкой без default features и dependency checks.
6. Для каждого шага сохранить golden/corpus тесты, feature matrix, `cargo test` с `--no-default-features` и целевые benchmarks. Рефакторинг структуры не должен незаметно менять формат, точность времени, цвет, память или поведение EOS.

## Что пока не делать

Не создавать crate на codec, не вводить динамические плагины, общий граф/JIT, собственный async runtime или единый mega-trait. Не копировать в core системные зависимости CUDA/Metal/wgpu/libav. Не «DRY-ить» стандартные реализации друг об друга: кодеки и контейнеры похожи по форме, но различаются семантикой и требованиями корректности.
