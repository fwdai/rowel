import { useId, useState } from 'react'
import { useTranslation } from 'react-i18next'
import type { Workspace } from '@/api/types'
import { describeError } from '@/api/errors'
import { exportVault } from '@/api/vault'
import { refreshApp } from '@/store'
import { useDates } from '@/hooks/useDates'
import { workspaceLabel } from '@/lib/workspace'
import Button from '@/components/elements/Button'
import Field from '@/components/elements/Field'
import Monogram from '@/components/elements/Monogram'
import Note from '@/components/elements/Note'
import { inputClass } from '@/components/elements/formStyles'
import { verbatimInput } from '@/components/elements/inputProps'
import ExpandableRow from '../../ExpandableRow'

// One lockbox as a backup file of its own: its tile and name over when it was
// last saved here, and Save… unfolding the master password that seals it. The
// password proves whoever is typing may carry the lockbox away, whichever one
// it is; the file holds this lockbox and nothing else.
export default function BackupRow({ workspace }: { workspace: Workspace }) {
  const { t } = useTranslation()
  const { relativeLong } = useDates()
  const id = useId()
  const label = workspaceLabel(workspace, t)
  const [password, setPassword] = useState('')
  const [exporting, setExporting] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [saved, setSaved] = useState(false)

  const onExport = () => {
    if (!password || exporting) return
    setExporting(true)
    setError(null)
    setSaved(false)
    exportVault(workspace.id, password)
      .then(path => {
        // A dismissed dialog saved nothing, and keeps the password for another go.
        if (path === null) return
        setPassword('')
        setSaved(true)
        // The probe is what carries when each lockbox was last saved, so
        // re-reading it is what moves the line under this row's name.
        return refreshApp()
      })
      .catch((e: unknown) => setError(describeError(e)))
      .finally(() => setExporting(false))
  }

  return (
    <ExpandableRow
      tile={<Monogram name={label} seed={workspace.id} color={workspace.color} size={32} />}
      label={label}
      description={
        workspace.backedUpAt
          ? t('Last saved {{when}}', { when: relativeLong(workspace.backedUpAt) })
          : t('Never saved')
      }
      action={t('Save…')}
      testid={`settings-backup-row-${workspace.id}`}
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
            testid={`settings-backup-save-${workspace.id}`}
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
