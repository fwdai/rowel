// The contacts as light falls on them: the same gold a passport's cover is
// stamped in (see `Cover/Foil`), cut by the lines between the pads.
const GOLD = ['#F6DE96', '#D9B76A', '#A9803A']
const CUT = '#8A6524'

/**
 * The gold contact pad of the chip a card carries: eight contacts round a
 * centre one. Decoration, like the portrait's silhouette — the vault holds no
 * chip — but it is the thing the back of a card has that nothing else does.
 */
export default function Chip({ className }: { className?: string }) {
  return (
    <svg aria-hidden viewBox="0 0 46 36" className={className}>
      <defs>
        <linearGradient id="chip-gold" x1="0" y1="0" x2="1" y2="1">
          <stop offset="0" stopColor={GOLD[0]} />
          <stop offset="0.5" stopColor={GOLD[1]} />
          <stop offset="1" stopColor={GOLD[2]} />
        </linearGradient>
      </defs>
      <rect x="0.5" y="0.5" width="45" height="35" rx="6" fill="url(#chip-gold)" stroke={CUT} />
      <g fill="none" stroke={CUT} strokeWidth="0.9">
        <rect x="16" y="11" width="14" height="14" rx="3" />
        <path d="M0.5 12.5H16M0.5 23.5H16M30 12.5H45.5M30 23.5H45.5M23 0.5V11M23 25V35.5" />
      </g>
    </svg>
  )
}
