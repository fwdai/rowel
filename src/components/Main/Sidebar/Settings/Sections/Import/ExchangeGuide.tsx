import { useTranslation } from 'react-i18next'
import ActionNote from './ActionNote'

// The Apple Passwords tile on a device where that app hands its data over
// itself: the transfer starts there, not here, so the tile can only say how.
// Its export file is still taken, through the picker the tile opens elsewhere.
export default function ExchangeGuide({ onFile }: { onFile: () => void }) {
  const { t } = useTranslation()

  return (
    <ActionNote
      testid="import-exchange-guide"
      action={t('Import a .csv file instead')}
      actionTestid="import-exchange-file"
      onAction={onFile}
    >
      {t(
        'In the Passwords app, choose Export Data to Another App and pick {{appName}}. {{appName}} opens with a preview of what will be imported, passkeys included.'
      )}
    </ActionNote>
  )
}
