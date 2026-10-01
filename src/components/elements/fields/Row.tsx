import { useId, type ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import { cx } from '@/utils/cx'
import type { TKey } from '@/i18n'
import { LABEL, ROW_HAIRLINE } from '../tokens'
import { useFields } from './context'

interface Props {
  /** Untranslated. Omitted for a full-bleed row — a note body. */
  label?: TKey
  /** A sigil in front of the value: what makes a URL look like a URL. */
  prefix?: ReactNode
  /** Trailing controls: reveal, copy, open, generate. */
  actions?: ReactNode
  /** Full-width slot under the value: a strength bar, a rotation stamp. */
  below?: ReactNode
  /** Rendered under the value once there is something to complain about. */
  error?: string
  /**
   * Given the id the row's `<label>` points at, so the control it renders
   * carries an accessible name. A full-bleed row has no label to hand over and
   * names its own control instead.
   */
  children: (id: string) => ReactNode
}

// Where the value column starts: label 128 (w-32) + gap 12 + sigil 16 (w-4) + gap 12.
// Stacked, there is no label column in front of the value, so the slot under it
// starts at the row's own edge.
const VALUE_START = 'pl-[168px] @max-[420px]:pl-0'

/*
 * Below 420px of *container* the row folds instead of shrinking: the label
 * takes a line of its own (`w-full` on a wrapping flex line is the break), the
 * sigil goes with the column it was aligning to, and value + actions keep the
 * next line to themselves. No render branch — every kind's rows inherit it from
 * whichever surface declares itself a `@container` (see Show/Read).
 *
 * Exported for the rows that share this geometry without rendering through it
 * (CustomFields, the env table), so a change to the fold is one change.
 */
export const STACK = '@max-[420px]:flex-wrap @max-[420px]:gap-y-1.5'
export const STACK_LABEL = '@max-[420px]:w-full'
export const STACK_SIGIL = '@max-[420px]:hidden'
// The rail no longer has a fixed column to fit, so where a finger is one of the
// pointers its controls grow to the 44px target. Both conditions: an iPad
// running the wide shell keeps the 60px rail, which only holds two 28px
// buttons. `any-pointer-coarse` rather than `pointer-coarse` so a hybrid — an
// iPad with a trackpad, a touch laptop — is sized for the finger it also has.
export const STACK_RAIL =
  '@max-[420px]:w-auto @max-[420px]:any-pointer-coarse:[&_button]:h-11 @max-[420px]:any-pointer-coarse:[&_button]:w-11'

// The rail: two 28px controls wide (28 + gap 4 + 28), held open so every value
// — and every editor's underline — ends at one x too.
export const RAIL = 'flex w-[60px] flex-none items-center justify-end gap-1'

/*
 * The phone's row (`max-md:`, the compact shell's cut): a two-column grid,
 * the label over the value in the first column and the rail in the second,
 * spanning both lines and centred on the pair. Folded as a wrapped flex row
 * the rail's 44pt buttons sat on the value's own line, stretching it to 44px
 * and centring the value in it — a gap under every label. The fold's classes
 * above still apply underneath; a grid simply ignores the wrap. The rail's
 * glyphs are the prototype's 18px and its buttons touch, so two of them do
 * not read as a toolbar.
 */
// The grid does not lean on the fold: a landscape phone (or a narrow window)
// is under 768px with a container *over* 420px, where the fold's classes do
// not fire — so the sigil is hidden, the label and rail let go of their fixed
// columns and the slot under the value starts at the edge, here too.
const PHONE = 'max-md:grid max-md:grid-cols-[minmax(0,1fr)_auto]'
const PHONE_LABEL = 'max-md:col-start-1 max-md:row-start-1 max-md:w-full'
const PHONE_SIGIL = 'max-md:hidden'
const PHONE_VALUE = 'max-md:col-start-1 max-md:row-start-2'
const PHONE_RAIL =
  'max-md:col-start-2 max-md:row-span-2 max-md:row-start-1 max-md:w-auto max-md:gap-0 max-md:self-center max-md:[&_svg]:size-[18px]'
// The strength meter and its stamp share one line under the value.
const PHONE_BELOW =
  'max-md:mt-2 max-md:flex-row max-md:flex-wrap max-md:items-center max-md:justify-between max-md:gap-x-3 max-md:pl-0'

// THE detail-row geometry: a w-32 micro-label column, the value, trailing
// controls, then anything that belongs under the value. Read values and their
// editors both render through it, so switching modes never moves a row.
export default function FieldRow({ label, prefix, actions, below, error, children }: Props) {
  const { t } = useTranslation()
  const { set } = useFields()
  const labelled = label !== undefined
  const id = useId()

  return (
    // Editing, each input draws its own underline, so the hairline between
    // rows would be a second line for the same job; the read view keeps it.
    // `group`: the read row's copy button only shows up on hover (see Field).
    <div className={cx('item group px-3.5 py-3 max-md:px-4', !set && ROW_HAIRLINE)}>
      <div className={`flex items-center gap-3 ${STACK} ${PHONE}`}>
        {labelled && (
          <>
            {/* A label never wraps: the column is sized for the longest of them
                and one that outgrows it gets shortened, here and in the
                catalog, rather than folded onto a second line. */}
            <label
              htmlFor={id}
              className={`w-32 flex-none whitespace-nowrap ${LABEL} ${STACK_LABEL} ${PHONE_LABEL}`}
            >
              {t(label)}
            </label>
            {/* Held open with or without a sigil, so every value starts at one x. */}
            <span
              className={`grid w-4 flex-none place-items-center text-text3 ${STACK_SIGIL} ${PHONE_SIGIL}`}
            >
              {prefix}
            </span>
          </>
        )}
        <div className={`min-w-0 flex-1 ${PHONE_VALUE}`}>{children(id)}</div>
        {labelled ? (
          <div className={`${RAIL} ${STACK_RAIL} ${PHONE_RAIL}`}>{actions}</div>
        ) : (
          actions
        )}
      </div>
      {(below || error) && (
        <div className={cx('mt-1.5 flex flex-col gap-1.5', PHONE_BELOW, labelled && VALUE_START)}>
          {below}
          {error && <span className="text-base text-bad">{error}</span>}
        </div>
      )}
    </div>
  )
}
