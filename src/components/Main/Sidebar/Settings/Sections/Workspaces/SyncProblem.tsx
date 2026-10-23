import type { ReactNode } from 'react'
import Note from '@/components/elements/Note'

interface Props {
  testid: string
  /** The failure as the status reported it (`syncErrorText`). */
  text: string | null
  /** The ways out it offers. */
  children: ReactNode
}

/**
 * A run failure that stops the open lockbox syncing until the user does
 * something about it, drawn as a row under the list: what happened, and the
 * buttons that resolve it.
 */
export default function SyncProblem({ testid, text, children }: Props) {
  return (
    <div
      data-testid={testid}
      className="flex flex-wrap items-center justify-between gap-2 px-4 py-3 inset-shadow-hairline"
    >
      <Note tone="bad">{text}</Note>
      <div className="flex flex-wrap gap-2">{children}</div>
    </div>
  )
}
