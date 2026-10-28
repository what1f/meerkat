# /// script
# requires-python = ">=3.11"
# dependencies = ["fonttools[woff]>=4.55"]
# ///
"""Subset the site fonts to the copy actually used.

Geist Mono (Latin) and Noto Sans SC (Chinese), both OFL, are fetched into
.cache/fonts and written to public/fonts as small variable woff2 files.
Run `pnpm fonts` after changing copy.
"""
import re
import urllib.request
from pathlib import Path

from fontTools import subset
from fontTools.ttLib import TTFont
from fontTools.varLib import instancer

ROOT = Path(__file__).resolve().parent.parent
CACHE = ROOT / ".cache" / "fonts"
OUT = ROOT / "public" / "fonts"
SOURCES = {
    "GeistMono.ttf": "https://github.com/google/fonts/raw/main/ofl/geistmono/GeistMono%5Bwght%5D.ttf",
    "NotoSansSC.ttf": "https://github.com/google/fonts/raw/main/ofl/notosanssc/NotoSansSC%5Bwght%5D.ttf",
}


def fetch(name: str) -> Path:
    path = CACHE / name
    if not path.exists():
        CACHE.mkdir(parents=True, exist_ok=True)
        urllib.request.urlretrieve(SOURCES[name], path)
    return path


def copy_chars() -> set[str]:
    chars: set[str] = set()
    for file in (ROOT / "src").rglob("*"):
        if file.suffix in {".astro", ".ts", ".md"}:
            chars |= set(file.read_text(encoding="utf-8"))
    return chars


def build(src: Path, out: Path, text: str, wght: tuple[int, int]) -> None:
    font = TTFont(src)
    opts = subset.Options()
    opts.flavor = "woff2"
    opts.layout_features = ["kern", "liga", "calt", "locl", "ccmp", "mark", "mkmk", "zero"]
    opts.name_IDs = [0, 1, 2, 3, 4, 5, 6]
    opts.notdef_outline = True
    sub = subset.Subsetter(opts)
    sub.populate(text=text)
    sub.subset(font)
    font = instancer.instantiateVariableFont(font, {"wght": wght})
    out.parent.mkdir(parents=True, exist_ok=True)
    font.flavor = "woff2"
    font.save(out)
    print(f"{out.relative_to(ROOT)}  {out.stat().st_size / 1024:.1f} KB")


def main() -> None:
    used = copy_chars()
    latin = "".join(chr(c) for c in range(0x20, 0x7F)) + "–—‘’“”…→←·×©®™€"
    cjk = "".join(sorted(c for c in used if ord(c) > 0x2E7F))
    cjk += "，。、；：？！“”‘’（）《》「」【】—…·"
    build(fetch("GeistMono.ttf"), OUT / "mono.woff2", latin, (400, 700))
    build(fetch("NotoSansSC.ttf"), OUT / "sc.woff2", cjk, (400, 700))


if __name__ == "__main__":
    main()
