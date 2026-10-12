import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { exportVault } from '@/api/vault'
import { BACKUP_EXTENSION } from '@/lib/backup'
import Button from '@/components/elements/Button'
import Chip from '@/components/elements/Chip'
import Note from '@/components/elements/Note'
import { inputClass } from '@/components/elements/formStyles'
import { verbatimInput } from '@/components/elements/inputProps'
import { DiskGlyph } from '../../../../icons'
import ExpandableRow from '../ExpandableRow'

// The whole vault as one backup file, resealed under the master password the
// user re-types here (the open session's key is never handed to the exporter).
export default function BackupRow() {
  const { t } = useTranslation()
  const [password, setPassword] = useState('')
  const [exporting, setExporting] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [saved, setSaved] = useState(false)

  const onExport = () => {
    if (!password || exporting) return
    setExporting(true)
    setError(null)
    setSaved(false)
    exportVault(password)
      .then(() => {
        setPassword('')
        setSaved(true)
      })
      .catch(() => setError(t('Invalid master password')))
      .finally(() => setExporting(false))
  }

  return (
    <ExpandableRow
      label={
        <span className="flex items-center gap-2">
          {t('Encrypted backup')}
          <Chip>{BACKUP_EXTENSION}</Chip>
        </span>
      }
      description={t('Your whole vault, sealed with your master password')}
      icon={<DiskGlyph size={16} />}
      action={t('Save…')}
      testid="settings-backup-row"
    >
      <div className="flex flex-wrap items-center gap-3">
        <input
          type="password"
          name="export_password"
          {...verbatimInput}
          placeholder={t('Master password')}
          className={`${inputClass} max-w-xs`}
          value={password}
          disabled={exporting}
          onChange={event => {
            setError(null)
            setPassword(event.target.value)
          }}
        />
        <Button
          size="md"
          disabled={!password}
          loading={exporting}
          onClick={onExport}
          testid="settings-backup-save"
        >
          {t('Save')}
        </Button>
        {error && <Note tone="bad">{error}</Note>}
        {saved && <Note tone="good">{t('Saved')}</Note>}
      </div>
    </ExpandableRow>
  )
}
