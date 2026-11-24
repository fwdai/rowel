import { describe, it, expect, beforeEach, afterEach } from 'vitest'
import { act, render, screen } from '@testing-library/react'
import * as ts from 'typescript'
import indexHtml from '../../index.html?raw'
import Main from '@/components/Main'
import { followSystemInsets } from '@/lib/android'
import { openAddPicker, openGenerator, flashCopied } from '@/store'
import { withEntries, loginMeta } from './utils'
import { setLayout } from './layout'

/**
 * The side insets — a cutout or a navigation bar down one side of a landscape
 * phone, the notch of an iPhone on its side — are kept by #root for whatever
 * is laid out in it, and by each surface positioned against the viewport for
 * itself, through `px-safe`, `inset-x-safe` or `mx-safe` (theme.css). Nothing
 * in jsdom lays a page out, so what is held here is that every such surface
 * says which: a new overlay that does not fails this rather than drawing its
 * controls under the cutout on a device nobody tested it on.
 */

const CLEARS = /(^|\s)(px-safe|inset-x-safe|mx-safe)(\s|$)/

// A class list that places its element against the viewport: `fixed`, with
// where (an inset or an edge) or how high (a z-index) — which is what tells it
// from the word in a sentence.
const viewportPlaced = (classes: string) => {
  const tokens = classes.split(/\s+/)
  return (
    tokens.includes('fixed') &&
    tokens.some(token => /^-?(inset|top|bottom|left|right|z)-/.test(token))
  )
}

// The ones that have no reason to stay clear of the sides, and why.
const EXEMPT: Record<string, string> = {
  // Invisible, behind an open menu: a tap anywhere outside the menu closes it,
  // beside a cutout as much as anywhere.
  'src/components/elements/Dropdown.tsx': 'fixed inset-0 z-10'
}

const sources = import.meta.glob<string>(['../**/*.{ts,tsx}', '!../**/*.test.{ts,tsx}', '!../test/**'], {
  query: '?raw',
  import: 'default',
  eager: true
})

// Every string a source file holds — literals, the text around a template's
// holes — read with the compiler, so a quote in a comment is not taken for one.
const strings = (path: string, text: string): string[] => {
  const found: string[] = []
  const visit = (node: ts.Node) => {
    if (ts.isStringLiteral(node) || ts.isNoSubstitutionTemplateLiteral(node)) found.push(node.text)
    else if (ts.isTemplateExpression(node))
      found.push([node.head.text, ...node.templateSpans.map(span => span.literal.text)].join(' '))
    ts.forEachChild(node, visit)
  }
  visit(ts.createSourceFile(path, text, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX))
  return found
}

describe('surfaces positioned against the viewport', () => {
  it('each keep clear of the side insets', () => {
    const offenders: string[] = []
    let seen = 0
    for (const [key, text] of Object.entries(sources)) {
      const path = `src/${key.replace(/^\.\.\//, '')}`
      for (const classes of strings(path, text).filter(viewportPlaced)) {
        seen++
        if (CLEARS.test(classes) || EXEMPT[path] === classes) continue
        offenders.push(`${path}: "${classes}"`)
      }
    }
    expect(offenders).toEqual([])
    // The scan is finding them at all: the frames, the toasts, the covers.
    expect(seen).toBeGreaterThanOrEqual(10)
  })

  // The launch splash is static markup over the page, laid out before any of
  // the above: it mirrors #root's side padding at every width.
  it('include the launch splash', () => {
    const rules = indexHtml.match(/\.splash\s*\{[^}]*\}/g) ?? []
    const sides = rules.filter(rule => /padding-left: var\(--safe-left/.test(rule))
    expect(sides.length).toBeGreaterThan(0)
    expect(rules.some(rule => /var\(--safe-left[^;]*\+ 1\.25rem/.test(rule))).toBe(true)
  })
})

// The same rule on a running phone shell, with the activity reporting a
// cutout down one side and a navigation bar down the other.
describe('the phone shell on its side', () => {
  beforeEach(() => {
    setLayout('compact')
    Object.defineProperty(window, 'devicePixelRatio', { value: 3, configurable: true })
    window.__ROWEL_ANDROID__ = {
      insets: () => '{"top":0,"right":144,"bottom":63,"left":96,"bar":0}',
      setDark: () => {},
      leave: () => {}
    }
  })

  afterEach(() => {
    delete window.__ROWEL_ANDROID__
    for (const side of ['top', 'right', 'bottom', 'left'])
      document.documentElement.style.removeProperty(`--safe-${side}`)
    document.documentElement.style.removeProperty('--bar-bottom')
  })

  it('gives every overlay it opens the side clearance', () => {
    followSystemInsets()
    expect(document.documentElement.style.getPropertyValue('--safe-left')).toBe('32px')
    expect(document.documentElement.style.getPropertyValue('--safe-right')).toBe('48px')

    withEntries([loginMeta({ id: 'l1', title: 'Google' })])
    render(<Main />)
    act(() => openAddPicker())
    act(() => openGenerator(() => {}))
    act(() => flashCopied())
    expect(screen.getByTestId('generator-dialog')).toBeInTheDocument()

    const placed = Array.from(document.querySelectorAll<HTMLElement>('[class]')).filter(el =>
      viewportPlaced(el.getAttribute('class') ?? '')
    )
    expect(placed.length).toBeGreaterThanOrEqual(3)
    for (const el of placed) expect(el.getAttribute('class')).toMatch(CLEARS)
  })
})
