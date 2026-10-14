import { useTranslation } from 'react-i18next'
import { syncDisconnect } from '@/api/sync'
import Button from '@/components/elements/Button'

// The way out, with what it costs said beside it.
export default function Footer() {
  const { t } = useTranslation()
  return (
    <div className="flex items-center gap-3 border-t border-line bg-field px-5 py-2.5">
      <p className="min-w-0 flex-1 text-sm text-text2">
        {t('Disconnecting keeps everything on this device and stops syncing.')}
      </p>
      <Button
        variant="pale-danger"
        size="md"
        className="flex-none"
        onClick={() => syncDisconnect()}
        testid="settings-drive-disconnect"
      >
        {t('Disconnect')}
      </Button>
    </div>
  )
}
