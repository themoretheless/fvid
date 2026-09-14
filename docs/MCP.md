# Fvid MCP

Опциональный модуль `fvid::mcp` и команда `fvid mcp`. Протокол и транспорты предоставляет официальный Rust SDK `rmcp 3.1.2`; обработчики напрямую вызывают Rust API Fvid и native media adapter. FFmpeg CLI в обработчиках не запускается.

## Сборка и локальный запуск

```sh
cargo build --release --features mcp
./target/release/fvid mcp --root /Users/themoretheless/Documents/ChatGPT/fvid --stdio
```

Feature `mcp` включает `media`, поэтому нужны FFmpeg headers/libraries и libclang, описанные в [MEDIA.md](MEDIA.md). Обычная сборка без `mcp` не подключает MCP/HTTP runtime. `--no-default-features --features mcp` оставляет CPU/native media и исключает GPU-адаптеры.

Stdio — JSON-RPC по stdin/stdout. Логи идут в stderr; stdout содержит только сообщения MCP. Сервер запускает MCP-клиент, а не интерактивная оболочка для ручного ввода команд.

Пример конфигурации клиента с секцией `mcpServers`:

```json
{
  "mcpServers": {
    "fvid": {
      "command": "/Users/themoretheless/Documents/ChatGPT/fvid/target/release/fvid",
      "args": ["mcp", "--root", "/Users/themoretheless/Documents/ChatGPT/fvid", "--stdio"]
    }
  }
}
```

Готовый пример: [client.json](../examples/mcp/client.json). Формат расположения конфигурации зависит от клиента; модуль не изменяет настройки приложений автоматически.

## Streamable HTTP

```sh
export FVID_MCP_TOKEN="$(openssl rand -hex 32)"
./target/release/fvid mcp --root /Users/themoretheless/Documents/ChatGPT/fvid --http 127.0.0.1:8787
```

Endpoint: `http://127.0.0.1:8787/mcp`. Нужен заголовок `Authorization: Bearer <значение FVID_MCP_TOKEN>`. Токен не передаётся в CLI args и не печатается сервером. HTTP слушает только loopback, проверяет Host/Origin и ограничивает тело запроса 1 MiB. Для простых ответов используется JSON; legacy session state отключён, жизненный цикл и согласование протокола ведёт SDK.

Это локальный MCP-сервер, не опубликованный плагин ChatGPT. Подключение из браузерного/облачного клиента требует доступного ему транспорта, а также решения для TLS/authentication и передачи медиафайлов. Публичный endpoint, OAuth, загрузка/скачивание файлов и установка плагина здесь не выполнялись. Stdio и локальный HTTP проверены протокольным клиентом; подключение из UI ChatGPT не проверено.

## Инструменты

| Инструмент | Что делает |
|---|---|
| `fvid_capabilities` | Инвентарь библиотек и список реализованных MCP-инструментов; не доказательство покрытия всех кодеков |
| `fvid_devices` | CPU и доступность Metal/CUDA/Vulkan/DX12/GL |
| `fvid_probe` | Информация о контейнере и дорожках |
| `fvid_remux` | Перепаковка/выбор дорожек без декодирования |
| `fvid_trim` | Строгий packet-copy рез; неподдерживаемые GOP/audio boundaries отвергаются |
| `fvid_concat` | Склейка совместимых потоков без перекодирования |
| `fvid_transcode_lossless` | FFV1/Matroska, crop/hflip/vflip, interval, PCM, главы |
| `fvid_transcode` | Явный энкодер и параметры, качество может быть lossy |
| `fvid_trim_pcm` | Обрезка PCM внутри пакетов, без копирования sample payload |
| `fvid_decode_audio` | Одна аудиодорожка в PCM с сохранением точности декодированных сэмплов |
| `fvid_process_y4m` | Rust CPU/GPU-фильтры и resident GPU chains |

Схемы публикуются через `tools/list`. `input`, `output`, `inputs` — пути на машине сервера, относительные к `--root` либо абсолютные внутри него. `streams` — индексы из probe. `crop` задаётся массивом `[x,y,width,height]`. Время `from`/`to` — строки десятичных секунд, без float. Неизвестные поля не принимаются.

Успешный результат содержит text и structuredContent. Операции записи возвращают `output` и `stats`; native-статистика совпадает с библиотечным API. Ошибки операций возвращаются как `isError: true`, ошибки маршрутизации/протокола — JSON-RPC errors.

Пример аргументов lossless-кропа:

```json
{"input":"input.mp4","output":"cropped.mkv","crop":[2,2,640,360],"hflip":true}
```

Пример GPU-цепочки:

```json
{
  "input":"input.y4m","output":"filtered.y4m","backend":"metal",
  "crop":[2,2,640,360],"hflip":true,"stages":[{"vflip":true}],"memory_mib":256
}
```

