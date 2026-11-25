/**
 * Guilloché: the woven fine-line engraving a card's background is printed
 * with. Drawn rather than shipped as artwork — each figure is one closed curve
 * and a weave is the same figure laid over itself a few phases apart, which is
 * how the lathe that gave the pattern its name cut it.
 *
 * Everything is in the units of an ID-1 card 460 wide (see `CARD`).
 */

export const CARD = { width: 460, height: 290 }

const TAU = Math.PI * 2

const trace = (steps: number, at: (turn: number) => [number, number]): string => {
  let d = ''
  for (let i = 0; i <= steps; i++) {
    const [x, y] = at(i / steps)
    d += `${i ? 'L' : 'M'}${x.toFixed(1)} ${y.toFixed(1)}`
  }
  return d
}

interface Rosette {
  x: number
  y: number
  radius: number
  /** How far the petals swing either side of the radius. */
  depth: number
  lobes: number
  /** How many copies are woven, each turned a step on from the last. */
  strands: number
}

/** A ring of petals, woven: the medallion at the heart of a banknote. */
export const rosette = ({ x, y, radius, depth, lobes, strands }: Rosette): string =>
  Array.from({ length: strands }, (_, strand) => {
    const phase = (strand / strands) * TAU
    return (
      trace(lobes * 16, turn => {
        const angle = turn * TAU
        const reach = radius + depth * Math.sin(lobes * angle + phase)
        return [x + reach * Math.cos(angle), y + reach * Math.sin(angle)]
      }) + 'Z'
    )
  }).join('')

interface Ribbon {
  y: number
  depth: number
  /** One wave's length along the card. */
  length: number
  strands: number
  /** A slow swell along the ribbon, so it thickens and thins as it runs. */
  swell?: number
}

/** A band of waves run edge to edge, woven the same way. */
export const ribbon = ({ y, depth, length, strands, swell = 0 }: Ribbon): string =>
  Array.from({ length: strands }, (_, strand) => {
    const phase = (strand / strands) * TAU
    return trace(Math.ceil((CARD.width / length) * 16), turn => {
      const x = turn * CARD.width
      const reach = depth * (1 + swell * Math.sin(turn * TAU))
      return [x, y + reach * Math.sin((x / length) * TAU + phase)]
    })
  }).join('')
