# /// script
# requires-python = ">=3.11"
# dependencies = ["numpy", "pillow", "potracer", "scipy"]
# ///
"""
Builds the passport-cover emblems the app fetches from the website
(https://rowel.app/assets/passport/emblems/<ISO>.svg): each country's coat of
arms reduced to the symbol a passport stamps in foil, as a one-colour SVG.

    uv run scripts/emblems/build.py --out <dir>

Then copy <dir> into rowel-web as public/assets/passport/emblems, and bump
`SET` in src-tauri/src/emblem.rs: the app caches an emblem for good, so that
is how it learns the set has changed.

For every ISO 3166-1 alpha-3 code `src/utils/countries.ts` knows, the arms are
Wikidata's "coat of arms image" (P94) unless `overrides.json` names another
Commons file. Each is rendered by Commons as a PNG and then:

1. its field goes — the single-colour region that is the emblem's own shape
   (Poland's red shield, Vietnam's red disk) with the symbol set inside it —
   all of its colour within that outline, so only the eagle or the star is
   left, and the pockets of field between a trident's arms are open;
2. every line where one colour meets another is cut back in, and the art's
   own thin outlines with them, so each part of the arms stands on its own in
   the foil the way relief reads on a cover, instead of merging into a blob;
3. the result is traced into a single even-odd path and cropped to it.

The app paints the shape in gold; the SVG carries no colour of its own. Every
step can be adjusted per country in `overrides.json`:

    "UKR": { "file": "Lesser Coat of Arms of Ukraine.svg", "field": false, "lines": false }
    "SVK": { "drop": ["red"] }

`drop` names the heraldic colours to remove instead of finding the field —
for a field the symbol touches the edge of, which is then kept as an engraved
shield rather than dropped: red, blue, green, black, white, gold. `lines` is
how hard a change of colour has to be to be cut (`EDGE` by default; higher
for art so heavily shaded that its lines would dissolve it, false for none).
`"skip": true` leaves a country out.

Alongside the SVGs it writes `credits.json` (each source file's author and
licence, from Commons: derived from CC-BY-SA art, ours need the credit too),
and contact sheets to review every country on its own cover colour.
"""

import argparse
import hashlib
import html
import json
import re
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
from concurrent.futures import ProcessPoolExecutor
from pathlib import Path

import numpy as np
import potrace
from PIL import Image, ImageDraw
from scipy import ndimage
from scipy.spatial import ConvexHull

ROOT = Path(__file__).resolve().parents[2]
COUNTRIES = ROOT / "src/utils/countries.ts"
COVERS = ROOT / "src/kinds/identity/Cover/colors.ts"
OVERRIDES = Path(__file__).with_name("overrides.json")

USER_AGENT = "Rowel-Password-Manager (https://rowel.app)"
# Wide enough for the line work to survive the extraction.
RENDER_WIDTH = 500
# What the symbol is traced at: enough to keep a two-pixel gap between two
# parts of the arms a gap, short of the full render the path would only grow at.
TRACE_SIZE = 400
# How hard a change of colour has to be to be cut as a line (see
# `boundaries`): shading stays below it, an edge between two colours does not.
EDGE = 90
# Between requests to Commons, which answers a burst with 429.
PAUSE = 1.5

QUERY = """
SELECT ?iso3 (MIN(?file) AS ?arms) WHERE {
  ?country wdt:P298 ?iso3 ; wdt:P94 ?image .
  FILTER NOT EXISTS { ?country wdt:P576 [] }
  BIND(wikibase:decodeUri(STRAFTER(STR(?image), "Special:FilePath/")) AS ?file)
}
GROUP BY ?iso3"""


def get(url: str) -> bytes:
    """GET with the backoff Commons asks for when it rate-limits."""
    for attempt in range(6):
        try:
            req = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
            with urllib.request.urlopen(req, timeout=60) as res:
                return res.read()
        except urllib.error.HTTPError as err:
            if err.code != 429 or attempt == 5:
                raise
            time.sleep(int(err.headers.get("Retry-After") or 0) or 10 * (attempt + 1))
    raise AssertionError("unreachable")


def known_codes() -> set[str]:
    codes = re.search(r"const CODES =\s*`([^`]*)`", COUNTRIES.read_text()).group(1)
    return {token[:3] for token in codes.split()}


def cover_colors() -> dict[str, str]:
    return dict(re.findall(r"([A-Z]{3}): '(#[0-9A-F]{6})'", COVERS.read_text()))