Первый stage задаётся верхними `crop/hflip/vflip`, затем применяются `stages` относительно результата предыдущего. Дополнительные stages требуют явного GPU backend и GPU-сборки. Счётчики `transfers` отражают API upload/download/filter calls, а не аппаратную трассировку. CPU или одиночный stage возвращает `transfers: null`. Native codecs остаются CPU-путём; GPU codec interop этим модулем не добавлен.

## Границы исполнения

- Все верхнеуровневые пути канонизируются и проверяются относительно root, включая разрешение симлинков. Папка результата должна существовать, сам результат — отсутствовать. Проверки не являются OS sandbox против конкурентных изменений файловой системы; рабочая папка должна контролироваться владельцем сервера.
- В MCP native-входы ограничены самостоятельными контейнерами: MOV/MP4, Matroska/WebM, WAV, MP3, FLAC, Ogg, AAC, AVI, MPEG/TS, ASF, FLV, Y4M, PNG/JPEG pipe. Playlist/manifest demuxers исключены до чтения stream info, чтобы они не обходили проверку путей через вложенные источники. CLI вне MCP сохраняет свой прежний набор форматов.
- Выход публикуется после успешного завершения без перезаписи. Частичные файлы удаляются при обычных ошибках. Crash durability/fsync не гарантированы.
- Число одновременно исполняемых операций задаёт `--jobs 1..32` (по умолчанию 1), общее для процесса и всех HTTP-клиентов. Например, `fvid mcp --root "$PWD" --stdio --jobs 2` допускает две независимые обработки. Другие вызовы сразу получают busy; очередь ожидающих работ не создаётся. Кодеки работают в blocking worker; отмена запроса/отключение клиента не прерывает уже начатую native-операцию. Она может закончить и опубликовать результат. Permit остаётся занят до фактического завершения, поэтому отмена не запускает работу сверх лимита. Паника/ошибка worker освобождает слот. Async-транспорт работает отдельно от blocking workers; ping и получение схем не занимают слоты обработки. Дополнительные blocking threads оставлены для I/O транспорта.
- Параллельность относится к независимым вызовам: один видеопоток не разбивается на параллельные кадры. У каждого вызова собственные codec/GPU state и буферы. `memory_mib` остаётся лимитом одного Y4M-вызова, суммарный расход растёт с `jobs`; это не общий RSS-бюджет. Внутренние потоки кодеков также могут конкурировать за CPU. Увеличение jobs не гарантирует ускорение. Конкурирующие записи в один путь завершаются одной успешной атомарной публикацией, остальные получают ошибку.
- Через MCP доступны только scalar encoder options `crf`, `preset`, `tune`, `lossless`, `deadline`, `cpu-used`, `threads`, `bf`, `g`, `level`. Опции с путями/внешними конфигурациями не передаются. Неиспользованные параметры отвергает native adapter.
- `memory_mib` ограничивает контролируемые Y4M frame/staging buffers. Это не общий лимит памяти native decoder/encoder. Ограничения качества/времени операций сохраняются из [MEDIA.md](MEDIA.md).

## Проверки

```sh
cargo test --offline --all-features
cargo clippy --offline --all-targets --all-features -- -D warnings
cargo check --offline --no-default-features --features mcp
python3 scripts/validate_mcp.py --binary target/release/fvid --transport stdio
python3 scripts/validate_mcp.py --binary target/release/fvid --transport http --gpu metal
```

HTTP-тест требует права открыть локальный порт, Metal-тест — доступа к физическому GPU. Fixtures создаются временно. Проверяется MCP negotiation версии 2025-11-25, ping, schemas всех 11 tools, фактическое выполнение операций, packet/pixel/sample equality, busy, root/symlink/playlist restrictions, no-clobber, malformed arguments, Host/Origin/auth/version rejection. Новейшие протокольные возможности SDK не считаются проверенными только из-за его версии.

Снимки: [stdio](../benchmarks/mcp-validation-stdio.json), [HTTP + Metal](../benchmarks/mcp-validation-http-metal.json). JSON содержит хеш бинарника, исходников и список проверенных сценариев.

Источники протокола: [Transports](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports), [Lifecycle](https://modelcontextprotocol.io/specification/2025-11-25/basic/lifecycle).


Конкурентный режим проверяется командой:

```sh
python3 scripts/validate_mcp.py --binary target/release/fvid --transport stdio --jobs 2
```

[Отчёт jobs=2](../benchmarks/mcp-validation-stdio-jobs2.json) включает два успешных одновременных вызова, отказ лишнему, ping, сравнение выходных пикселей и конкурентную публикацию одного имени файла. Детерминированный Rust-тест дополнительно удерживает два worker на барьерах, проверяет совместный лимит между клонами сервера, отзыв async handle без преждевременного освобождения слота и восстановление после паники worker. [HTTP/Metal с jobs=2](../benchmarks/mcp-validation-http-jobs2-metal.json) проверяет протокол и GPU-обработку с новой конфигурацией; одновременные HTTP-запросы этим сценарием не измеряются.
