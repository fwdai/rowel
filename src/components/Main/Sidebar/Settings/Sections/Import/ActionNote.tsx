import type { ReactNode } from 'react'
import Note from '@/components/elements/Note'
import { TEXT_LINK } from '@/components/elements/tokens'

interface Props {
  testid: string
  action: string
  actionTestid: string
  onAction: () => void
  children: ReactNode
}

// A note in the result card that has nothing to import and says why, closed
// by the one thing there is to do instead, as a quiet link in the sentence.
export default function ActionNote({ testid, action, actionTestid, onAction, children }: Props) {
  return (
    <Note testid={testid}>
      {children}{' '}
      <button type="button" data-testid={actionTestid} onClick={onAction} className={TEXT_LINK}>
        {action}
      </button>
    </Note>
  )
}
