import { CENTER, MARK, SPIKE_ANGLES, SPIKE_PATH, spikeTransform } from './asteriskGeometry'

interface Props {
  /** Pixels, or a CSS length ('0.65em') to follow the surrounding type. */
  size?: number | string
  className?: string
}

// The brand asterisk at character size: the mascot's body with all six spikes
// (no freed head), drawn from the shared geometry with no soften filter, since
// at this size the filter's rounding is under a pixel and a filter per cell
// would be the priciest thing in a row of forty. Faceless, so it takes the rail
// mark's trimmed hub and slimmer spikes to keep the notches open. Fills with
// currentColor so it colors like the text it stands in for.
export default function AsteriskGlyph({ size = 10, className }: Props) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 64 64"
      fill="currentColor"
      aria-hidden
      className={className}
    >
      {SPIKE_ANGLES.map(angle => (
        <path key={angle} d={SPIKE_PATH} transform={spikeTransform(angle, MARK.spike)} />
      ))}
      <circle cx={CENTER} cy={CENTER} r={MARK.hub} />
    </svg>
  )
}
