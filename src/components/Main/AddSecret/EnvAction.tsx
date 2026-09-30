import { useTranslation } from 'react-i18next'
import { isMobile } from '@/lib/platform'
import { useEnvIngest } from '@/kinds/env/useIngest'
import { openEnvDraft } from '../EnvDrop/open'
import { EnvGlyph } from '../icons'
import ActionRow from './ActionRow'

/**
 * The env file's way in that skips the tile: on the desktop a line of copy,
 * because the whole window is already the target while the picker is up (see
 * `Main/EnvDrop`); on a phone, where nothing is dropped, the file picker.
 */
export default function EnvAction() {
  const { t } = useTranslation()
  // A cancelled dialog leaves the picker as it was; a chosen file hands the
  // window over to the editor, which `openEnvDraft` closes the picker for.
  const { pick, error } = useEnvIngest(openEnvDraft)

  return (
    <ActionRow
      testid="add-env-file"
      glyph={<EnvGlyph size={16} />}
      label={isMobile ? t('Choose a .env file') : t('Drop a .env file')}
      caption={
        isMobile
          ? t('Kept whole, read as a table of variables.')
          : t('Anywhere on this window. Kept whole, read as a table of variables.')
      }
      onClick={isMobile ? () => void pick() : undefined}
    >
      {error && (
        <p data-testid="add-env-error" className="text-sm text-bad">
          {error}
        </p>
      )}
    </ActionRow>
  )
}
