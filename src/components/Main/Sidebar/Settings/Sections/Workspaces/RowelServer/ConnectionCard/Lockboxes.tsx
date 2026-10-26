import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { describeError } from '@/api/errors'
import { orgAddLockbox, orgRefreshLockboxKey, type OrgLockbox } from '@/api/org'
import { refreshApp } from '@/store'
import { cx } from '@/utils/cx'
import Button from '@/components/elements/Button'
import Note from '@/components/elements/Note'
import { LABEL, META, TEXT_LINK } from '@/components/elements/tokens'
import type { Listed } from './useDetails'

interface Props {
  url: string
  lockboxes: Listed<OrgLockbox>
  /** The lists are read again once a lockbox was added. */
  onChanged: () => void
}

/**
 * The org lockboxes the member may hold. Adding one makes it a lockbox on this
 * device like any other, and the copy says the one thing that is not like any
 * other: the server holds its key too. The workspace list lives on the launch
 * probe, so an add re-probes before the server's lists are read again.
 */
export default function Lockboxes({ url, lockboxes, onChanged }: Props) {
  const { t } = useTranslation()
  const [busy, setBusy] = useState<string | null>(null)
  const [failure, setFailure] = useState<string | null>(null)

  const run = (id: string, work: Promise<unknown>) => {
    setBusy(id)
    setFailure(null)
    work
      .catch((error: unknown) => setFailure(describeError(error)))
      .finally(() => setBusy(null))
  }
  const add = (id: string) => run(id, orgAddLockbox(url, id).then(refreshApp).then(onChanged))
  const refreshKey = (workspaceId: string) => run(workspaceId, orgRefreshLockboxKey(workspaceId))

  return (
    <div data-testid="org-lockboxes" className="mt-3">
      <div className={cx(LABEL, 'mb-1.5')}>{t('Org lockboxes')}</div>
      <Note className="mb-1.5">
        {t(
          'An org lockbox appears beside your other lockboxes, behind the same master password, and syncs through the server. The server holds its key too, so the server can open it.'
        )}
      </Note>
      {lockboxes.items.length === 0 && !lockboxes.error && (
        <Note testid="org-lockboxes-empty">{t('The server lists no lockboxes for you yet')}</Note>
      )}
      {lockboxes.items.map(({ id, name, workspaceId }) => (
        <div
          key={id}
          data-testid={`org-lockbox-${id}`}
          className="flex flex-wrap items-center gap-x-3 gap-y-1 py-1.5"
        >
          <span className="min-w-0 flex-1 truncate text-base text-text">{name}</span>
          {workspaceId ? (
            <>
              <span className={META}>{t('On this device')}</span>
              <button
                type="button"
                data-testid={`org-refresh-key-${id}`}
                disabled={busy === workspaceId}
                onClick={() => refreshKey(workspaceId)}
                className={cx('text-sm', TEXT_LINK)}
              >
                {t('Refresh key')}
              </button>
            </>
          ) : (
            <Button
              variant="pale"
              size="md"
              loading={busy === id}
              testid={`org-add-${id}`}
              onClick={() => add(id)}
            >
              {t('Add')}
            </Button>
          )}
        </div>
      ))}
      {(lockboxes.error || failure) && (
        <Note tone="bad" testid="org-lockboxes-error" className="mt-1">
          {failure ?? lockboxes.error}
        </Note>
      )}
    </div>
  )
}
