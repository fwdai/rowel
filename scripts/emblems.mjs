// Regenerates src-tauri/src/emblem/emblems.json: which Wikimedia Commons file is
// each country's coat of arms, keyed by ISO 3166-1 alpha-3 code.
//
// Read from Wikidata's "coat of arms image" (P94) here, once, rather than by the
// app at run time: a wiki anyone can edit does not get to decide what the app
// shows, and the result is reviewed in a diff like any other change. Only the
// codes `src/utils/countries.ts` knows are kept, and dissolved states are left
// out, since an old state can still hold a code. Where Wikidata lists several
// images the first by name wins, so a rerun on the same data writes the same
// file. `OVERRIDES` wins over Wikidata, for the arms a cover does not print
// the way Wikidata draws them.
//
//   bun scripts/emblems.mjs

import { readFileSync, writeFileSync } from 'node:fs'

const OUT = 'src-tauri/src/emblem/emblems.json'

// India's lion capital is drawn in black, which disappears into its navy cover;
// the passport stamps it in gold.
const OVERRIDES = {
  IND: 'Emblem of India (gold).svg'
}

const QUERY = `
SELECT ?iso3 (MIN(?file) AS ?arms) WHERE {
  ?country wdt:P298 ?iso3 ; wdt:P94 ?image .
  FILTER NOT EXISTS { ?country wdt:P576 [] }
  BIND(wikibase:decodeUri(STRAFTER(STR(?image), "Special:FilePath/")) AS ?file)
}
GROUP BY ?iso3`

const known = new Set(
  readFileSync('src/utils/countries.ts', 'utf8')
    .match(/const CODES =\s*`([^`]*)`/)[1]
    .split(/\s+/)
    .filter(Boolean)
    .map(code => code.slice(0, 3))
)

const url = `https://query.wikidata.org/sparql?query=${encodeURIComponent(QUERY)}`
const res = await fetch(url, {
  headers: {
    Accept: 'application/sparql-results+json',
    'User-Agent': 'Rowel-Password-Manager (https://github.com/fwdai/rowel)'
  }
})
if (!res.ok) throw new Error(`Wikidata answered ${res.status}`)

const rows = (await res.json()).results.bindings
const emblems = Object.fromEntries(
  Object.entries({
    ...Object.fromEntries(rows.map(row => [row.iso3.value, row.arms.value])),
    ...OVERRIDES
  })
    .filter(([code]) => known.has(code))
    .sort(([a], [b]) => a.localeCompare(b))
)

writeFileSync(OUT, `${JSON.stringify(emblems, null, 2)}\n`)
console.log(`${Object.keys(emblems).length} emblems → ${OUT}`)
