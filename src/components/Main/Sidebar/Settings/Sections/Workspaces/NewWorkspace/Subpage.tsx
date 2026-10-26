import { useId, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { createWorkspace, lockSettings, selectWorkspaces, useApp } from '@/store'
import { describeError } from '@/api/errors'
import { providerName } from '@/api/provider'
import { syncProvider } from '@/api/sync'
import { workspaceLabel } from '@/lib/workspace'
import { firstUnusedColor, type WorkspaceColor } from '@/lib/workspaceColor'
import { cx } from '@/utils/cx'
import Field from '@/components/elements/Field'
import Note from '@/components/elements/Note'
import { inputClass } from '@/components/elements/formStyles'
import { verbatimInput } from '@/components/elements/inputProps'
import { CARD, LABEL } from '@/components/elements/tokens'
import SubpageFrame from '../../../SubpageFrame'
import WorkspaceForm from '../WorkspaceForm'
import { nameTaken } from '../WorkspaceForm/nameTaken'

// Creating a second (or third) encrypted database. There is one master
// password for the device, so the form asks for it rather than for a new one:
// the backend proves it against the primary before creating anything, which
// is what keeps every workspace under the same password — and what lets the
// whole app open with one unlock. A wrong one is the unlock's own error.
//
// Creating opens the new workspace immediately: `workspace_create` hands back
// an unlocked session, so Settings is left behind by the flow change rather
// than closed here.
export default function NewWorkspaceSubpage() {
  const { t } = useTranslation()
  const passwordId = useId()
  const workspaces = useApp(selectWorkspaces)
  // A new workspace joins the open one's account.
  const provider = useApp(state => syncProvider(state.sync))
  const [name, setName] = useState('')
  const [color, setColor] = useState<WorkspaceColor>(() =>
    firstUnusedColor(workspaces.map(workspace => workspace.color))
  )
  const [password, setPassword] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  const label = name.trim()
  const taken = workspaces.map(workspace => workspaceLabel(workspace, t))
  const ready = !!label && !nameTaken(label, taken) && !!password

  // The fields are read once here and held while busy, so what is created is
  // exactly what is on screen.
  const submit = async () => {
    if (!ready || busy) return
    setBusy(true)
    lockSettings(true)
    setError(null)
    try {
      await createWorkspace(label, password, color)
    } catch (err: unknown) {
      setBusy(false)
      setError(describeError(err) || t('Something went wrong'))
    } finally {
      lockSettings(false)
    }
  }

  const meta = provider
    ? `${t('Empty · encrypted')} · ${t('Syncs to {{provider}}', { provider: providerName(provider) })}`
    : t('Empty · encrypted')

  return (
    <SubpageFrame
      footer={{
        hint: ready ? t('Ready') : t('Name and master password required'),
        cta: t('Create lockbox'),
        disabled: !ready,
        loading: busy,
        // The frame's Enter covers the password field too: its own onEnter
        // would submit a second time on the same key.
        onSubmit: () => void submit(),
        testid: 'workspace-create'
      }}
    >
      <WorkspaceForm
        name={name}
        onName={setName}
        color={color}
        onColor={setColor}
        taken={taken}
        preview={{ seed: 'new', meta }}
        disabled={busy}
        testidPrefix="workspace-new"
      />
      <div className={cx(LABEL, 'mt-6 mb-2')}>{t("Confirm it's you")}</div>
      {/* A plain field, not the lock screen's card: here it is one input of a
          form beside the name, and should read as that form's. */}
      <div className={cx(CARD, 'p-5')}>
        <Field id={passwordId} label={t('Master password')}>
          <input
            id={passwordId}
            type="password"
            name="workspace_new_password"
            {...verbatimInput}
            className={cx(inputClass, error && 'border-bad')}
            data-testid="workspace-new-password"
            aria-invalid={!!error}
            value={password}
            disabled={busy}
            onChange={event => {
              setError(null)
              setPassword(event.target.value)
            }}
          />
          {error && (
            <Note tone="bad" testid="workspace-new-error">
              {error}
            </Note>
          )}
          <Note>{t('The new lockbox is sealed with your existing master password.')}</Note>
        </Field>
      </div>
    </SubpageFrame>
  )
}
