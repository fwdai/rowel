import type { ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import { useCopied } from '@/hooks/useCopied'
import { cx } from '@/utils/cx'
import Copied from '../Copied'
import CopyHint from '../Face/CopyHint'

interface Props {
  /** What pressing hands over — the real value, whatever is shown. */
  value: string
  /**
   * The accessible name. A masked value is a row of glyphs and names nothing,
   * so the row says what it is and what pressing does instead:
   * "Password · Copy".
   */
  label: string
  /** The row's label points here, so a click on it lands on the value. */
  id?: string
  testid?: string
  /** The value's own face: size, ink, tracking, or a chip row's layout. */
  className?: string
  children: ReactNode
}

/**
 * A read row's value, as the copy control it is — and the only one a row has:
 * a press hands the value over and the shared mark (see `elements/Copied`)
 * takes the value's place for a beat to say so. Under a pointer the value
 * takes the same wash and corner hint a cell on a face does (see
 * `Face/Cell`), so a row and a card teach one gesture; a rail holds only what
 * means something else — the eye, the open-link, the countdown. One element
 * for every row that shows a value — the typed fields, a custom pair, an env
 * variable, a key's comment, a scope list, the one-time code — so each is
 * copied the same way, on the desktop and on the phone alike.
 *
 * The wash pulls 6px into the gutter on either side so the text stays put, and
 * keeps 24px clear on the right for the hint, so it never sits on the value.
 */
export default function CopyValue({ value, label, id, testid, className, children }: Props) {
  const { t } = useTranslation()
  const { copied, copy } = useCopied()
  return (
    <button
      id={id}
      type="button"
      title={t('Copy')}
      aria-label={label}
      onClick={() => copy(value)}
      data-testid={testid}
      className={cx(
        'group relative -mx-1.5 cursor-pointer rounded-sm px-1.5 pr-6 text-left transition-colors hover:bg-hover',
        className
      )}
    >
      <Copied copied={copied} left="left-1.5">
        {children}
      </Copied>
      <CopyHint center />
    </button>
  )
}
