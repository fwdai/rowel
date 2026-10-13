import type { ReactNode } from 'react'
import { cx } from '@/utils/cx'
import { META_TYPE } from './tokens'

export type ChipTone = 'neutral' | 'accent' | 'good' | 'bad'

const TONES: Record<ChipTone, string> = {
  neutral: 'bg-tile text-text2',
  accent: 'bg-accent-soft text-accent',
  good: 'bg-good/10 text-good',
  bad: 'bg-bad/10 text-bad'
}

interface Props {
  tone?: ChipTone
  /** A status dot before the text: the chip says where something stands. */
  dot?: boolean
  testid?: string
  children: ReactNode
}

// THE status chip: a 20px pill in the meta face, beside a title or in a row's
// control slot — a sync state, a count, "Open now", "Always on". One shape for
// all of them, so a chip means the same thing wherever it sits.
export default function Chip({ tone = 'neutral', dot, testid, children }: Props) {
  return (
    <span
      data-testid={testid}
      data-tone={tone}
      className={cx(
        'inline-flex h-5 flex-none items-center gap-1.5 rounded-full px-2 font-medium',
        META_TYPE,
        TONES[tone]
      )}
    >
      {dot && <span aria-hidden className="h-[5px] w-[5px] rounded-full bg-current" />}
      {children}
    </span>
  )
}
