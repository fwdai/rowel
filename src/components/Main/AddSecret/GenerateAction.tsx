import { useTranslation } from 'react-i18next'
import { closeAddPicker, openGenerator } from '@/store'
import { DicesGlyph } from '../icons'
import ActionRow from './ActionRow'

/**
 * The standalone generator's pointer door: the same open as ⌘G
 * (Main/useShortcuts) with no apply callback, so the dialog just copies.
 *
 * It lives under the Add tiles because that is the moment someone needs a new
 * secret — even one for a form outside the vault. It is not a way in, though,
 * so it comes last, and its caption says nothing it makes is saved unless the
 * user goes on to add it.
 */
export default function GenerateAction() {
  const { t } = useTranslation()

  return (
    <ActionRow
      testid="generator-button"
      glyph={<DicesGlyph size={16} />}
      label={t('Generate a password…')}
      caption={t('For anything outside the vault: copied when you’re done, never saved.')}
      onClick={() => {
        closeAddPicker()
        openGenerator()
      }}
    />
  )
}
