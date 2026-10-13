import { useId, useState } from 'react'
import { useTranslation } from 'react-i18next'
import type { Workspace } from '@/api/types'
import { deleteWorkspace, selectActiveWorkspace, useApp, useVault } from '@/store'
import { unsealError } from '@/components/Start/shared/errors'
import { workspaceLabel } from '@/lib/workspace'
import { cx } from '@/utils/cx'
import Field from '@/components/elements/Field'
import Note from '@/components/elements/Note'
import Segmented from '@/components/elements/Segmented'
import { inputClass } from '@/components/elements/formStyles'
import { verbatimInput } from '@/components/elements/inputProps'
import { CARD, LABEL } from '@/components/elements/tokens'
import SubpageFrame from '../../../SubpageFrame'
import { useSubpage } from '../../../sectionNav'
import Preview from '../WorkspaceForm/Preview'
import { workspaceAbout, workspaceSyncs } from '../about'

type Scope = 'device' | 'everywhere'

/**
 * Confirming the one destructive thing Settings › Workspaces can do.
 *
 * Two proofs rather than an armed button, because there is no undo and nothing
 * on screen would tell two workspaces apart afterwards: the vault's own master
 * password (which is also what the backend needs — a locked workspace cannot be
 * opened to check it against the session), and the label typed out, which is
 * what makes "this one" deliberate.
 *
 * A workspace that syncs also gets the choice of taking the vault out of the
 * Google account with it (`syncs`); one that does not is only ever a local
 * delete, and is not offered a scope to pick from at all.
 */
export default function Confirm({ workspace }: { workspace: Workspace }) {
  const { t } = useTranslation()
  const { close } = useSubpage()
  const active = useApp(selectActiveWorkspace)
  const sync = useApp(state => state.sync)
  const liveCount = useVault(state => state.items.length)
  // Offer "Delete everywhere" only to a workspace with a vault on Drive.
  const syncs = workspaceSyncs(workspace, { active, sync })
  const passwordId = useId()
  const confirmId = useId()
  const label = workspaceLabel(workspace, t)
  const [password, setPassword] = useState('')
  const [typed, setTyped] = useState('')
  const [scope, setScope] = useState<Scope>('device')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const everywhere = syncs && scope === 'everywhere'
  const ready = password.length > 0 && typed.trim() === label

  const submit = () => {
    if (!ready || busy) return
    setBusy(true)
    setError(null)
    deleteWorkspace(workspace.id, password, everywhere)
      .then(close)
      .catch((err: unknown) => {
        setError(unsealError(t, err, t('That is not this workspace’s master password')))
        setBusy(false)
      })
  }

  const current = workspace.id === active
  const meta = workspaceAbout(workspace, { current, syncs, liveCount }, t)

  return (
    <SubpageFrame
      testid="workspace-delete-page"
      footer={{
        hint: ready ? t('Ready') : t('Password and the name typed back required'),
        cta: t('Delete'),
        danger: true,
        disabled: !ready,
        loading: busy,
        onSubmit: submit,
        testid: 'workspace-delete-submit'
      }}
    >
      <div className="flex flex-col gap-3">
        {/* The workspace itself, as the list draws it: what is about to go. */}
        <Preview
          name={label}
          color={workspace.color}
          seed={workspace.id}
          meta={meta}
          testid="workspace-delete-preview"
        />
        <div className={cx(CARD, 'flex flex-col gap-4 p-5')}>
          {syncs && (
            <Field label={t('What to delete')}>
              <Segmented<Scope>
                options={[
                  { value: 'device', label: t('Remove from this device') },
                  { value: 'everywhere', label: t('Delete everywhere') }
                ]}
                value={scope}
                onChange={setScope}
                name={t('What to delete')}
                testidPrefix="workspace-delete-scope"
              />
            </Field>
          )}
          <Note>
            {everywhere
              ? t(
                  'This removes the workspace from this device and deletes its copy in Google Drive. Your other devices will stop syncing it.'
                )
              : t(
                  'This removes the workspace and everything in it from this device. A copy on Google Drive is left where it is, so the account still holds it.'
                )}
          </Note>
        </div>
      </div>

      <div className={cx(LABEL, 'mt-6 mb-2')}>{t("Confirm it's you")}</div>
      <div className={cx(CARD, 'flex flex-col gap-4 p-5')}>
        <Field id={passwordId} label={t('Master password of this workspace')}>
          <input
            id={passwordId}
            type="password"
            name="workspace_delete_password"
            {...verbatimInput}
            data-testid="workspace-delete-password"
            className={cx(inputClass, error && 'border-bad')}
            aria-invalid={!!error}
            value={password}
            disabled={busy}
            onChange={event => {
              setError(null)
              setPassword(event.target.value)
            }}
          />
          {error && (
            <Note tone="bad" testid="workspace-delete-error">
              {error}
            </Note>
          )}
        </Field>
        <Field id={confirmId} label={t('Type {{name}} to confirm', { name: label })}>
          <input
            id={confirmId}
            type="text"
            {...verbatimInput}
            data-testid="workspace-delete-confirm"
            className={inputClass}
            value={typed}
            disabled={busy}
            onChange={event => setTyped(event.target.value)}
          />
        </Field>
      </div>
    </SubpageFrame>
  )
}
