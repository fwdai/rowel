import { cx } from '@/utils/cx'
import { CopyGlyph } from '@/components/Main/icons'

// The one hint that a value is a copy control: a small copy glyph in the
// surface's own ink, dimmed, only while the pointer is over the pressable. It
// sits 6px in from the wash's right edge — the wash's own padding, so it is
// inset as far as the text is — in the top corner beside a caption, or centred
// on a bare line. The values themselves stay plain, so a card keeps looking
// like a card and a row like a row. A finger cannot hover, and never sees it:
// on touch the value is the copy control with no hint needed.
export default function CopyHint({ center, copied }: { center?: boolean; copied?: boolean }) {
  return (
    <span
      aria-hidden
      data-testid="copy-hint"
      className={cx(
        'pointer-events-none absolute right-1.5 opacity-0 transition-opacity',
        !copied && 'group-hover:opacity-60',
        center ? 'top-1/2 -translate-y-1/2' : 'top-1.5'
      )}
    >
      <CopyGlyph size={12} />
    </span>
  )
}
