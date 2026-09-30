// Regenerates the browser extension's artwork (extension/rowel/icons) from
// Rowel's, with the same `tauri icon` rasterizer icons.mjs uses.
//
// The files are upstream's under Rowel's names (icons/rowel*.png and rowel.svg
// for keepassxc*; see extension/upstream-map.json), so an upstream sync only
// ever conflicts on the pixels. Two sources:
//   - the app icon (the mark on its light tile) for the extension icons, the
//     in-page field icon and the colored toolbar set;
//   - the bare mark, inked per toolbar theme, for the monochrome sets.
// The toolbar state badges (lock, cross, bang, the "update" sparkle) are lifted
// from upstream's own SVGs and laid over the Rowel art, so the script is
// idempotent and picks up upstream badge changes when re-run after a sync.
import { execFileSync } from 'node:child_process'
import { copyFileSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

const TILE = 'src-tauri/icons/asterisq.svg'
const MARK = 'src/assets/images/logo.svg'
const ICONS = 'extension/rowel/icons'
const TOOLBAR = join(ICONS, 'toolbar')

// Toolbar themes: `light` is drawn for light browser chrome (dark ink) and
// `dark` for dark chrome. `colored` is the default, full-color set.
const INK = { light: '#34373e', dark: '#f2f2f2' }
// Disconnected, locked and the idle "dark" state draw the logo greyed out.
const DIMMED = /cross|locked|dark/
const DIM_INK = '#8e8e8e'

const tauri = join('node_modules', '.bin', process.platform === 'win32' ? 'tauri.cmd' : 'tauri')
const tmp = mkdtempSync(join(tmpdir(), 'rowel-extension-icons-'))

const inner = svg => svg.replace(/^[\s\S]*?<svg[^>]*>/, '').replace(/<\/svg>\s*$/, '').trim()
const viewBox = svg => svg.match(/<svg[^>]*viewBox="([^"]+)"/)[1]
const tile = readFileSync(TILE, 'utf8')
const mark = readFileSync(MARK, 'utf8')

// The logo, drawn into a 64×64 coordinate space (every upstream icon's).
function logo(style, dimmed) {
  if (style === 'colored') {
    const art = `<svg width="64" height="64" viewBox="${viewBox(tile)}">${inner(tile)}</svg>`
    return dimmed ? `<g opacity=".5">${art}</g>` : art
  }
  const fill = dimmed ? DIM_INK : INK[style]
  return `<svg width="64" height="64" viewBox="${viewBox(mark)}" fill="${fill}">${inner(mark)}</svg>`
}

// [start, end) of each top-level element inside the root <svg>.
function topLevel(body) {
  const spans = []
  const tag = /<(\/?)([a-zA-Z][\w:.-]*)[^>]*?(\/?)>/g
  let depth = 0
  let open
  for (let m; (m = tag.exec(body)); ) {
    const [, closing, name, selfClosing] = m
    if (depth === 0 && !closing) open = { name, start: m.index }
    if (closing) depth--
    else if (!selfClosing) depth++
    if (depth === 0) spans.push({ ...open, end: tag.lastIndex })
  }
  return spans
}

// Upstream draws the KeePassXC logo as the first element that is not <defs>;
// everything else (gradients, the state badge) is kept as-is, in order.
function rebrand(source, style, dimmed) {
  const head = source.match(/<svg[^>]*>/)[0]
  const body = inner(source)
  const spans = topLevel(body)
  const target = spans.find(s => s.name !== 'defs')
  const parts = spans.map(s => (s === target ? logo(style, dimmed) : body.slice(s.start, s.end)))
  return `${head}${parts.join('')}</svg>\n`
}

// Width of a PNG, from its IHDR chunk.
const pngSize = file => readFileSync(file).readUInt32BE(16)

function rasterize(svgFile, size, pngFile) {
  const out = join(tmp, String(Math.random()).slice(2))
  execFileSync(tauri, ['icon', svgFile, '-o', out, '-p', String(size)], { stdio: 'ignore' })
  copyFileSync(join(out, `${size}x${size}.png`), pngFile)
}

try {
  // Toolbar sets: SVG for Firefox, PNG (same size as upstream's) for Chromium.
  for (const style of ['colored', 'dark', 'light']) {
    const dir = join(TOOLBAR, style)
    for (const file of readdirSync(dir).filter(f => f.endsWith('.svg'))) {
      const svg = join(dir, file)
      const png = svg.replace(/\.svg$/, '.png')
      writeFileSync(svg, rebrand(readFileSync(svg, 'utf8'), style, DIMMED.test(file)))
      rasterize(svg, pngSize(png), png)
    }
  }

  // In-page field icons: the plain tile, and the colored toolbar's states.
  writeFileSync(join(ICONS, 'rowel.svg'), tile)
  copyFileSync(join(TOOLBAR, 'colored', 'icon_cross.svg'), join(ICONS, 'disconnected.svg'))
  copyFileSync(join(TOOLBAR, 'colored', 'icon_locked.svg'), join(ICONS, 'locked.svg'))

  // Extension icons (manifest, notifications, options page favicons).
  for (const file of readdirSync(ICONS).filter(f => /^rowel(-dark)?_\d+x\d+\.png$/.test(f))) {
    const png = join(ICONS, file)
    rasterize(TILE, pngSize(png), png)
  }
} finally {
  rmSync(tmp, { recursive: true, force: true })
}
