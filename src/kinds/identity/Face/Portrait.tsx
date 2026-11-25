import { cx } from '@/utils/cx'

// Where the photograph goes. The vault holds no photo, so the frame holds the
// silhouette every blank form prints there — shoulders cut by the frame's edge,
// as a passport photo is. It is what makes a face read as an ID at a glance
// rather than as a table of fields.
//
// A fraction of the document's own width, at the 3:4 a portrait photo is shot
// at, so it is the same size on the document however wide the pane is — which
// is how a photograph behaves on a card. Not a fraction of whatever height the
// fields came out at: the picture is part of the document, not a reaction to it.
//
// A little over a fifth, rather than the ~28% a passport and an ID-1 card both
// really use: the number along the foot wants those millimetres more than the
// picture does, and a narrower photo also leaves the fields beside it wide
// enough to set in two lines instead of three.

interface Props {
  /** The photo's share of the document's width; a little over a fifth unless said. */
  className?: string
}

export default function Portrait({ className = 'w-[22%]' }: Props) {
  return (
    <div
      aria-hidden
      className={cx(
        'flex aspect-[3/4] flex-none items-end justify-center overflow-hidden rounded-[6px] border border-(--face-rule) bg-black/4 text-(--face-ink)',
        className
      )}
    >
      <svg viewBox="0 0 64 60" className="h-[72%] w-auto opacity-15" fill="currentColor">
        <circle cx="32" cy="17" r="14" />
        <path d="M4 60c0-15 12-26 28-26s28 11 28 26Z" />
      </svg>
    </div>
  )
}
