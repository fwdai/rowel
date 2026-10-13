"""
Attribution for the published emblems: `credits.json` and the `credits.html`
page the website serves beside them.

Every emblem is derived from a Commons file, and most of those are under
licences that require credit — CC BY and CC BY-SA ask for the author, a link to
the licence, and a note of what was changed, and BY-SA asks for the derived
work to carry the same licence. So for each source this records its title and
page, its author, its licence and licence link, and what was done to it, and
publishes every emblem under its source's licence.

The author comes from Commons' metadata, then from the file page's own fields
(`|author=`, `|artist=`, `{{self|…|author=}}`) where the metadata has none, and
then from a reviewed `"credit"` override in `overrides.json`. A record that
needs attribution and still lacks an author or a licence link stops the build:
nothing is written until it is complete.
"""

import html
import json
import re
import time
import urllib.parse
from collections.abc import Callable

# Licences under which no credit is owed.
FREE = {"public domain", "cc0", "pd"}

# Commons licence names with no URL of their own, where credit is still owed.
NO_URL = {"attribution"}

MODIFICATIONS = (
    "Reduced to a one-colour symbol: the background field removed, colour "
    "boundaries and inner outlines cut as thin lines, and the result traced to "
    "a single vector path."
)

API = "https://commons.wikimedia.org/w/api.php?"


def attribution_required(record: dict) -> bool:
    name = record["license"].lower()
    return not any(name.startswith(free) for free in FREE)


def problems(record: dict) -> list[str]:
    """What an attribution-required record is missing, if anything."""
    if not attribution_required(record):
        return []
    missing = []
    if not record["author"]:
        missing.append("author")
    if not record["licenseUrl"] and record["license"].lower() not in NO_URL:
        missing.append("licence link")
    return missing


def wikitext_author(text: str) -> str:
    """The author a file page names in its own fields, as plain text."""
    found = re.search(r"\{\{\s*self\s*\|[^}]*?author\s*=\s*([^|}]+)", text, re.I)
    if not found:
        found = re.search(r"^\s*\|\s*(?:author|artist)\s*=\s*(.+?)\s*$", text, re.I | re.M)
    if not found:
        return ""
    value = found.group(1)
    # User links and user templates to their names, then drop the rest.
    value = re.sub(r"\{\{\s*(?:U|User|Uwt|Ult|Unt)\s*\|\s*([^|}]+)[^}]*\}\}", r"\1", value, flags=re.I)
    value = re.sub(r"\[\[[^|\]]*\|([^\]]+)\]\]", r"\1", value)
    value = re.sub(r"\[\[([^\]]+)\]\]", r"\1", value)
    value = re.sub(r"\{\{[^}]*\}\}", "", value)
    value = re.sub(r"<[^>]+>", "", value)
    return html.unescape(re.sub(r"\s+", " ", value)).strip(" ,;")


def plain(meta: dict, key: str) -> str:
    return html.unescape(re.sub(r"<[^>]+>", "", meta.get(key, {}).get("value", ""))).strip()


def fetch(files: dict[str, str], get: Callable[[str], bytes], pause: float) -> dict[str, dict]:
    """The attribution record of every source file, from Commons."""
    titles = {f"File:{file}": code for code, file in files.items()}
    names, out = list(titles), {}
    for at in range(0, len(names), 50):
        url = API + urllib.parse.urlencode(
            {
                "action": "query",
                "format": "json",
                "prop": "imageinfo|revisions",
                "iiprop": "extmetadata|url",
                "iiextmetadatafilter": "Artist|Attribution|LicenseShortName|LicenseUrl",
                "rvprop": "content",
                "rvslots": "main",
                "titles": "|".join(names[at : at + 50]),
            }
        )
        data = json.loads(get(url))
        normalized = {n["to"]: n["from"] for n in data["query"].get("normalized", [])}
        for page in data["query"]["pages"].values():
            title = normalized.get(page["title"], page["title"])
            info = (page.get("imageinfo") or [{}])[0]
            meta = info.get("extmetadata", {})
            revision = (page.get("revisions") or [{}])[0]
            text = revision.get("slots", {}).get("main", {}).get("*", "")
            out[titles[title]] = {
                "source": title,
                "url": info.get("descriptionurl", ""),
                "author": plain(meta, "Artist") or plain(meta, "Attribution") or wikitext_author(text),
                "license": plain(meta, "LicenseShortName"),
                "licenseUrl": plain(meta, "LicenseUrl"),
            }
        time.sleep(pause)
    return out


def complete(records: dict[str, dict], overrides: dict) -> dict[str, dict]:
    """The records with reviewed overrides applied, the modifications noted
    and each emblem's own licence set — or SystemExit listing what is missing."""
    out = {}
    for code, record in sorted(records.items()):
        record = {**record, **overrides.get(code, {}).get("credit", {})}
        record["modifications"] = MODIFICATIONS
        record["emblemLicense"] = record["license"]
        out[code] = record
    missing = {code: problems(r) for code, r in out.items() if problems(r)}
    if missing:
        lines = "\n".join(f"  {code}: no {', '.join(what)}  ({out[code]['url']})" for code, what in missing.items())
        raise SystemExit(
            "Attribution incomplete — add a reviewed \"credit\" override to overrides.json:\n" + lines
        )
    return out


def page(records: dict[str, dict]) -> str:
    """credits.html: every emblem's source, author, licence and changes."""
    e = html.escape
    rows = []
    for code, r in sorted(records.items()):
        licence = (
            f'<a href="{e(r["licenseUrl"])}">{e(r["license"])}</a>' if r["licenseUrl"] else e(r["license"])
        )
        rows.append(
            "<tr>"
            f'<td><img src="{e(code)}.svg" alt="" width="28" height="28"> {e(code)}</td>'
            f'<td><a href="{e(r["url"])}">{e(r["source"].removeprefix("File:"))}</a></td>'
            f"<td>{e(r['author']) or '—'}</td>"
            f"<td>{licence}</td>"
            "</tr>"
        )
    return f"""<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Passport emblems — credits · Rowel</title>
<style>
  body {{ font: 15px/1.5 system-ui, sans-serif; max-width: 960px; margin: 2rem auto; padding: 0 1rem; color: #1b1d21; }}
  table {{ border-collapse: collapse; width: 100%; }}
  th, td {{ text-align: left; padding: .4rem .6rem; border-bottom: 1px solid #e4e4e7; vertical-align: middle; }}
  td img {{ vertical-align: middle; filter: brightness(0); }}
  a {{ color: #1d4ed8; }}
</style>
</head>
<body>
<h1>Passport emblems — credits</h1>
<p>The emblems Rowel shows on passport covers are derived from coats of arms
published on <a href="https://commons.wikimedia.org/">Wikimedia Commons</a>.</p>
<p><strong>Changes made:</strong> {e(MODIFICATIONS)}</p>
<p>Each emblem here is made available under the same licence as the work it is
derived from, listed beside it. National emblems may also be protected by laws
on state insignia, independent of copyright.</p>
<table>
<thead><tr><th>Country</th><th>Source</th><th>Author</th><th>Licence</th></tr></thead>
<tbody>
{chr(10).join(rows)}
</tbody>
</table>
</body>
</html>
"""
