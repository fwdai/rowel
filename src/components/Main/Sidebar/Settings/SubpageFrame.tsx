import type { KeyboardEvent, ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import { cx } from '@/utils/cx'
import { useLayout } from '@/hooks/useLayout'
import Button from '@/components/elements/Button'
import { META } from '@/components/elements/tokens'
import { SCREEN_SCROLLER, TAB_BAR_CLEARANCE } from '../../Compact/chrome'
import { useSubpage } from './sectionNav'

export interface SubpageFooter {
  /** A line of context beside the actions (what the CTA will do, a count). */
  hint?: ReactNode
  cta: string
  /** The CTA destroys something: drawn in the warning fill, not the accent. */
  danger?: boolean
  disabled?: boolean
  loading?: boolean
  onSubmit: () => void
  testid?: string
}

interface Props {
  children: ReactNode
  footer: SubpageFooter
  testid?: string
}

/**
 * A sub-page's body and its action bar, in both shells.
 *
 * On the wide shell the bar is pinned under a scrolling body, so the CTA stays
 * in reach however long the form runs. On the phone the tab bar floats over the
 * bottom of the screen and would cover a pinned bar, so the bar rides at the end
 * of the scroller instead, above the tab bar's clearance.
 *
 * Enter submits from anywhere inside, except where Enter means something else:
 * a textarea's new line, or a focused button pressing itself.
 */
export default function SubpageFrame({ children, footer, testid }: Props) {
  const { t } = useTranslation()
  const { close } = useSubpage()
  const compact = useLayout() === 'compact'
  const inert = footer.disabled || footer.loading

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key !== 'Enter' || e.defaultPrevented || e.nativeEvent.isComposing) return
    if ((e.target as HTMLElement).closest('textarea, button')) return
    e.preventDefault()
    if (!inert) footer.onSubmit()
  }

  const bar = (
    <div
      className={cx(
        'flex flex-none items-center gap-2 border-t border-line bg-pane py-3.5',
        compact ? 'mt-6 px-4' : 'px-7'
      )}
    >
      <div data-testid="settings-subpage-hint" className={cx(META, 'min-w-0 flex-1')}>
        {footer.hint}
      </div>
      <Button
        variant="pale"
        size="md"
        testid="settings-subpage-cancel"
        disabled={footer.loading}
        onClick={close}
      >
        {t('Cancel')}
      </Button>
      {/* The button's own borderless chip, not a bordered Kbd: on the accent
          fill a dimmed Kbd is the one thing that disappears. */}
      <Button
        variant={footer.danger ? 'danger' : 'primary'}
        size="md"
        kbd="⏎"
        testid={footer.testid ?? 'settings-subpage-submit'}
        disabled={footer.disabled}
        loading={footer.loading}
        onClick={footer.onSubmit}
      >
        {footer.cta}
      </Button>
    </div>
  )

  return (
    <div data-testid={testid} className="flex min-h-0 flex-1 flex-col" onKeyDown={onKeyDown}>
      <div
        className={cx(
          SCREEN_SCROLLER,
          compact ? cx('pt-1', TAB_BAR_CLEARANCE) : 'px-7 pt-3 pb-7'
        )}
      >
        <div className={compact ? 'px-4' : undefined}>{children}</div>
        {compact && bar}
      </div>
      {!compact && bar}
    </div>
  )
}
