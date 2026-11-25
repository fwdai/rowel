/**
 * The fine-line ground a card's stock is printed with: a field of shallow
 * waves, each a step out of phase with the one above, run edge to edge. It is
 * texture rather than artwork — no figure in it that a real card would have to
 * match — so it reads as security paper and says nothing of its own.
 *
 * In the units of an ID-1 card 460 wide.
 */

export const CARD = { width: 460, height: 290 }

const SPACING = 9
const DEPTH = 3.5
const LENGTH = 150
const DRIFT = 0.55
const STEP = 10

const wave = (row: number): string => {
  const y = row * SPACING
  let d = ''
  for (let x = 0; x <= CARD.width; x += STEP) {
    const lift = DEPTH * Math.sin((x / LENGTH) * Math.PI * 2 + row * DRIFT)
    d += `${x ? 'L' : 'M'}${x} ${(y + lift).toFixed(1)}`
  }
  return d
}

/** The whole field, as one path. */
export const GROUND = Array.from({ length: Math.ceil(CARD.height / SPACING) + 1 }, (_, row) =>
  wave(row)
).join('')