def sources(overrides: dict) -> dict[str, str]:
    """ISO alpha-3 → Commons file name, Wikidata's unless overridden."""
    url = "https://query.wikidata.org/sparql?" + urllib.parse.urlencode(
        {"query": QUERY, "format": "json"}
    )
    rows = json.loads(get(url))["results"]["bindings"]
    files = {row["iso3"]["value"]: row["arms"]["value"] for row in rows}
    files.update({code: o["file"] for code, o in overrides.items() if "file" in o})
    known = known_codes()
    skipped = {code for code, o in overrides.items() if o.get("skip")}
    return {code: files[code] for code in sorted(files) if code in known - skipped}


def render(file: str, cache: Path) -> Path:
    """Commons' PNG render of `file`, cached by file name across runs."""
    path = cache / f"{hashlib.sha1(file.encode()).hexdigest()[:16]}.png"
    if not path.exists():
        url = (
            "https://commons.wikimedia.org/wiki/Special:FilePath/"
            + urllib.parse.quote(file)
            + f"?width={RENDER_WIDTH}"
        )
        path.write_bytes(get(url))
        time.sleep(PAUSE)
    return path


def tinctures(rgb: np.ndarray) -> dict[str, np.ndarray]:
    """Each pixel's heraldic colour, by hue, saturation and value."""
    hsv = np.array(Image.fromarray(rgb).convert("HSV")).astype(float)
    hue, sat, val = hsv[..., 0] * 360 / 255, hsv[..., 1] / 255, hsv[..., 2] / 255
    vivid = (sat > 0.35) & (val > 0.25)
    return {
        "red": vivid & ((hue < 20) | (hue > 330)),
        "gold": vivid & (hue >= 20) & (hue < 70),
        "green": vivid & (hue >= 70) & (hue < 175),
        "blue": vivid & (hue >= 175) & (hue <= 330),
        "black": val < 0.2,
        "white": (sat < 0.15) & (val > 0.8),
    }


def symbol(
    png: Path, field: bool = True, lines: bool | int = True, drop: list[str] | None = None
) -> np.ndarray:
    """The emblem as a boolean mask: the symbol, without its field, its parts
    parted by the lines where one colour meets another."""
    rgba = np.array(Image.open(png).convert("RGBA"))
    rgb, opaque = rgba[..., :3].astype(float), rgba[..., 3] > 128

    keep = opaque.copy()
    if drop:
        colors = tinctures(rgba[..., :3])
        for name in drop:
            keep &= ~colors[name]
    elif field:
        found = find_field(rgb, opaque)
        if found is not None:
            keep &= ~found

    if lines is not False:
        keep &= ~boundaries(rgb, keep, EDGE if lines is True else lines)
    return without_specks(keep)


def find_field(rgb: np.ndarray, opaque: np.ndarray) -> np.ndarray | None:
    """The field the symbol is set on, every pixel of it — or None for arms
    that have none (an eagle on its own).

    The field is the big single-colour region that is the emblem's own shape:
    a shield or a disk, solid inside its outline (the share of its convex hull
    it fills — Vietnam's disk, notched by its cog, fills 0.79), with art of
    another colour set within it. A spread eagle or a
    trident is no such shape, which is what keeps Austria's black eagle from
    being taken for one. Once found, all of its colour inside its outline goes
    — the pockets of blue between a trident's arms too, which are separate
    regions of the same field.
    """
    total = opaque.sum()
    tones = np.array(Image.fromarray(rgb.astype(np.uint8)).quantize(colors=12, method=Image.Quantize.MEDIANCUT))
    tones[~opaque] = 255
    best, area = None, 0
    for tone in np.unique(tones[opaque]):
        regions, count = ndimage.label(tones == tone)
        sizes = ndimage.sum(np.ones_like(regions), regions, range(1, count + 1))
        for i in np.nonzero(sizes >= 0.10 * total)[0] + 1:
            # Judged by its core: a field can touch more of its own colour
            # through a thin join — Vietnam's red disk, its red ribbon — and
            # the two together are no shield.
            region = core(regions == i)
            size = region.sum()
            if size < 0.10 * total or size <= area:
                continue
            shape = ndimage.binary_fill_holes(region)
            inside = shape & opaque & ~region
            if inside.sum() < 0.01 * total or solidity(shape) < 0.72:
                continue
            best, area = (region, shape), size
    if best is None:
        return None
    region, shape = best
    color = rgb[region].mean(axis=0)
    return shape & opaque & (np.linalg.norm(rgb - color, axis=-1) < 48)


def core(region: np.ndarray) -> np.ndarray:
    """The largest piece of a region once thin joins are cut, at full size."""
    opened = ndimage.binary_opening(region, iterations=4)
    pieces, count = ndimage.label(opened)
    if count == 0:
        return opened
    sizes = ndimage.sum(opened, pieces, range(1, count + 1))
    largest = pieces == np.argmax(sizes) + 1
    # Grown back over what the opening shaved off its edge, but not past it.
    return ndimage.binary_dilation(largest, iterations=4) & region


