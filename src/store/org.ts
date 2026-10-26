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
 *
 * `attempt` is the id Rust gave the live press's sign-in once its command
 * answered — null before that, and whenever nothing is out. An `org:connect`
 * event counts only when it carries this id: a cancelled or replaced attempt's
 * last word, already on its way when it was given up on, lands nowhere.
 */
export interface OrgConnectState {
  status: 'idle' | 'pending' | 'error'
  url: string | null
  attempt: number | null
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

const CONNECT_IDLE: OrgConnectState = { status: 'idle', url: null, attempt: null, error: null }

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

// Which press is live. A cancel, a reset or a newer press moves it, so an
// older press's command answer — the attempt id it began, or its rejection —
// finds nothing to land on. Module-level: nothing renders it.
let press = 0

/**
 * Start a sign-in at `url`. The store moves to `pending` here rather than on
 * the event, so the press changes the screen in the same frame; a rejection
 * is an immediate failure (no browser, a URL Rust refused) and lands where the
 * event's error would.
 */
export const startOrgConnect = (url: string): void => {
  const mine = ++press
  useOrg.setState({ connect: { status: 'pending', url, attempt: null, error: null } })
  orgConnect(url).then(
    attempt => {
      if (mine === press) useOrg.setState({ connect: { status: 'pending', url, attempt, error: null } })
    },
    (error: unknown) => {
      if (mine === press)
        useOrg.setState({
          connect: { status: 'error', url, attempt: null, error: describeError(error) }
        })
    }
  )
}

const settle = () => {
  press++
  useOrg.setState({ connect: CONNECT_IDLE })
}

/** Back out of a sign-in still out with the browser. */
export const cancelOrgConnect = (): void => {
  settle()
  orgCancelConnect().catch(() => {})
}

/** Clear a failed connect's note, for the next attempt. */
export const resetOrgConnect = (): void => settle()

/**
 * What `org:connect` said, if it is about the live attempt; anything else is
 * a word from one already given up on. A finished sign-in is a new
 * connection, so the list is re-read.
 */
export const orgConnectReported = ({ attempt, status, error }: OrgConnectStatus): void => {
  const live = useOrg.getState().connect
  // The attempt id alone names the sign-in: Rust spells the URL its own way
  // (no trailing slash, lowercase host), so the one the press typed would
  // miss its own events.
  if (live.attempt === null || live.attempt !== attempt) return
  if (status === 'done') {
    settle()
    void loadOrgConnections()
    return
  }
  useOrg.setState({
    connect: { status, url: live.url, attempt, error: status === 'error' ? (error ?? null) : null }
  })
}

/** `org:changed`: the connections are re-read here, and every card re-reads its own lists. */
export const orgChanged = (): void => {
  useOrg.setState(state => ({ seq: state.seq + 1 }))
  void loadOrgConnections()
}
