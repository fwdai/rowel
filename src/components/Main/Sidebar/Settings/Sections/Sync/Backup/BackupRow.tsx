import { useId, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { describeError } from '@/api/errors'
import { selectActiveWorkspace, selectWorkspaces, useApp } from '@/store'
import { workspaceLabel } from '@/lib/workspace'
import { exportVault } from '@/api/vault'
import { BACKUP_EXTENSION } from '@/lib/backup'
import Button from '@/components/elements/Button'
import Chip from '@/components/elements/Chip'
import Field from '@/components/elements/Field'
import Note from '@/components/elements/Note'
import { inputClass } from '@/components/elements/formStyles'
import { verbatimInput } from '@/components/elements/inputProps'
import { DiskGlyph } from '@/components/Main/icons'
import ExpandableRow from '../../ExpandableRow'

// The open lockbox as one backup file, resealed under the master password the
// user re-types here (the open session's key is never handed to the exporter).
// The row names the lockbox: the file holds that one, not every lockbox here.
export default function BackupRow() {
  const { t } = useTranslation()
  const id = useId()
  const active = useApp(selectActiveWorkspace)
  const workspace = useApp(selectWorkspaces).find(workspace => workspace.id === active)
  const lockbox = workspace ? workspaceLabel(workspace, t) : t('Personal')
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
      .catch((e: unknown) => setError(describeError(e)))
      .finally(() => setExporting(false))
  }

  return (
    <ExpandableRow
      label={
        <span className="flex items-center gap-2">
          {t('Save a backup')}
          <Chip>{BACKUP_EXTENSION}</Chip>
        </span>
      }
      description={t('Everything in {{lockbox}}, sealed with your master password', { lockbox })}
      icon={<DiskGlyph size={16} />}
      action={t('Save…')}
      testid="settings-backup-row"
    >
      <div className="flex max-w-sm flex-col gap-3">
        <Field id={id} label={t('Master password')}>
          <input
            id={id}
            type="password"
            name="export_password"
            {...verbatimInput}
            className={inputClass}
            value={password}
            disabled={exporting}
            onChange={event => {
              setError(null)
              setPassword(event.target.value)
            }}
          />
        </Field>
        <div className="flex flex-wrap items-center gap-3">
          <Button
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
      </div>
    </ExpandableRow>
  )
}
