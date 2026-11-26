import type { CSSProperties } from 'react'

// Gold foil as the light catches it: bright where it faces the light, top left,
// going dark across to the far corner.
const GOLD = 'linear-gradient(135deg, #F6DE96 0%, #D9B76A 45%, #967028 100%)'

const mask = (src: string, metal: string): CSSProperties => {
  const image = `url("${src}")`
  return {
    backgroundImage: metal,
    maskImage: image,
    maskSize: 'contain',
    maskRepeat: 'no-repeat',
    maskPosition: 'center',
    WebkitMaskImage: image,
    WebkitMaskSize: 'contain',
    WebkitMaskRepeat: 'no-repeat',
    WebkitMaskPosition: 'center'
  }
}

interface Props {
  /** The emblem: a one-colour SVG, as a data: URI or an asset URL, used only
   * for its shape. */
  src: string
  /** What it is stamped in, as a CSS gradient: gold unless said. */
  metal?: string
  testid?: string
  className?: string
}

/**
 * A shape stamped in foil, the way a passport's cover prints its emblem in gold
 * and a card carries it as a hologram.
 * The emblem is only the stencil (see `scripts/emblems/build.py`): it carries
 * no colour of its own, so every country comes out in the same metal.
 */
export default function Foil({ src, metal = GOLD, testid, className }: Props) {
  return <div aria-hidden data-testid={testid} className={className} style={mask(src, metal)} />
}
