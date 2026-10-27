import { describe, it, expect } from 'vitest'
import enUS from '@/i18n/locales/en-US.json'

/*
 * Copy hygiene, English only.
 *
 * en-US is the key catalogue: `t()` falls back to the key itself, so a key
 * missing there still renders — it just renders untranslatable, invisible to
 * every other locale. This is the producer-side check for that.
 *
 * Every translated locale is complete: a key en-US has and a locale lacks
 * renders in English there, which nobody using that locale asked for, so a PR
 * that adds a key translates it into all nine. A pluralised key only counts as
 * translated once the locale carries every plural category that language needs,
 * because a lone `_one` still falls back to English at every other count — and
 * only those: zh-CN carries `_other` alone.
 *
 * Any key a locale does carry has to keep en-US's placeholders, or the
 * interpolation silently drops a value.
 *
 * Sources are read through `import.meta.glob` rather than node:fs so the suite
 * needs no Node types in the app's tsconfig.
 */

const sources = import.meta.glob<string>(
  ['../**/*.{ts,tsx}', '!../test/**', '!../**/*.d.ts'],
  {
    query: '?raw',
    import: 'default',
    eager: true
  }
)

const locales = import.meta.glob<Record<string, string>>('../i18n/locales/*.json', {
  import: 'default',
  eager: true
})

// `t('…')`, `t("…")` and `t(`…`)` without interpolation. Dynamic `t(variable)`
// sites are out of reach here and stay the reviewer's job.
const CALL = /\bt\(\s*(?:'((?:[^'\\]|\\.)*)'|"((?:[^"\\]|\\.)*)"|`([^`$\\]*)`)\s*[,)]/g