def solidity(shape: np.ndarray) -> float:
    """How much of its convex hull a shape fills: ~1 for a shield or a disk."""
    points = np.argwhere(shape)
    if len(points) < 3:
        return 0.0
    return shape.sum() / ConvexHull(points).volume


def boundaries(rgb: np.ndarray, keep: np.ndarray, edge: int) -> np.ndarray:
    """Where the art changes colour inside what is kept, as thin lines: each
    element of the arms then stands on its own in the foil — the star apart
    from the disk, the cog's teeth apart from the rice — the way relief reads
    on a cover.

    A boundary is a hard change between pixels two apart, unsmoothed: a real
    edge keeps its full contrast there, where a shaded surface changes a little
    at a time and stays below it. The lines are as thin as that finds them and
    no thicker, and a fragment too short to be a line — the grain of a shaded
    eagle — is not cut, so detailed art keeps its lines without dissolving. The
    art's own dark outlines count too, where they run inside the symbol; at its
    edge a thin dark stroke is the symbol (Albania's feathers, Oman's khanjar).
    """
    step = np.zeros(keep.shape)
    for axis in (0, 1):
        ahead, behind = np.roll(rgb, -1, axis), np.roll(rgb, 1, axis)
        step = np.maximum(step, np.linalg.norm(ahead - behind, axis=-1))
    edges = without_specks((step > edge) & keep, smallest=20)

    lum = rgb @ np.array([0.2126, 0.7152, 0.0722])
    dark = (lum < 70) & keep
    inside = ndimage.binary_erosion(keep, iterations=3)
    outline = dark & ~ndimage.binary_opening(dark, iterations=2) & inside
    return edges | outline


def without_specks(mask: np.ndarray, smallest: int = 12) -> np.ndarray:
    """The mask without islands too small to be anything but noise — thin gold
    lines, unlike with an opening, survive."""
    regions, count = ndimage.label(mask)
    sizes = ndimage.sum(mask, regions, range(1, count + 1))
    return np.isin(regions, np.nonzero(sizes >= smallest)[0] + 1)


def svg(mask: np.ndarray) -> str | None:
    """The mask traced into one even-odd path, cropped to it."""
    rows, cols = np.nonzero(mask)
    if rows.size == 0:
        return None
    pad = 2
    top, left = max(rows.min() - pad, 0), max(cols.min() - pad, 0)
    crop = mask[top : rows.max() + pad + 1, left : cols.max() + pad + 1]
    scale = TRACE_SIZE / max(crop.shape)
    if scale < 1:
        size = (round(crop.shape[1] * scale), round(crop.shape[0] * scale))
        image = Image.fromarray((crop * 255).astype(np.uint8)).resize(size, Image.LANCZOS)
        crop = np.array(image) >= 128
    height, width = crop.shape

    n = lambda v: f"{v:.1f}".rstrip("0").rstrip(".")  # noqa: E731
    parts = []
    # potracer traces the False pixels; ours are True.
    for curve in potrace.Bitmap(~crop).trace(turdsize=4, alphamax=1.0, opttolerance=0.4):
        start = curve.start_point
        parts.append(f"M{n(start.x)} {n(start.y)}")
        for seg in curve.segments:
            end = seg.end_point
            if seg.is_corner:
                parts.append(f"L{n(seg.c.x)} {n(seg.c.y)}L{n(end.x)} {n(end.y)}")
            else:
                c1, c2 = seg.c1, seg.c2
                parts.append(f"C{n(c1.x)} {n(c1.y)} {n(c2.x)} {n(c2.y)} {n(end.x)} {n(end.y)}")
        parts.append("Z")
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {width} {height}">'
        f'<path fill-rule="evenodd" d="{"".join(parts)}"/></svg>\n'
    )


def build(job: tuple[str, str, dict]) -> tuple[str, str | None, np.ndarray | None]:
    code, png, options = job
    mask = symbol(
        Path(png), options.get("field", True), options.get("lines", True), options.get("drop")
    )
    return code, svg(mask), mask


