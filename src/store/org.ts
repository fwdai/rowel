import { create } from 'zustand'
import { describeError } from '@/api/errors'
import {
  orgCancelConnect,
  orgConnect,
  orgConnections,
  type OrgConnectStatus,
  type OrgConnection
} from '@/api/org'

/**
 * The Rowel Servers this install is connected to, and a sign-in in progress.
 * Read on demand — Settings › Lockboxes is the one screen that shows them —
 * and kept across a lock, since a connection is the device's rather than the
 * session's. The devices and lockboxes under each connection are not here:
 * each card reads its own from Rust and re-reads them when `seq` moves.
 */

/**
 * `idle`    — nothing out
 * `pending` — the browser is open on the server's sign-in; `url` is the one
 *             the press named, so the screen can say which while it waits
 * `error`   — the connect or the sign-in failed, with the reason
 */
export interface OrgConnectState {
  status: 'idle' | 'pending' | 'error'
  url: string | null
  error: string | null
}

export interface OrgState {
  /** Null until first read, so a screen can tell "none" from "not asked yet". */
  connections: OrgConnection[] | null
  connect: OrgConnectState
  /**
   * How many times Rust has said something changed behind the screen's back
   * (`org:changed`). Nothing renders the number; a card re-reads its lists
   * when it moves.
   */
  seq: number
}

const CONNECT_IDLE: OrgConnectState = { status: 'idle', url: null, error: null }

export const initialOrg: OrgState = { connections: null, connect: CONNECT_IDLE, seq: 0 }

export const useOrg = create<OrgState>()(() => initialOrg)

/**
 * Re-read the connections. Never rejects: a failed read keeps what was last
 * known on screen, as `refreshApp` does for the probe.
 */
export const loadOrgConnections = (): Promise<void> =>
  orgConnections()
    .then(connections => useOrg.setState({ connections }))
    .catch(() => {})

/**
 * Start a sign-in at `url`. The store moves to `pending` here rather than on
 * the event, so the press changes the screen in the same frame; a rejection
 * is an immediate failure (no browser, a URL Rust refused) and lands where the
 * event's error would.
 */
export const startOrgConnect = (url: string): void => {
  useOrg.setState({ connect: { status: 'pending', url, error: null } })
  orgConnect(url).catch((error: unknown) =>
    useOrg.setState({ connect: { status: 'error', url, error: describeError(error) } })
  )
}

/** Back out of a sign-in still out with the browser. */
export const cancelOrgConnect = (): void => {
  useOrg.setState({ connect: CONNECT_IDLE })
  orgCancelConnect().catch(() => {})
}

/** Clear a failed connect's note, for the next attempt. */
export const resetOrgConnect = (): void => useOrg.setState({ connect: CONNECT_IDLE })

/** What `org:connect` said. A finished sign-in is a new connection, so the list is re-read. */
export const orgConnectReported = ({ url, status, error }: OrgConnectStatus): void => {
  if (status === 'done') {
    useOrg.setState({ connect: CONNECT_IDLE })
    void loadOrgConnections()
    return
  }
  useOrg.setState({
    connect: { status, url, error: status === 'error' ? (error ?? null) : null }
  })
}

/** `org:changed`: the connections are re-read here, and every card re-reads its own lists. */
export const orgChanged = (): void => {
  useOrg.setState(state => ({ seq: state.seq + 1 }))
  void loadOrgConnections()
}
