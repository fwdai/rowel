import type { ReactNode } from 'react'
import { cx } from '@/utils/cx'

type Tone = 'muted' | 'good' | 'bad'

const INK: Record<Tone, string> = {
  muted: 'text-text2',
  good: 'text-good',
  bad: 'text-bad'
}

interface Props {
  tone?: Tone
  testid?: string
  className?: string
  children: ReactNode
}

// THE line of feedback under a control or a row: an error, a confirmation, an
// empty state, a hint. The note tier (12px) in the ink its tone asks for, and
// nothing else — placement is the caller's.
export default function Note({ tone = 'muted', testid, className, children }: Props) {
  return (
    <p data-testid={testid} className={cx('text-sm', INK[tone], className)}>
      {children}
    </p>
  )
}