def credits(files: dict[str, str]) -> dict[str, dict]:
    """Author and licence of every source file, from Commons' own metadata."""
    titles = {f"File:{file}": code for code, file in files.items()}
    out: dict[str, dict] = {}
    names = list(titles)
    for at in range(0, len(names), 50):
        url = "https://commons.wikimedia.org/w/api.php?" + urllib.parse.urlencode(
            {
                "action": "query",
                "format": "json",
                "prop": "imageinfo",
                "iiprop": "extmetadata|url",
                "iiextmetadatafilter": "Artist|LicenseShortName|LicenseUrl",
                "titles": "|".join(names[at : at + 50]),
            }
        )
        data = json.loads(get(url))
        normalized = {n["to"]: n["from"] for n in data["query"].get("normalized", [])}
        for page in data["query"]["pages"].values():
            title = normalized.get(page["title"], page["title"])
            info = (page.get("imageinfo") or [{}])[0]
            meta = info.get("extmetadata", {})
            text = lambda key: html.unescape(  # noqa: E731
                re.sub(r"<[^>]+>", "", meta.get(key, {}).get("value", ""))
            ).strip()
            out[titles[title]] = {
                "source": title,
                "url": info.get("descriptionurl", ""),
                "author": text("Artist"),
                "license": text("LicenseShortName"),
                "licenseUrl": text("LicenseUrl"),
            }
        time.sleep(PAUSE)
    return dict(sorted(out.items()))


def sheets(masks: dict[str, np.ndarray], originals: dict[str, Path], review: Path) -> None:
    """Every country twice — its source art, and the symbol as the app will
    paint it, in gold on its own cover — large enough to judge the detail,
    twenty-four to a sheet."""
    colors, tile, per_row, per_sheet = cover_colors(), 150, 6, 24
    codes = sorted(masks)
    for number, at in enumerate(range(0, len(codes), per_sheet), start=1):
        batch = codes[at : at + per_sheet]
        rows = -(-len(batch) // per_row)
        sheet = Image.new("RGB", (per_row * (2 * tile + 16) + 8, rows * (tile + 22) + 8), "#F4F4F5")
        draw = ImageDraw.Draw(sheet)
        for i, code in enumerate(batch):
            x, y = 8 + (i % per_row) * (2 * tile + 16), 8 + (i // per_row) * (tile + 22)
            cover = Image.new("RGBA", (tile, tile), colors.get(code, "#2B3240"))
            source = Image.open(originals[code]).convert("RGBA")
            source.thumbnail((tile - 12, tile - 12))
            sheet.paste(Image.new("RGB", (tile, tile), "#FFFFFF"), (x, y))
            sheet.paste(source, (x + (tile - source.width) // 2, y + (tile - source.height) // 2), source)
            foil = Image.new("RGBA", masks[code].shape[::-1], (217, 183, 106, 255))
            foil.putalpha(Image.fromarray((masks[code] * 255).astype(np.uint8)))
            foil.thumbnail((int(tile * 0.7), int(tile * 0.7)))
            cover.alpha_composite(foil, ((tile - foil.width) // 2, (tile - foil.height) // 2))
            sheet.paste(cover, (x + tile + 4, y))
            draw.text((x, y + tile + 4), code, fill="#000000")
        sheet.save(review / f"sheet-{number}.png")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--out", type=Path, required=True, help="where the SVGs go")
    parser.add_argument("--cache", type=Path, default=Path(tempfile.gettempdir()) / "rowel-emblems/cache")
    parser.add_argument("--review", type=Path, default=Path(tempfile.gettempdir()) / "rowel-emblems/review")
    parser.add_argument("--only", nargs="*", help="rebuild just these codes")
    args = parser.parse_args()
    for folder in (args.out, args.cache, args.review):
        folder.mkdir(parents=True, exist_ok=True)

    overrides = json.loads(OVERRIDES.read_text())
    files = sources(overrides)
    if args.only:
        files = {code: files[code] for code in args.only}

    originals = {}
    for i, (code, file) in enumerate(files.items(), start=1):
        try:
            originals[code] = render(file, args.cache)
        except urllib.error.HTTPError as err:
            print(f"{code}: {file}: {err}", file=sys.stderr)
        print(f"\rrendered {i}/{len(files)}", end="", file=sys.stderr)
    print(file=sys.stderr)

    jobs = [(code, str(png), overrides.get(code, {})) for code, png in originals.items()]
    masks = {}
    with ProcessPoolExecutor() as pool:
        for code, markup, mask in pool.map(build, jobs):
            if markup is None:
                print(f"{code}: nothing left of the emblem", file=sys.stderr)
                continue
            (args.out / f"{code}.svg").write_text(markup)
            masks[code] = mask

    if not args.only:
        (args.out / "credits.json").write_text(
            json.dumps(credits({c: files[c] for c in masks}), indent=2, ensure_ascii=False) + "\n"
        )
    sheets(masks, originals, args.review)
    print(f"{len(masks)} emblems → {args.out}; review sheets in {args.review}", file=sys.stderr)


if __name__ == "__main__":
    main()
