import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import type { Workspace } from '@/api/types'
import {
  useApp,
  useVault,
  refreshApp,
  lockSettings,
  selectActiveWorkspace,
  selectWorkspaces
} from '@/store'
import { workspaceRename, workspaceSetColor } from '@/api/workspace'
import { describeError } from '@/api/errors'
import { workspaceLabel } from '@/lib/workspace'
import { isWorkspaceColor, type WorkspaceColor } from '@/lib/workspaceColor'
import SubpageFrame from '../../../SubpageFrame'
import { useSubpage } from '../../../sectionNav'
import WorkspaceForm from '../WorkspaceForm'
import { nameTaken } from '../WorkspaceForm/nameTaken'
import { workspaceAbout, workspaceProvider } from '../about'

// Any workspace can be edited, the locked ones included — the backend writes a
// locked vault's name into the registry, and the open one's into the vault as
// well so it travels. A colour is registry only, so it stays on this device.
export default function Editor({ workspace }: { workspace: Workspace }) {
  const { t } = useTranslation()
  const { close } = useSubpage()
  const list = useApp(selectWorkspaces)
  const active = useApp(selectActiveWorkspace)
  const sync = useApp(state => state.sync)
  const liveCount = useVault(state => state.items.length)

  const label = workspaceLabel(workspace, t)
  const saved = isWorkspaceColor(workspace.color) ? workspace.color : null
  const [name, setName] = useState(label)
  const [color, setColor] = useState<WorkspaceColor | null>(saved)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  const next = name.trim()
  const taken = list.filter(({ id }) => id !== workspace.id).map(other => workspaceLabel(other, t))
  const renamed = next !== label
  const recoloured = color !== saved
  const changed = renamed || recoloured
  const valid = !!next && !nameTaken(next, taken)
  const ready = changed && valid

  // The probe is what carries names and colours, so an edit is complete once
  // it has been re-read — the list and the header both redraw from it.
  const submit = async () => {
    if (!ready || busy) return
    setBusy(true)
    lockSettings(true)
    setError(null)
    let failure: unknown = null
    try {
      try {
        if (renamed) await workspaceRename(workspace.id, next)
        if (recoloured) await workspaceSetColor(workspace.id, color)
      } catch (err: unknown) {
        failure = err
      }
      // Re-read either way: a rename that landed before the colour failed is
      // in the list, and — since the fields are compared against the workspace
      // as it now is — a retry sends only what is still unsaved.
      await refreshApp()
    } finally {
      lockSettings(false)
    }
    if (failure === null) return close()
    setBusy(false)
    setError(describeError(failure) || t('Something went wrong'))
  }

  const current = workspace.id === active
  const meta = workspaceAbout(
    workspace,
    { current, provider: workspaceProvider(workspace, { active, sync }), liveCount },
    t
  )

  return (
    <SubpageFrame
      footer={{
        // A change that cannot be saved (a name taken, or none) says why in
        // the form, so the hint has nothing to add.
        hint: !changed ? t('No changes yet') : valid ? t('Ready') : undefined,
        cta: t('Save changes'),
        disabled: !ready,
        loading: busy,
        onSubmit: () => void submit(),
        testid: 'workspace-edit-save'
      }}
    >
      <WorkspaceForm
        name={name}
        onName={setName}
        color={color}
        onColor={setColor}
        taken={taken}
        preview={{ seed: workspace.id, meta }}
        disabled={busy}
        testidPrefix="workspace-edit"
      />
      {error && (
        <p data-testid="workspace-edit-error" className="mt-3 text-sm text-bad">
          {error}
        </p>
      )}
    </SubpageFrame>
  )
}
