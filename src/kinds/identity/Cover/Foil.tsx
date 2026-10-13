import { useId } from 'react'

// Each pixel's luminance, into all three colour channels; alpha untouched.
const LUMINANCE = [
  '0.2126 0.7152 0.0722 0 0',
  '0.2126 0.7152 0.0722 0 0',
  '0.2126 0.7152 0.0722 0 0',
  '0 0 0 1 0'
].join(' ')

// Then that luminance onto the foil, shadow to highlight, per channel (0–1):
// #967028 where the artwork is black, #F6DE96 where it is white.
const FOIL = { r: '0.588 0.965', g: '0.439 0.871', b: '0.157 0.588' }

interface Props {
  /** The artwork, as a data: URI. */
  src: string
  className?: string
}

/**
 * An image stamped in gold foil, the way a passport's cover prints its coat of
 * arms: one metal, its light and shade taken from the artwork's own. Every
 * country's arms come out in the same material, and a black emblem becomes
 * dark gold instead of disappearing into a dark cover.
 *
 * An SVG filter on an inline `<image>` rather than a CSS filter: CSS has no
 * duotone (`sepia()` leaves black black), and WebKit cannot reference a filter
 * defined outside the page.
 */
export default function Foil({ src, className }: Props) {
  // useId's characters are not all valid in a `url(#…)` reference.
  const id = `foil${useId().replace(/[^\w-]/g, '')}`

  return (
    <svg aria-hidden viewBox="0 0 1 1" className={className}>
      <filter id={id} colorInterpolationFilters="sRGB">
        <feColorMatrix type="matrix" values={LUMINANCE} />
        <feComponentTransfer>
          <feFuncR type="table" tableValues={FOIL.r} />
          <feFuncG type="table" tableValues={FOIL.g} />
          <feFuncB type="table" tableValues={FOIL.b} />
        </feComponentTransfer>
      </filter>
      <image href={src} width="1" height="1" filter={`url(#${id})`} />
    </svg>
  )
}
