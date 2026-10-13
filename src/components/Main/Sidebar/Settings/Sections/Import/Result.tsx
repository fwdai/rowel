import { useId, useState } from 'react'
import { useTranslation } from 'react-i18next'
import Button from '@/components/elements/Button'
import Field from '@/components/elements/Field'
import Note from '@/components/elements/Note'
import { inputClass } from '@/components/elements/formStyles'
import { verbatimInput } from '@/components/elements/inputProps'
import { CARD, META } from '@/components/elements/tokens'
import Progress from './Progress'
import RowErrors from './RowErrors'
import type { useImport } from './useImport'

const fileName = (path: string) => path.replace(/^.*[\\/]/, '')

// The one card under the grid: what was picked, what it holds, and the button
// that commits it.
export default function Result({ flow }: { flow: ReturnType<typeof useImport> }) {
  const { t } = useTranslation()
  const { picked, preview, result, count, running, error, progress } = flow
  const passwordId = useId()
  const [password, setPassword] = useState('')

  if (!picked) return null

  return (
    <div className={`${CARD} mt-3 flex flex-col gap-3 p-4`} data-testid="import-result">
      <div className={META}>{fileName(picked.path)}</div>

      {picked.kind === 'rowel' ? (
        // The OS opened one of our own backups with the app, but a vault is
        // already open: it is a whole sealed database, not rows to merge.
        <Note testid="import-rowel-notice">
          {t('Backups restore a fresh install and are not imported into an existing vault.')}
        </Note>
      ) : picked.kind === 'swftx' ? (
        <div className="flex max-w-sm flex-col gap-3">
          <Field id={passwordId} label={t('Vault file password')}>
            <input
              id={passwordId}
              type="password"
              name="import_password"
              {...verbatimInput}
              className={inputClass}
              value={password}
              disabled={running}
              onChange={event => setPassword(event.target.value)}
            />
          </Field>
          <Button
            className="self-start"
            loading={running}
            onClick={() => flow.runBackup(password)}
            testid="import-run-backup"
          >
            {t('Run import')}
          </Button>
        </div>
      ) : (
        preview && (
          <div className="flex flex-wrap items-center gap-3">
            <span className="text-sm text-text2">
              {t('Ready to import')}:{' '}
              <strong className="text-text">{preview.total}</strong>
              {preview.skipped > 0 && (
                <span className="text-bad">
                  {' '}
                  · {t('{{count}} rows skipped', { count: preview.skipped })}
                </span>
              )}
            </span>
            <Button loading={running} onClick={flow.commit} testid="import-commit">
              {t('Import')} {preview.total}
            </Button>
          </div>
        )
      )}

      {running && <Progress done={progress.done} total={progress.total} />}

      {error && (
        <Note tone="bad" testid="import-error">
          {error}
        </Note>
      )}
      {result && (
        <Note tone="good">
          {t('Imported')} {result.imported}
          {result.skipped > 0 && ` · ${t('{{count}} skipped', { count: result.skipped })}`}
        </Note>
      )}
      {count !== null && (
        <Note tone="good">
          {t('Imported')} {count}
        </Note>
      )}

      {preview && <RowErrors kind="error" rows={preview.errors} />}
      {preview && <RowErrors kind="warning" rows={preview.warnings} />}
      {result && <RowErrors kind="error" rows={result.errors} />}
      {result && <RowErrors kind="warning" rows={result.warnings} />}
    </div>
  )
}
