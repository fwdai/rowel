import { cx } from '@/utils/cx'
import { CopyGlyph } from '@/components/Main/icons'

// The one hint that a value on a face is a copy control: a small copy glyph in
// the face's second ink, at the pressable's right edge, only while the pointer
// is over it — in the top corner beside a caption, centred on a bare line. The
// values themselves stay plain, so the card keeps looking like a card. A finger
// cannot hover, and never sees it — on touch the value is the copy control as
// it is on every row (see `fields/CopyValue`).
export default function CopyHint({ center }: { center?: boolean }) {
  return (
    <span
      aria-hidden
      className={cx(
        'pointer-events-none absolute right-1 text-(--face-ink2) opacity-0 transition-opacity group-hover:opacity-100',
        center ? 'top-1/2 -translate-y-1/2' : 'top-1'
      )}
    >
      <CopyGlyph size={12} />
    </span>
  )
}
