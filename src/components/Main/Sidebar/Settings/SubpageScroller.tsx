import type { ReactNode } from 'react'
import { cx } from '@/utils/cx'
import { useLayout } from '@/hooks/useLayout'
import { SCREEN_SCROLLER, TAB_BAR_CLEARANCE } from '../../Compact/chrome'

/**
 * A sub-page that is a page of settings rather than a form: the section's own
 * scroller and insets, in both shells, with nothing pinned under it. A form
 * with an action to submit takes `SubpageFrame` instead.
 */
export default function SubpageScroller({
  children,
  testid
}: {
  children: ReactNode
  testid?: string
}) {
  const compact = useLayout() === 'compact'
  return (
    <div
      data-testid={testid}
      className={cx(SCREEN_SCROLLER, compact ? cx('px-4', TAB_BAR_CLEARANCE) : 'px-7 pt-3 pb-7')}
    >
      {children}
    </div>
  )
}
