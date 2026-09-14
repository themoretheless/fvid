#!/usr/bin/env python3
"""Render a benchmark snapshot without altering its provenance."""
import json
from pathlib import Path
ROOT = Path(__file__).resolve().parents[1]
r = json.loads((ROOT/'benchmarks/media-benchmark.json').read_text())
s = '# Native media: CPU benchmark\n\n'
s += f"Снимок: {r['created_utc']}. По 30 исходных кадров 720p/1080p, проверочный прогрев и {r['runs']} замеров в перемешанном порядке. Все декодированные пиксели сверены перед замером каждого варианта; для remux также сжатые пакеты. Время всего CLI с чтением и записью, без fsync. Lossless-trim выбирает [0.2,0.8), остальные операции — всю длительность.\n\n"
s += '| Размер | Операция | Fvid, мс | FFmpeg default, мс | FFmpeg 1 thread, мс | Снижение времени против default |\n|---|---|---:|---:|---:|---:|\n'
for x in r['results']:
    m = x['median_ms']
    s += f"| {x['width']}×{x['height']} | {x['operation']} | {m['fvid']:.2f} | {m['ffmpeg_default']:.2f} | {m['ffmpeg_1thread']:.2f} | {(1-m['fvid']/m['ffmpeg_default'])*100:.1f}% |\n"
s += '\nЭто короткие CPU-тесты на одном хосте. Малые различия не доказывают устойчивого преимущества; необходимы длинные ролики, разнообразный контент и другие машины. Общая скорость FFmpeg и GPU этим тестом не квалифицированы. Предыдущий снимок до включения автоматического числа потоков кодеков сохранён отдельно; он относится к прежнему коду.\n\n[Сырые результаты, команды и provenance](media-benchmark.json) · [До исправления потоков](media-benchmark-before-threading.json) · [E2E-проверки](media-validation.json).\n'
(ROOT/'benchmarks/MEDIA_BENCHMARK.md').write_text(s)
