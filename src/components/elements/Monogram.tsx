import type { CSSProperties } from 'react'
import { isWorkspaceColor, WORKSPACE_COLOR_CLASSES } from '@/lib/workspaceColor'
import { cx } from '@/utils/cx'

interface Props {
  /** What the tile stands for; the first letter is what it shows. */
  name: string
  /** What the hue is derived from — an id, so a rename keeps the colour. */
  seed: string
  /** A chosen palette key; anything else falls back to the hue from `seed`. */
  color?: string
  size?: number
  // The letter's size; half the tile's unless a larger tile wants it lighter.
  fontSize?: number
  className?: string
}

// A stable hue for a string: the same seed always lands on the same colour,
// and neighbouring ids spread across the wheel rather than clustering.
const hueOf = (seed: string): number => {
  let hash = 5381
  for (let i = 0; i < seed.length; i++) hash = (hash * 33) ^ seed.charCodeAt(i)
  return Math.abs(hash) % 360
}

// A chosen colour is a solid tile with white ink and a glint along its top
// edge; without one it is the `monogram` tint in the seed's hue.
const tileClass = (color: string | undefined): string =>
  isWorkspaceColor(color)
    ? cx(WORKSPACE_COLOR_CLASSES[color].bg, 'text-white shadow-[inset_0_1px_0_rgba(255,255,255,.22)]')
    : 'monogram'

// An identity chip: the first letter of a name on a tile in a colour that is
// the thing's own, so a vault is told apart at a glance before it is read. The
// hashed tint, ink and hairline edge are the `monogram` utility in theme.css,
// themed there; only the hue is per instance, handed down as `--hue`. A chosen
// palette colour replaces all of that with its themed solid.
export default function Monogram({
  name,
  seed,
  color,
  size = 24,
  fontSize = Math.round(size * 0.5),
  className
}: Props) {
  const solid = isWorkspaceColor(color)
  return (
    <span
      aria-hidden
      className={cx(
        tileClass(color),
        'grid flex-none place-items-center rounded-[30%] font-semibold uppercase leading-none',
        className
      )}
      style={
        (solid
          ? { width: size, height: size, fontSize }
          : { width: size, height: size, fontSize, '--hue': hueOf(seed) }) as CSSProperties
      }
    >
      {name.trim().charAt(0)}
    </span>
  )
}
