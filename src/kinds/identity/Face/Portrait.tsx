import { useTranslation } from 'react-i18next'
import { CloseGlyph } from '@/components/Main/icons'

interface Props {
  /** The holder's photograph; without it the frame holds the silhouette. */
  src?: string
  /** Editing: takes the photograph off the document. */
  onRemove?: () => void
}

// Where the photograph goes: the one a scan cut out of the document, or — for
// a document typed in by hand, or scanned where no face was found — the
// silhouette every blank form prints there, shoulders cut by the frame's edge
// as a passport photo is. Either way it is what makes a face read as an ID at
// a glance rather than as a table of fields.
//
// A fraction of the document's own width, at the 3:4 a portrait photo is shot
// at, so it is the same size on the document however wide the pane is — which
// is how a photograph behaves on a card. Not a fraction of whatever height the
// fields came out at: the picture is part of the document, not a reaction to it.
// The scanner's crop is 3:4 too, so the photograph fills the frame uncut.
//
// A little over a fifth, rather than the ~28% a passport and an ID-1 card both
// really use: on an ID-1 the number and the three-line machine-readable band
// want those millimetres more than the picture does, and a narrower photo also
// leaves the fields beside it wide enough to set in two lines instead of three.
export default function Portrait({ src, onRemove }: Props) {
  const { t } = useTranslation()

  return (
    <div
      data-testid="identity-portrait"
      className="relative flex aspect-[3/4] w-[22%] flex-none items-end justify-center overflow-hidden rounded-[6px] border border-(--face-rule) bg-black/4 text-(--face-ink)"
    >
      {src ? (
        // Decorative to a screen reader: the holder is named beside it.
        <img src={src} alt="" draggable={false} className="h-full w-full object-cover" />
      ) : (
        <svg
          aria-hidden
          viewBox="0 0 64 60"
          className="h-[72%] w-auto opacity-15"
          fill="currentColor"
        >
          <circle cx="32" cy="17" r="14" />
          <path d="M4 60c0-15 12-26 28-26s28 11 28 26Z" />
        </svg>
      )}
      {src && onRemove && (
        <button
          type="button"
          title={t('Remove photo')}
          aria-label={t('Remove photo')}
          data-testid="remove-portrait"
          onClick={onRemove}
          className="absolute right-1 top-1 grid h-5 w-5 cursor-pointer place-items-center rounded-full bg-black/55 text-white transition-colors hover:bg-black/75 any-pointer-coarse:before:absolute any-pointer-coarse:before:-inset-3 any-pointer-coarse:before:content-['']"
        >
          <CloseGlyph />
        </button>
      )}
    </div>
  )
}
