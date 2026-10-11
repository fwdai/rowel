import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import type { PasswordHistoryItem } from '@/api/types'
import { cx } from '@/utils/cx'
import { ChevronDownGlyph } from '../../../Main/icons'
import Panel from '../../Panel'
import { META } from '../../tokens'
import Clear from './Clear'
import Item from './Item'

interface Props {
  /** The entry the history belongs to. */
  id: string
  /** Previous passwords, newest first. Never empty: no history, no disclosure. */
  history: PasswordHistoryItem[]
  /** The rotation stamp ("Changed 3d ago"), or '' when the entry has none. */
  stamp: string
  /**
   * Given when the toggle is the row's whole value — a cleared password with
   * previous ones — so the row's `<label>` has a control to point at.
   */
  toggleId?: string
}

/**
 * A login's previous passwords, behind the line that already says when the
 * password last changed: that line gains a count and becomes the toggle, and
 * the list opens inline under it. One face for both shells — the rows are the
 * detail row's geometry, so they fold with the container like every other.
 */
export default function PasswordHistory({ id, history, stamp, toggleId }: Props) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  const count = t('{{count}} previous', { count: history.length })

  return (
    <>
      <button
        id={toggleId}
        type="button"
        aria-expanded={open}
        data-testid="password-history-toggle"
        onClick={() => setOpen(!open)}
        // On the phone it keeps to the meter's line as the bare stamp does
        // (see PasswordField): a zero basis so it never breaks the line, the
        // grow to fill it, and the words truncating before the chevron.
        className={cx(
          META,
          'flex cursor-pointer items-center gap-1 text-left transition-colors hover:text-text any-pointer-coarse:min-h-11',
          'max-md:min-w-0 max-md:flex-1 max-md:basis-0 max-md:justify-end'
        )}
      >
        <span className="max-md:truncate">{stamp ? `${stamp} · ${count}` : count}</span>
        <ChevronDownGlyph
          className={cx('flex-none transition-transform', open && 'rotate-180')}
        />
      </button>
      {open && (
        // A line of its own on the phone, where the slot is a wrapping row.
        <div className="w-full" data-testid="password-history">
          <Panel>
            {history.map((item, index) => (
              <Item key={`${item.replacedAt}-${index}`} item={item} index={index} />
            ))}
          </Panel>
          <Clear id={id} />
        </div>
      )}
    </>
  )
}
