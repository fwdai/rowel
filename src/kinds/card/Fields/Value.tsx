import type { ReactNode } from 'react'
import { cx } from '@/utils/cx'
import { useTranslation } from 'react-i18next'
import type { TKey } from '@/i18n'

/** Dot masks used as empty-state placeholders on the card face. */
type Mask = '•••' | '••••' | '••/••' | '•••• •••• •••• ••••'
import { useCopied } from '@/hooks/useCopied'
import Copied from '@/components/elements/Copied'
import { PRESS } from '@/components/elements/Face'
import { CAPTION } from '@/components/elements/Face/Cell'
import CopyHint from '@/components/elements/Face/CopyHint'
import { useFields } from '@/components/elements/fields'
import { verbatimInput } from '@/components/elements/inputProps'
import { META_TYPE } from '@/components/elements/tokens'

/** A mask is decoration rather than copy, so it never goes through `t()`. */
const isMask = (placeholder: TKey | Mask): placeholder is Mask => placeholder.startsWith('•')

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
  className?: string
}

// One data point on the card face, in whichever mode the pane is in. Reading,
// the whole value is a click-to-copy target (the 1Password pattern — no
// per-field buttons, so the face stays a card, not a toolbar) and copy always
// copies the true value, masked or not; the shared mark confirms it in place
// in the card's own ink (see `elements/Copied`). Editing, the same slot is the
// input: the face IS the form. Every ink here is the face's (`--face-*`), so
// the card is one object with the other faces and never a palette of its own.
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
  className
}: Props) {
  const { t } = useTranslation()
  const { set, attempted } = useFields()
  const { copied, copy } = useCopied()

  const caption = label && (
    <span className={`block ${CAPTION} text-(--face-ink2)`}>{t(label)}</span>
  )

  if (set) {
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
            'block w-full min-w-0 truncate border-b bg-transparent outline-none transition-colors placeholder:text-(--face-ink2)',
            invalid
              ? 'border-(--face-bad)'
              : 'border-(--face-rule) focus:border-(--face-ink2)',
            label && 'mt-1',
            ink
          )}
        />
        {invalid && (
          <span className={`mt-1 block ${META_TYPE} text-(--face-bad)`}>{t('Required')}</span>
        )}
      </label>
    )
  }

  const line = (
    <span className={cx('relative flex min-w-0 items-center gap-1.5', label && 'mt-1')}>
      <Copied copied={copied} ink="text-(--face-ink)">
        <span className={cx('block truncate', ink)} data-testid={testid}>
          {display}
        </span>
        {flag}
      </Copied>
    </span>
  )

  if (!copyValue)
    return (
      <div className={cx('min-w-0 px-1.5 py-1 text-(--face-ink2)', className)}>
        {caption}
        {line}
      </div>
    )

  return (
    <button
      type="button"
      onClick={() => copy(copyValue)}
      title={t('Copy')}
      className={cx(PRESS, 'min-w-0 px-1.5 py-1', className)}
    >
      {caption}
      {line}
      <CopyHint center={!label} />
    </button>
  )
}
