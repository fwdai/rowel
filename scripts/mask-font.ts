// Bakes the brand asterisk into src/assets/fonts/rowel-mask.otf: a font whose
// only glyph is the mask character. A password input draws its bullets through
// its own font, so setting this one on the master password field makes the
// browser itself draw the asterisk, with the field's text, caret, selection
// and scrolling all left native. Run with `bun run mask-font`.
//
// The glyph is the character-size variant of the body (MARK in
// asteriskGeometry.ts), the same shape AsteriskGlyph draws as SVG.
import { mkdirSync, writeFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import opentype from 'opentype.js'
import {
  CENTER,
  MARK,
  SPIKE,
  SPIKE_ANGLES
} from '@/components/elements/asteriskGeometry'

const OUT = resolve(import.meta.dir, '../src/assets/fonts/rowel-mask.otf')

const EM = 1000
// The advance of a typical monospace digit, so a masked character and a
// revealed one (set in the mono stack) are centered on the same spot of their
// cell. theme.css subtracts this same 0.6em from the pitch: keep them in step.
const ADVANCE = 600
// The geometry's 64-unit box, in font units: 8px at the field's 15px type,
// two thirds of the 12px pitch.
const BOX = (8 / 15) * EM
// The line is ascent 800 over descent 200, so 300 above the baseline is its
// middle: the glyph sits dead center in the field whatever the line height.
const ASCENT = 800
const DESCENT = -200
const MIDLINE = (ASCENT + DESCENT) / 2

type Point = [number, number]

// Geometry units (y down, centered on CENTER) to font units (y up).
const place = ([x, y]: Point): Point => [
  Math.round(ADVANCE / 2 + ((x - CENTER) * BOX) / 64),
  Math.round(MIDLINE - ((y - CENTER) * BOX) / 64)
]

// A point of the upright spike, slimmed about its axis and rotated into place.
const onSpike = (angle: number) => {
  const rad = (angle * Math.PI) / 180
  return (half: number, y: number): Point => {
    const dx = half * MARK.spike
    const dy = y - CENTER
    return place([
      CENTER + dx * Math.cos(rad) - dy * Math.sin(rad),
      CENTER + dx * Math.sin(rad) + dy * Math.cos(rad)
    ])
  }
}

const path = new opentype.Path()

// Every contour runs the same way round, so the overlaps between the spikes
// and the hub fill solid under the non-zero rule.
const { base, shoulder, tip } = SPIKE
for (const angle of SPIKE_ANGLES) {
  const at = onSpike(angle)
  path.moveTo(...at(-base.half, base.y))
  path.lineTo(...at(-shoulder.half, shoulder.y))
  path.quadraticCurveTo(...at(-tip.control, tip.y), ...at(-tip.half, tip.y))
  path.lineTo(...at(tip.half, tip.y))
  path.quadraticCurveTo(...at(tip.control, tip.y), ...at(shoulder.half, shoulder.y))
  path.lineTo(...at(base.half, base.y))
  path.close()
}

// The hub: a circle as four cubic arcs, wound the way the spikes are.
const r = MARK.hub
const k = r * 0.5523
const hub = (x: number, y: number) => place([CENTER + x, CENTER + y])
path.moveTo(...hub(0, -r))
path.curveTo(...hub(k, -r), ...hub(r, -k), ...hub(r, 0))
path.curveTo(...hub(r, k), ...hub(k, r), ...hub(0, r))
path.curveTo(...hub(-k, r), ...hub(-r, k), ...hub(-r, 0))
path.curveTo(...hub(-r, -k), ...hub(-k, -r), ...hub(0, -r))
path.close()

const font = new opentype.Font({
  familyName: 'Rowel Mask',
  styleName: 'Regular',
  unitsPerEm: EM,
  ascender: ASCENT,
  descender: DESCENT,
  // Windows reads the line from these two; left alone they are the glyph's own
  // bounds, which would sit it off the middle there.
  tables: { os2: { usWinAscent: ASCENT, usWinDescent: -DESCENT } },
  glyphs: [
    new opentype.Glyph({
      name: '.notdef',
      advanceWidth: ADVANCE,
      path: new opentype.Path()
    }),
    // What the engines substitute for a masked character: U+2022 in WebKit and
    // most of Chromium, U+25CF in Chromium on Windows.
    new opentype.Glyph({
      name: 'mask',
      unicodes: [0x2022, 0x25cf],
      advanceWidth: ADVANCE,
      path
    })
  ]
})

mkdirSync(dirname(OUT), { recursive: true })
const bytes = Buffer.from(font.toArrayBuffer())
writeFileSync(OUT, bytes)
console.log(`wrote ${OUT} (${bytes.length} bytes)`)
