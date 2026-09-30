#!/usr/bin/env python3
"""Inline KaTeX woff2 fonts into katex.min.css as data: URIs.

Why: dx/manganis only bundles files referenced via asset!(), and it leaves
url() references inside CSS untouched — so the relative font URLs in the
upstream katex.min.css would 404 in the built app. Inlining the woff2 files
(that browsers actually fetch; the woff/ttf fallbacks are unreachable in any
modern browser) makes the stylesheet fully self-contained.

Usage: from web/ run `python3 tools/gen_katex_inline.py` with
assets/katex/katex.min.css and assets/katex/fonts/*.woff2 present
(KaTeX 0.18.5 dist, the version katex-rs 0.3.0 tracks).
"""

import base64
import re
import sys
from pathlib import Path

WEB = Path(__file__).resolve().parent.parent
CSS = WEB / "assets/katex/katex.min.css"
FONTS = WEB / "assets/katex/fonts"

SRC_RE = re.compile(
    r'src:url\((?:/assets/katex/)?fonts/(?P<name>[A-Za-z0-9_-]+)\.woff2\) format\("woff2"\)'
    r',url\((?:/assets/katex/)?fonts/[^)]+\.woff\) format\("woff"\)'
    r',url\((?:/assets/katex/)?fonts/[^)]+\.ttf\) format\("truetype"\)'
)


def main() -> int:
    css = CSS.read_text(encoding="utf-8")
    matches = list(SRC_RE.finditer(css))
    if not matches:
        print("no font src descriptors matched — upstream css format changed?",
              file=sys.stderr)
        return 1
    missing = [m.group("name") for m in matches
               if not (FONTS / f"{m.group('name')}.woff2").exists()]
    if missing:
        print("missing woff2 files:", missing, file=sys.stderr)
        return 1

    def inline(m: re.Match[str]) -> str:
        data = (FONTS / f"{m.group('name')}.woff2").read_bytes()
        b64 = base64.b64encode(data).decode("ascii")
        return f'src:url(data:font/woff2;base64,{b64}) format("woff2")'

    out = SRC_RE.sub(inline, css)
    if "url(fonts/" in out or "url(/assets/katex/fonts/" in out:
        print("unhandled url(fonts/...) reference remains", file=sys.stderr)
        return 1
    CSS.write_text(out, encoding="utf-8")
    print(f"inlined {len(matches)} fonts into {CSS}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
