import { useTranslation } from 'react-i18next'
import { closePasswordPrompt, selectWorkspaces, useApp } from '@/store'
import { workspaceLabel } from '@/lib/workspace'
import Button from '@/components/elements/Button'
import Note from '@/components/elements/Note'

/**
 * The lockboxes beside this one that kept a password of their own, by name as
 * Settings › Lockboxes lists them. An id the registry no longer holds is shown
 * as is rather than dropped: the user was told something did not move.
 */
export default function Unmoved({ ids }: { ids: string[] }) {
  const { t } = useTranslation()
  const workspaces = useApp(selectWorkspaces)
  const name = (id: string) => {
    const workspace = workspaces.find(w => w.id === id)
    return workspace ? workspaceLabel(workspace, t) : id
  }

  return (
    <div data-testid="password-changed-unmoved" className="mt-3">
      <Note>{t('These lockboxes use a different password and will ask for it when opened:')}</Note>
      <ul className="mt-2 list-disc pl-5 text-base text-text">
        {ids.map(id => (
          <li key={id}>{name(id)}</li>
        ))}
      </ul>
      <div className="mt-5 flex items-center gap-2">
        <Button size="md" onClick={closePasswordPrompt} testid="password-changed-close">
          {t('Close')}
        </Button>
      </div>
    </div>
  )
}
