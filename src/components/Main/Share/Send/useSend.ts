import { useCallback, useEffect, useState } from 'react'
import { shareCreate, shareRevoke, type ShareCreated } from '@/api/share'
import { describeError } from '@/api/errors'
import { providerShares } from '@/api/provider'
import { syncProvider } from '@/api/sync'
import { useApp, closeSend, queueOrphan } from '@/store'
import { revokeOrphans } from '@/services/shares'
import { useCopied } from '@/hooks/useCopied'
import { useLatestRequest } from '@/hooks/useLatestRequest'

/**
 * Which operation failed, not just what it said. "Try again" has to retry the
 * thing that went wrong: retrying a failed revoke with a create would publish a
 * second link while the first one is still live.
 */
export interface Failure {
  op: 'create' | 'revoke'
  message: string
}

export interface Send {
  /**
   * Without Drive there is nowhere to put the file, so the dialog explains —
   * a vault on another provider included, until it can hold shares too.
   */
  connected: boolean
  share: ShareCreated | null
  failed: Failure | null
  busy: boolean
  copied: boolean
  copy: () => void
  /** Retries a failed create; the panel only offers it in that state. */
  create: () => void
  /** Revokes, and retries itself after a failed revoke. */
  revoke: () => void
}

/**
 * The whole of sending: seal the entry, hold the link it comes back as, hand it
 * to the clipboard, or take it back. The hook lives as long as the dialog does
 * — closing it unmounts the hook, which is what clears the link.
 *
 * The seal fires on mount rather than behind a button — there is nothing to
 * choose in v1, so a confirm step would only be a press between the user and
 * the one thing this dialog does.
 */
export function useSend(entryId: string): Send {
  const connected = useApp(state => providerShares(syncProvider(state.sync)))
  const [share, setShare] = useState<ShareCreated | null>(null)
  const [failed, setFailed] = useState<Failure | null>(null)
  const [busy, setBusy] = useState(false)
  const { copied, copy: copyValue } = useCopied()

  // Only the newest request may reach the screen: a seal is a network round
  // trip, so a slow one can land after the dialog has moved on and put someone
  // else's link under this title.
  const begin = useLatestRequest()

  // A share an earlier dialog could not take back is tried again here, on the
  // way into the next one.
  useEffect(() => {
    void revokeOrphans()
  }, [])

  const create = useCallback(() => {
    const current = begin()
    setBusy(true)
    setFailed(null)
    shareCreate(entryId)
      .then(created => {
        // Nobody will ever see this link, and the sealed file is already sitting
        // in the sender's Drive with 24 hours to live. Take it back — and if
        // that fails too, remember it, so the next share surface tries again
        // rather than the share quietly outliving everyone who knew about it.
        if (!current()) {
          queueOrphan(created.fileId)
          void revokeOrphans()
          return
        }
        setShare(created)
        setBusy(false)
      })
      // Said in the user's own language where the kind has a settled meaning
      // ("sync is not configured"); the ones Rust writes per call site ("a
      // passkey cannot be shared") still come through as they arrive.
      .catch(reason => {
        if (!current()) return
        setFailed({ op: 'create', message: describeError(reason) })
        setBusy(false)
      })
  }, [entryId, begin])

  useEffect(() => {
    if (connected) create()
  }, [connected, create])

  // Through the clipboard service like every other secret: the link is a bearer
  // token for one entry, so it obeys the auto-clear the user configured.
  const copy = () => share && copyValue(share.link)

  const revoke = useCallback(() => {
    if (!share || busy) return
    const current = begin()
    setBusy(true)
    setFailed(null)
    shareRevoke(share.fileId)
      .then(() => current() && closeSend())
      .catch(reason => {
        if (!current()) return
        // The link is still live, so it stays on screen with the complaint
        // under it — the only honest thing to show after a failed revoke.
        setFailed({ op: 'revoke', message: describeError(reason) })
        setBusy(false)
      })
  }, [share, busy, begin])

  return { connected, share, failed, busy, copied, copy, create, revoke }
}
