import type { ReactNode } from 'react'
import { cx } from '@/utils/cx'
import { useTranslation } from 'react-i18next'
import type { TKey } from '@/i18n'

/** Dot masks used as empty-state placeholders on the card face. */
type Mask = '•••' | '••••' | '••/••' | '•••• •••• •••• ••••'
import { useCopied } from '@/hooks/useCopied'
import { useFields } from '@/components/elements/fields'
import { verbatimInput } from '@/components/elements/inputProps'
import { CheckGlyph, CopyGlyph } from '@/components/Main/icons'
import { LABEL_TYPE, META_TYPE } from '@/components/elements/tokens'

/** A mask is decoration rather than copy, so it never goes through `t()`. */
const isMask = (placeholder: TKey | Mask): placeholder is Mask => placeholder.startsWith('•')

// Opaque version of the hover wash (white/10 composited over the card's
// ground), so a click doesn't visibly shift the ground but the value is fully
// hidden under the check — no text/icon overlap. The card face is a gradient,
// so the composite differs per zone: 'top' matches the lighter head of the
// card (the name), 'base' the dark body (number and bottom row).
function CopiedMark({ zone }: { zone: 'top' | 'base' }) {
  return (
    <span
      data-testid="copied-mark"
      className={`absolute -inset-x-1 inset-y-0 grid place-items-center rounded-[3px] text-white ${
        zone === 'top' ? 'bg-[#3F4247]' : 'bg-[#2B2D31]'
      }`}
    >
      <CheckGlyph size={12} />
    </span>
  )
}

interface Props {
  /** The input's `name` — the selector the e2e suite owns. */
  name: string
  /** Untranslated; the bottom row labels its values, the number and name don't. */
  label?: TKey
  /** What reading shows: masked, grouped, or the empty placeholder. */
  display: string
  /** What clicking copies. Empty leaves the read value inert. */
  copyValue: string
  /** Raw box contents while editing. */
  value: string
  onChange: (value: string) => void
  testid: string
  /**
   * Either a catalogue key ("MM/YY") or a dot mask ("••••"), which is
   * decoration rather than copy and has no entry to translate.
   */
  placeholder: TKey | Mask
  maxLength: number
  /** Type scale for the value line: the number is the big one. */
  ink: string
  /** Blocks the save while empty and marks the box after the first attempt. */
  required?: boolean
  /**
   * Overrides the box's own emptiness test, for a slot whose contents are more
   * than one draft key: a half-typed "MM" is not an empty box, but it is not a
   * saveable expiry either.
   */
  invalid?: boolean
  /** A marker beside the read value — the expiry's "Expired" pill. */
  flag?: ReactNode
  zone?: 'top' | 'base'
  className?: string
}

// One data point on the card face, in whichever mode the pane is in. Reading,
// the whole value is a click-to-copy target (the 1Password pattern — no
// per-field buttons, so the face stays a card, not a toolbar) and copy always
// copies the true value, masked or not. Editing, the same slot is the input:
// the face IS the form.
export default function Value({
  name,
  label,
  display,
  copyValue,
  value,
  onChange,
  testid,
  placeholder,
  maxLength,
  ink,
  required,
  invalid: invalidOverride,
  flag,
  zone = 'base',
  className
}: Props) {
  const { t } = useTranslation()
  const { set, attempted } = useFields()
  const { copied, copy } = useCopied()

  const caption = label && (
    <span className={`block ${LABEL_TYPE} opacity-50`}>{t(label)}</span>
  )

  if (set) {
    // The face's own error ink: `text-bad` is tuned for the app ground, not for
    // dark plastic, so the card uses a lighter red like every other hex on it.
    const invalid = invalidOverride ?? (required && attempted && !value.trim())
    return (
      <label className={cx('block min-w-0 px-1.5 py-1', className)}>
        {caption}
        <input
          name={name}
          value={value}
          placeholder={isMask(placeholder) ? placeholder : t(placeholder)}
          maxLength={maxLength}
          {...verbatimInput}
          onChange={event => onChange(event.target.value)}
          className={cx(
            'block w-full min-w-0 truncate border-b bg-transparent outline-none transition-colors placeholder:text-white/25',
            invalid ? 'border-[#FF8A8A]' : 'border-white/25 focus:border-white/70',
            label && 'mt-1',
            ink
          )}
        />
        {invalid && (
          <span className={`mt-1 block ${META_TYPE} text-[#FF8A8A]`}>{t('Required')}</span>
        )}
      </label>
    )
  }

  const line = (
    <span className={cx('relative flex min-w-0 items-center gap-1.5', label && 'mt-1')}>
      <span className={cx('block truncate', ink)} data-testid={testid}>
        {display}
      </span>
      {flag}
      {copied && <CopiedMark zone={zone} />}
    </span>
  )

  if (!copyValue)
    return (
      <div className={cx('min-w-0 px-1.5 py-1 opacity-50', className)}>
        {caption}
        {line}
      </div>
    )

  return (
    <button
      type="button"
      onClick={() => copy(copyValue)}
      title={t('Copy')}
      className={cx(
        'group relative min-w-0 cursor-pointer rounded-sm px-1.5 py-1 text-left transition-colors hover:bg-white/10',
        className
      )}
    >
      {caption}
      {line}
      {/* The only hint that the face is clickable — the values themselves stay
          plain, so the card keeps looking like a card. */}
      <span className="pointer-events-none absolute right-1 top-1/2 -translate-y-1/2 opacity-0 transition-opacity group-hover:opacity-60">
        <CopyGlyph size={12} />
      </span>
    </button>
  )
}
