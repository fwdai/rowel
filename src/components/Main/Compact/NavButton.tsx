import type { ReactNode } from 'react'
import { cx } from '@/utils/cx'

interface Props {
  onClick: () => void
  testid: string
  /** The screen's one committing action, set bold as iOS sets "Save". */
  primary?: boolean
  /** A press that will throw something away: the warning ink. */
  danger?: boolean
  disabled?: boolean
  /** Sits on the right edge, so its text ends where the row does. */
  trailing?: boolean
  children: ReactNode
}

// A worded control in a pushed screen's `NavBar` — Cancel, Save, Discard
// changes? — on the 56px row, in the accent, with the 44px target a finger
// needs. One component, so the form's two controls and the held frame's
// Cancel are the same control and cannot drift apart.
export default function NavButton({
  onClick,
  testid,
  primary,
  danger,
  disabled,
  trailing,
  children
}: Props) {
  return (
    <button
      type="button"
      data-testid={testid}
      disabled={disabled}
      onClick={onClick}
      className={cx(
        'flex h-14 min-w-11 cursor-pointer items-center px-2 text-md disabled:cursor-default disabled:opacity-50',
        trailing && 'justify-end',
        primary && 'font-semibold',
        danger ? 'text-bad' : 'text-accent'
      )}
    >
      {children}
    </button>
  )
}
