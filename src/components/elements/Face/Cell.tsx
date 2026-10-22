import type { ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import type { TKey } from '@/i18n'
import { cx } from '@/utils/cx'
import { useCopied } from '@/hooks/useCopied'
import Copied from '../Copied'
import { PRESS } from '.'
import CopyHint from './CopyHint'

// A face is a miniature, so its captions sit one step under the detail rows'
// 11px label tier and are tracked a little tighter. A caption is also what makes
// a field as wide as it is — "PERSONAL NO." is wider than any number under it —
// so this size is what decides how many fields fit across a document.
export const CAPTION = 'text-2xs uppercase tracking-[0.1em]'

interface Props {
  /** The draft key — `entry-value-<name>` is the selector the e2e suite owns. */
  name: string
  /** Untranslated: the caption over the value, and what names the control. */
  label?: TKey
  /**
   * A caption that is not the label — the hash a fingerprint is cut with,
   * already spelled. Given, it replaces the label as the caption ('' for none)
   * while the label still names the control.
   */
  caption?: string
  /** What is stored, and what a click copies. */
  value: string
  /** What is shown, when that differs: a date in the user's pattern, a mask. */
  display?: ReactNode
  /**
   * The value line's type, leading included. The name and the number are the
   * big ones; a digest is the mono tier.
   */
  ink?: string
  /** Wrap rather than truncate: a document number is never worth clipping. */
  wrap?: boolean
  className?: string
}

// One printed value on a face: a micro-label, the value, and a copy on click —
// the same gesture every face teaches, confirmed in place by the shared mark
// in the face's own ink (see `Copied`), hinted at by the glyph in the corner.
export default function Cell({
  name,
  label,
  caption,
  value,
  display = value,
  ink = 'text-base leading-tight',
  wrap,
  className
}: Props) {
  const { t } = useTranslation()
  const { copied, copy } = useCopied()
  const named = label ? t(label) : caption
  const head = caption ?? named

  return (
    <button
      type="button"
      onClick={() => copy(value)}
      title={t('Copy')}
      aria-label={named ? `${named} · ${t('Copy')}` : t('Copy')}
      className={cx(PRESS, '-mx-1.5 -my-0.5 min-w-0 px-1.5 py-0.5', className)}
    >
      {head && <span className={`block truncate ${CAPTION} text-(--face-ink2)`}>{head}</span>}
      <span
        className={cx('relative block min-w-0', wrap ? 'break-all' : 'truncate', ink)}
        data-testid={`entry-value-${name}`}
      >
        <Copied copied={copied} ink="text-(--face-ink)">
          {display}
        </Copied>
      </span>
      <CopyHint copied={copied} />
    </button>
  )
}
