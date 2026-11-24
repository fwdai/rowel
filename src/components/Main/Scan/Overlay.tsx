import { useTranslation } from 'react-i18next'
import { ScanGlyph } from '../icons'

// Shown while an image is dragged over the window: the whole surface becomes
// the target, so the answer to "where do I drop this?" is "anywhere". Above the
// toasts and inert — the drag is the OS's, not the DOM's, so nothing here has
// to be hit-testable.
export default function Overlay() {
  const { t } = useTranslation()

  return (
    <div
      data-testid="scan-overlay"
      aria-hidden
      className="animate-fade pointer-events-none fixed inset-0 z-[1100] grid place-items-center bg-scrim px-safe backdrop-blur-sm"
    >
      <div className="m-6 flex flex-col items-center gap-3 rounded-xl border border-dashed border-accent-line px-10 py-9 text-center">
        <span className="grid h-12 w-12 place-items-center rounded-lg bg-accent-soft text-accent">
          <ScanGlyph size={22} />
        </span>
        <strong className="text-lg font-semibold tracking-display">
          {t('Drop to scan the card or document')}
        </strong>
        <span className="max-w-xs text-base text-text2">
          {t('Read on this device. The image is never copied or uploaded.')}
        </span>
      </div>
    </div>
  )
}
