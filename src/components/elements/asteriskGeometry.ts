// Geometry of the brand asterisk, shared by the AsteriskBody component (the
// lock-screen mascot's body, drawn live and softened by an SVG filter) and
// scripts/logo.tsx (which bakes the same softened outline, in the logo's
// slimmer variant, into the plain vector path at src/assets/images/logo.svg
// that the rail and everything outside the app display). Change the shape
// here and only here, then `bun run logo` and `bun run extension:icons`.
//
// A masked character, the secret you can't read. Drawn in a 64-unit box
// centered at 32: a round hub and six flared spikes at 60° whose tips reach
// 29 from center. Spike bases (corners 10 from center) are buried under the
// hub so the union fills solid.
export const CENTER = 32

// The mascot's hub is full enough to carry a face. Faceless uses can go
// smaller to open up the notches; below ~10.5 the spike bases start to show.
export const HUB_RADIUS = 14
export const SPIKE_ANGLES = [0, 60, 120, 180, 240, 300]

// One flared spike pointing up, as half-widths about the axis x = CENTER:
// narrow at the base, wide at the shoulder, then a rounded tip whose corners
// are quadratic curves from the shoulder to the flat top.
export const SPIKE = {
  base: { y: 24, half: 6 },
  shoulder: { y: 7, half: 8 },
  tip: { y: 3, half: 4.5, control: 8.5 }
}

const { base, shoulder, tip } = SPIKE
const L = CENTER - shoulder.half
const R = CENTER + shoulder.half
export const SPIKE_PATH =
  `M${CENTER - base.half} ${base.y} L${L} ${shoulder.y} ` +
  `Q${CENTER - tip.control} ${tip.y} ${CENTER - tip.half} ${tip.y} ` +
  `L${CENTER + tip.half} ${tip.y} Q${CENTER + tip.control} ${tip.y} ${R} ${shoulder.y} ` +
  `L${CENTER + base.half} ${base.y} Z`

// Per-spike transform: rotate into place and, for slimmer variants, scale the
// width about the spike's own axis so flare, tip rounding and reach stay put.
export const spikeTransform = (angle: number, width: number) =>
  `rotate(${angle} ${CENTER} ${CENTER})` +
  (width !== 1
    ? ` translate(${CENTER} 0) scale(${width} 1) translate(${-CENTER} 0)`
    : '')

// A point `distance` from the center along a spike's axis, for anchoring
// things to the body (the empty state's badge sits on a spike head).
export const pointOnSpike = (angle: number, distance: number) => {
  const rad = (angle * Math.PI) / 180
  return {
    x: +(CENTER + Math.sin(rad) * distance).toFixed(2),
    y: +(CENTER - Math.cos(rad) * distance).toFixed(2)
  }
}

// The soften filter: a Gaussian blur, then alpha remapped as
// `alpha * slope + offset` and clamped, which keeps everything above
// `-offset / slope` (0.43) coverage. Rounds every corner of the combined
// silhouette, including the concave notch bottoms that per-shape stroke joins
// cannot fillet, and grows edges by about 0.3 units.
export const SOFTEN = { blur: 1.8, slope: 30, offset: -13 }

// The logo's variant of the body (src/assets/images/logo.svg, baked by
// `bun run logo`): the mascot with all six spikes and no face, so it goes
// slimmer than the lock screen's to open the notches, but keeps enough hub
// to read as the same character.
export const LOGO = { hub: 13, spike: 0.8 }

// The character-size glyph's variant (AsteriskGlyph): a trimmed hub and
// slimmer spikes so the notches stay open at ten pixels.
export const MARK = { hub: 11, spike: 0.85 }
