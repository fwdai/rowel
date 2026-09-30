import { useTranslation } from 'react-i18next'
import { closeAddPicker, openReceive } from '@/store'
import { ShareGlyph } from '../icons'
import ActionRow from './ActionRow'

/**
 * The one way in that adds an entry without writing one: a link someone else
 * made. It belongs under the tiles rather than among them — every tile asks
 * *what kind*, and this one already knows, because the link says.
 */
export default function ReceiveAction() {
  const { t } = useTranslation()

  return (
    <ActionRow
      testid="add-receive-share"
      glyph={<ShareGlyph size={16} />}
      label={t('Receive a shared secret…')}
      caption={t('Paste a link someone sent you from {{appName}}.')}
      onClick={() => {
        openReceive()
        closeAddPicker()
      }}
    />
  )
}
