import type { KeyboardEvent, ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import { cx } from '@/utils/cx'
import { useLayout } from '@/hooks/useLayout'
import Button from '@/components/elements/Button'
import { META } from '@/components/elements/tokens'
import {
  ACTION_BUTTON,
  ACTION_FILL,
  ACTION_FILL_DANGER,
  ROOT_ACTION,
  ROOT_CLEARANCE,
  SCREEN_SCROLLER
} from '../../Compact/chrome'
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
 * A sub-page's body and its action, in both shells.
 *
 * On the wide shell the action bar is pinned under a scrolling body, so the
 * CTA stays in reach however long the form runs. On the phone the action is
 * the slab every pushed screen puts where a thumb is — the detail screen's
 * Copy, the generator's Use — lifted clear of the tab bar as the generator
 * root's is; the nav row's Back is the Cancel, and the hint has nothing to
 * add that a dimmed slab does not say.
 *
 * Enter submits from anywhere inside, except where Enter means something else:
 * a textarea's new line, or a focused button pressing itself.
 */
export default function SubpageFrame({ children, footer, testid }: Props) {
  const { t } = useTranslation()
  const { close } = useSubpage()
  const compact = useLayout() === 'compact'
  const inert = footer.disabled || footer.loading
  const submitTestid = footer.testid ?? 'settings-subpage-submit'

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key !== 'Enter' || e.defaultPrevented || e.nativeEvent.isComposing) return
    if ((e.target as HTMLElement).closest('textarea, button')) return
    e.preventDefault()
    if (!inert) footer.onSubmit()
  }

  if (compact)
    return (
      <div data-testid={testid} className="flex min-h-0 flex-1 flex-col" onKeyDown={onKeyDown}>
        <div className={cx(SCREEN_SCROLLER, 'px-4 pt-1', ROOT_CLEARANCE)}>{children}</div>
        <button
          type="button"
          data-testid={submitTestid}
          disabled={inert}
          onClick={footer.onSubmit}
          className={cx(
            ACTION_BUTTON,
            footer.danger ? ACTION_FILL_DANGER : ACTION_FILL,
            ROOT_ACTION,
            inert && 'opacity-50'
          )}
        >
          {footer.cta}
        </button>
      </div>
    )

  return (
    <div data-testid={testid} className="flex min-h-0 flex-1 flex-col" onKeyDown={onKeyDown}>
      <div className={cx(SCREEN_SCROLLER, 'px-7 pt-3 pb-7')}>{children}</div>
      <div className="flex flex-none items-center gap-2 border-t border-line bg-pane px-7 py-3.5">
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
          testid={submitTestid}
          disabled={footer.disabled}
          loading={footer.loading}
          onClick={footer.onSubmit}
        >
          {footer.cta}
        </Button>
      </div>
    </div>
  )
}
