import { useId, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { exportEntries, type ExportFormat } from '@/api/imports'
import { describeError } from '@/api/errors'
import { useVault } from '@/store'
import { cx } from '@/utils/cx'
import Button from '@/components/elements/Button'
import Field from '@/components/elements/Field'
import Note from '@/components/elements/Note'
import { inputClass } from '@/components/elements/formStyles'
import { verbatimInput } from '@/components/elements/inputProps'
import { LABEL, TEXT_LINK } from '@/components/elements/tokens'
import { useSectionNav } from '../../../sectionNav'
import Formats from './Formats'
import Warning from './Warning'

const fileName = (path: string) => path.replace(/^.*[\\/]/, '')

// A portable, unencrypted dump for another manager. CSV cells are sanitized
// against formula injection in the backend. Moving to another device of our
// own is what the encrypted backup is for, and the footer says so.
export default function ExportPane() {
  const { t } = useTranslation()
  const items = useVault(state => state.items)
  // Whichever shell this is in: the modal's nav, or the phone's pushed pane.
  const go = useSectionNav()
  const [format, setFormat] = useState<ExportFormat>('bitwarden')
  const [acknowledged, setAcknowledged] = useState(false)
  // The file is the whole vault in plaintext, so it takes the master password,
  // like the encrypted backup does (see `exportEntries`).
  const passwordId = useId()
  const [password, setPassword] = useState('')
  const [running, setRunning] = useState(false)
  const [saved, setSaved] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)

  const run = () => {
    if (running || !acknowledged || !password) return
    setRunning(true)
    setSaved(null)
    setError(null)
    exportEntries(format, password)
      .then(path => {
        setPassword('')
        setSaved(path)
      })
      .catch((e: unknown) => setError(describeError(e)))
      .finally(() => setRunning(false))
  }

  return (
    <div className="flex flex-col">
      <div className={cx(LABEL, 'mb-2')}>{t('Format')}</div>
      <Formats value={format} onChange={setFormat} label={t('Format')} />
      <Warning acknowledged={acknowledged} onAcknowledge={setAcknowledged} />

      <div className="mt-4 max-w-sm">
        <Field id={passwordId} label={t('Master password')}>
          <input
            id={passwordId}
            type="password"
            name="export_entries_password"
            data-testid="settings-export-password"
            {...verbatimInput}
            className={inputClass}
            value={password}
            disabled={running}
            onChange={event => {
              setError(null)
              setPassword(event.target.value)
            }}
          />
        </Field>
      </div>

      <div className="mt-4 flex flex-wrap items-center gap-3">
        <p className="min-w-0 flex-1 text-sm text-text2">
          {t('Moving to another device?')}{' '}
          <button
            type="button"
            data-testid="settings-export-backup-link"
            onClick={() => go('sync')}
            className={TEXT_LINK}
          >
            {t('Save an encrypted backup instead')}
          </button>
        </p>
        <Button
          disabled={!acknowledged || !password}
          loading={running}
          onClick={run}
          testid="settings-export-run"
        >
          {t('Export {{count}} items', { count: items.length })}
        </Button>
      </div>

      {error && (
        <Note tone="bad" className="mt-3">
          {error}
        </Note>
      )}
      {saved && (
        <Note tone="good" className="mt-3">
          {t('Saved to')} {fileName(saved)}
        </Note>
      )}
    </div>
  )
}
