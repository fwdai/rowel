import { useTranslation } from 'react-i18next'
import { useUi } from '@/store'

// The app-level "Copied to Clipboard" pill, raised by `flashCopied` for a copy
// whose control is gone by the time it lands — the generator's "Use & copy"
// closes the dialog on the press. A control still on screen flashes its own
// check instead (`useCopied`). Mounted from `Main`, outside any dialog.
// Mounting is what replays `animate-pop`, so every copy pops even while the
// pill from the previous one is still up. Centering uses auto margins so the
// pop's transform doesn't fight it. On a phone it drops below the safe area, so
// the Dynamic Island or notch doesn't swallow it; elsewhere the inset is 0.
//
// Not the shared `TOAST` token: that is a bordered card in a corner, this is a
// centered pill at the top.
export default function CopyToast() {
  const { t } = useTranslation()
  const copied = useUi(state => state.copied)

  if (!copied) return null

  return (
    <div
      data-testid="copy-toast"
      className="animate-pop fixed inset-x-0 top-[calc(env(safe-area-inset-top)+1rem)] z-50 mx-auto w-max rounded-full bg-text px-5 py-2 text-base text-detail shadow-float"
    >
      {t('Copied to Clipboard')}
    </div>
  )
}