// Every literal key, mapped to the first file that asks for it.
const literalKeys = (): Map<string, string> => {
  const found = new Map<string, string>()
  for (const [path, text] of Object.entries(sources))
    for (const match of text.matchAll(CALL)) {
      const key = (match[1] ?? match[2] ?? match[3]).replace(/\\(['"])/g, '$1')
      if (!found.has(key)) found.set(key, path)
    }
  return found
}

// `{{appName}}` is exempt: it is the app's name (a constant, see `@/lib/app`,
// supplied to every call through i18next's `defaultVariables`), not per-call
// data, so nothing is lost when a locale names the app where en-US does not —
// or says nothing about it where en-US does. Every other placeholder stands
// for a value the sentence needs, so it stays symmetric.
const placeholders = (value: string) =>
  [...value.replace(/\{\{appName\}\}/g, '').matchAll(/\{\{[^}]*\}\}/g)].map(m => m[0]).join()

const catalogue: Record<string, string> = enUS

// i18next resolves `t(key, { count })` against `${key}_zero|one|two|few|many|other`
// rather than the bare key, so a pluralized entry never appears under `key`
// itself in the catalogue.
const PLURAL_SUFFIXES = ['_zero', '_one', '_two', '_few', '_many', '_other']
const inCatalogue = (key: string) =>
  key in catalogue || PLURAL_SUFFIXES.some(suffix => `${key}${suffix}` in catalogue)

// A locale legitimately carries plural categories English does not have:
// English needs `_one`/`_other`, Russian and Polish need `_few` and `_many`
// too. Such a key is recognised by its base having a plural entry in en-US.
const PLURAL_SUFFIX = new RegExp(`(${PLURAL_SUFFIXES.join('|')})$`)
const knownKey = (key: string) => {
  if (key in catalogue) return true
  const base = key.replace(PLURAL_SUFFIX, '')
  return base !== key && PLURAL_SUFFIXES.some(suffix => `${base}${suffix}` in catalogue)
}

const translated = Object.entries(locales).filter(([path]) => !path.endsWith('en-US.json'))

const localeName = (path: string) => path.slice(path.lastIndexOf('/') + 1).replace('.json', '')

// The unit a key belongs to: a plural set collapses to its base, so en-US's
// `X_one`/`X_other` is one unit `X`.
const unit = (key: string) => key.replace(PLURAL_SUFFIX, '')

// The units en-US spells out as `X_one`/`X_other` rather than as a bare `X`.
const pluralUnits = new Set(
  Object.keys(catalogue)
    .map(unit)
    .filter(base => !(base in catalogue))
)

// The plural categories i18next will ask a locale for, taken from the same
// `Intl.PluralRules` it resolves `count` against so the expectation here cannot
// drift from the runtime: zh-CN needs only `_other`, French also `_many`,
// Russian and Polish `_few` and `_many` on top of `_one`/`_other`.
const pluralSuffixes = (locale: string): string[] =>
  new Intl.PluralRules(locale).resolvedOptions().pluralCategories.map(category => `_${category}`)

// A locale covers a plural unit only by carrying *every* form that language
// needs — one missing category is an English fallback at those counts.
const covers = (keys: Set<string>, name: string, key: string) =>
  pluralUnits.has(key)
    ? pluralSuffixes(name).every(suffix => keys.has(`${key}${suffix}`))
    : keys.has(key)

describe('i18n', () => {
  it('has an en-US entry for every literal t() key in src/', () => {
    const missing = [...literalKeys()]
      .filter(([key]) => !inCatalogue(key))
      .map(([key, path]) => `${JSON.stringify(key)} (${path})`)

    expect(missing).toEqual([])
  })

  it('keeps en-US placeholders in every locale that carries the key', () => {
    // A plural form English does not have (`_few`, `_many`) is compared against
    // whichever form en-US does define — it needs the same placeholders.
    const counterpart = (key: string): string | undefined => {
      if (key in catalogue) return catalogue[key]
      const base = key.replace(PLURAL_SUFFIX, '')
      if (base === key) return undefined
      const match = PLURAL_SUFFIXES.map(s => `${base}${s}`).find(k => k in catalogue)
      return match && catalogue[match]
    }

    const mismatched = translated.flatMap(([path, locale]) =>
      Object.entries(locale)
        .filter(([key, value]) => {
          const source = counterpart(key)
          return source !== undefined && placeholders(source) !== placeholders(value)
        })
        .map(([key, value]) => `${path}: ${JSON.stringify(key)} -> ${JSON.stringify(value)}`)
    )

    expect(mismatched).toEqual([])
  })

  it('has a translation for every en-US key in every locale', () => {
    const units = [...new Set(Object.keys(catalogue).map(unit))]
    const carried = translated.map(([path, locale]) => {
      const name = localeName(path)
      const keys = new Set(Object.keys(locale))
      return { name, units: new Set(units.filter(key => covers(keys, name, key))) }
    })

    const percent = (locale: (typeof carried)[number]) =>
      Math.round((units.filter(key => locale.units.has(key)).length / units.length) * 100)

    const coverage =
      `i18n coverage of ${units.length} en-US keys — ` +
      carried.map(locale => `${locale.name} ${percent(locale)}%`).join(', ')
    console.info(coverage)

    const untranslated = carried.flatMap(locale =>
      units.filter(key => !locale.units.has(key)).map(key => `${locale.name}: ${JSON.stringify(key)}`)
    )

    expect(untranslated, coverage).toEqual([])
  })

  it('carries complete plural sets', () => {
    // Stricter than the coverage gate above, and independent of it: a locale
    // that has started on a plural unit has to finish it, even where another
    // locale already covers that unit.
    const incomplete = translated.flatMap(([path, locale]) => {
      const name = localeName(path)
      const keys = new Set(Object.keys(locale))
      const required = pluralSuffixes(name)

      return [...pluralUnits]
        .filter(key => PLURAL_SUFFIXES.some(suffix => keys.has(`${key}${suffix}`)))
        .map(key => ({ key, missing: required.filter(suffix => !keys.has(`${key}${suffix}`)) }))
        .filter(({ missing }) => missing.length > 0)
        .map(({ key, missing }) => `${name}: ${JSON.stringify(key)} -> missing ${missing.join(', ')}`)
    })

    expect(incomplete).toEqual([])
  })

  it('carries no keys en-US has dropped', () => {
    const orphans = translated.flatMap(([path, locale]) =>
      Object.keys(locale)
        .filter(key => !knownKey(key))
        .map(key => `${path}: ${JSON.stringify(key)}`)
    )

    expect(orphans).toEqual([])
  })
})
