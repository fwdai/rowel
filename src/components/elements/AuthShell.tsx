import type { CSSProperties, ReactNode, Ref } from 'react'
import { useTranslation } from 'react-i18next'
import { cx } from '@/utils/cx'
import Back from '@/assets/images/back.svg?react'
import VaultFooter from './VaultFooter'
import { LABEL } from './tokens'

interface Props {
  children: ReactNode
  onBack?: () => void
  /**
   * Draw the footer strip (where the vault lives + the version; see
   * VaultFooter). The lock screen's: it says which vault is about to open. The
   * first run has no vault yet, so it draws none — and nothing else sits under
   * its content.
   */
  footer?: boolean
  /**
   * The content lays itself out over the shell's whole height — the phone
   * lock screen's three bands — rather than centring as one block, which is
   * what a form or a card wants.
   */
  fill?: boolean
  /**
   * The ground's own inline style: a caller sized to the visual viewport
   * (`viewportStyle`) passes its height here, so the keyboard shortens the
   * ground rather than covering what is centred on it.
   */
  style?: CSSProperties
  /**
   * The scrolling ground itself, for a caller that keeps the shell mounted
   * across screens and has to put the scroll back at the top when the screen
   * under it changes (see Start).
   */
  ref?: Ref<HTMLDivElement>
}

// The shared full-height, centered auth ground: a neutral background (via the
// `bg-app` token, so it reads in both light and dark) with a soft radial glow
// behind a max-560px centered column. "Back" sits top-left, where every
// stepped flow puts it, clear of the content it steps away from; the footer
// strip is opt-in. Reused by the lock, setup and restore screens.
export default function AuthShell({ children, onBack, footer, fill, style, ref }: Props) {
  const { t } = useTranslation()
  return (
    // Compact narrows the gutters, pads past the notch by a nav bar's height
    // (so the centered column never lays over "Back") and past the home
    // indicator by the footer's height when there is one. The column then
    // centers with `my-auto` rather than `justify-center`, which keeps it
    // scrollable instead of clipped when it still does not fit. At md: — the
    // same 768px the shell switches on — every one of those is off again and
    // the wide layout is what it was.
    <div
      ref={ref}
      style={style}
      className={cx(
        'relative flex h-full flex-col items-center overflow-x-hidden overflow-y-auto overscroll-contain bg-app px-5 pt-[calc(env(safe-area-inset-top)+2.75rem)] text-text select-none md:px-10 md:pt-0 md:pb-0',
        footer ? 'pb-[calc(env(safe-area-inset-bottom)+3.25rem)]' : 'pb-[env(safe-area-inset-bottom)]'
      )}
    >
      <div
        aria-hidden
        className="pointer-events-none absolute left-1/2 top-[44%] h-[700px] w-[1200px] max-w-none -translate-x-1/2 -translate-y-1/2"
        style={{
          background:
            'radial-gradient(closest-side, var(--topglow), transparent 70%)'
        }}
      />

      {/*
        Lock/setup/restore render without the Header, so without this strip the
        window has no drag region at all on these screens. It matches the
        Header's 38px height and sits under the traffic lights. It is painted
        before the panel below, so any panel content that reaches into the top
        strip keeps its own clicks -- only bare background drags the window.
      */}
      <div
        data-tauri-drag-region
        className="absolute inset-x-0 top-0 h-[38px]"
      />

      {/* Under the drag strip and the traffic lights on desktop; a nav bar's
          height into the safe area on a phone. */}
      {onBack && (
        <button
          type="button"
          data-testid="go-back-button"
          onClick={onBack}
          className={`absolute left-3 top-[calc(env(safe-area-inset-top)+0.5rem)] flex h-7 cursor-pointer items-center gap-1 rounded-sm border-0 bg-transparent pl-1.5 pr-2.5 ${LABEL} transition-colors hover:bg-hover hover:text-text md:left-4 md:top-11`}
        >
          <Back width="11" className="[&_path]:fill-current" />
          {t('Back')}
        </button>
      )}

      <div
        className={cx(
          'relative w-[560px] max-w-full',
          fill ? 'flex flex-1 flex-col' : 'my-auto'
        )}
      >
        {children}
      </div>

      {footer && (
        <div className="absolute inset-x-0 bottom-0 flex h-13 items-center justify-center">
          <VaultFooter />
        </div>
      )}
    </div>
  )
}
