#!/usr/bin/env python3
"""Rebuild benchmarks/klite-pack-components.json from codecguide's own pages.

The ability table says *what* K-Lite decodes; the contents comparison says *which component* of
which edition carries it, and the download page lists the capabilities a pack edition markets
without a filter of its own. The gate reads the result as the pack's component list, so this is
the one place the vendor's wording is transcribed — the file is regenerated, never hand-edited.
"""
import html, json, re, subprocess, sys
from common import ROOT

EDITIONS = ("basic", "standard", "full", "mega")
CONTENTS = "https://www.codecguide.com/klcp_contents_comparison.htm"
EDITIONS_PAGE = "https://www.codecguide.com/download_kl.htm"
OUT = ROOT / "benchmarks" / "klite-pack-components.json"
COLOR = {"#00FF00": "yes", "#FF0000": "no", "#CCCC00": "partial"}

# Capabilities the download page credits an edition with that no row of the contents table names.
# Kept here rather than scraped because the page states them as prose bullets.
MARKETING = [
    ("3D video decoding", "Plugin for 3D video decoding (H.264 MVC)",
     {"basic": "no", "standard": "no", "full": "yes", "mega": "yes"}),
    ("Playback capabilities", "HDR video playback",
     {"basic": "no", "standard": "yes", "full": "yes", "mega": "yes"}),
    ("Playback capabilities", "Subtitle display",
     {"basic": "no", "standard": "yes", "full": "yes", "mega": "yes"}),
    ("Playback capabilities", "Hardware accelerated video decoding",
     {"basic": "no", "standard": "yes", "full": "yes", "mega": "yes"}),
    ("Playback capabilities", "Audio bitstreaming",
     {"basic": "no", "standard": "yes", "full": "yes", "mega": "yes"}),
    ("Playback capabilities", "DVD and Blu-ray (after decryption)",
     {"basic": "no", "standard": "yes", "full": "yes", "mega": "yes"}),
    ("Shell integration", "Video thumbnails in Explorer",
     {"basic": "no", "standard": "yes", "full": "yes", "mega": "yes"}),
    ("Shell integration", "File association options",
     {"basic": "no", "standard": "yes", "full": "yes", "mega": "yes"}),
    ("Shell integration", "Broken codec detection",
     {"basic": "no", "standard": "yes", "full": "yes", "mega": "yes"}),
]


def get(url):
    result = subprocess_run(url)
    if result.returncode:
        raise SystemExit(f"cannot read {url}: {result.stderr.decode()[:120]}")
    return result.stdout.decode(errors="replace")


def subprocess_run(url):
    import subprocess
    return subprocess.run(["curl", "-sfL", "--max-time", "60", url], capture_output=True)


def cells(row):
    return [html.unescape(re.sub(r"<[^>]+>", " ", cell)).strip()
            for cell in re.findall(r"(?is)<t[dh].*?</t[dh]>", row)]


def contents(page):
    components, section = [], None
    for row in re.findall(r"(?is)<tr.*?</tr>", page):
        marks = re.findall(r'bgcolor="?(#[0-9A-Fa-f]{6})"?', row)
        if len(marks) != len(EDITIONS):
            label = cells(row)
            if len(label) == 1 and label[0]:
                section = label[0]
            continue
        name = re.sub(r"\s+", " ", cells(row)[0])
        components.append({"section": section, "component": name, "source": CONTENTS,
                           "editions": dict(zip(EDITIONS, (COLOR.get(m, "unknown") for m in marks)))})
    return components


def main():
    contents_page = get(CONTENTS)
    editions_page = get(EDITIONS_PAGE)
    components = contents(contents_page)
    for section, name, marks in MARKETING:
        components.append({"section": section, "component": name, "editions": marks,
                           "source": EDITIONS_PAGE})
    # Every marketing bullet must really be on the download page, so the transcription above is
    # checked against the wording rather than trusted.
    page_text = re.sub(r"\s+", " ", html.unescape(re.sub(r"<[^>]+>", " ", editions_page)))
    missing = [name for _, name, _ in MARKETING if name.split(" (")[0] not in page_text]
    if missing:
        raise SystemExit("the download page no longer says: " + ", ".join(missing))
    unknown = [c["component"] for c in components if "unknown" in c["editions"].values()]
    if unknown:
        raise SystemExit("unmapped edition colours: " + ", ".join(unknown))
    OUT.write_text(json.dumps({
        "sources": {"contents": CONTENTS, "editions": EDITIONS_PAGE},
        "fetched": "2026-09-24", "editions": list(EDITIONS), "components": components},
        indent=2, sort_keys=True) + "\n")
    print(f"{len(components)} pack components -> {OUT.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
