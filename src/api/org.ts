import { call } from './client'

/**
 * A company's Rowel Server, as the app joins it: the member signs in through
 * the browser once, this device's enrollment waits until another of their
 * devices or an admin approves it, and from then on the server lists the org
 * lockboxes the member may hold. An org lockbox's key is the server's as much
 * as the member's — it has to act for agents while nobody is there — so every
 * screen that adds one says so in plain words.
 */

/** Where this device's enrollment stands with a server. */
export type OrgDeviceState = 'pending' | 'approved' | 'revoked'

export interface OrgConnection {
  /** The server, normalized, which is how every other command names it. */
  url: string
  tenantName: string
  memberEmail: string
  memberName: string | null
  /** Both null until the server has taken this device's enrollment. */
  deviceId: string | null
  deviceState: OrgDeviceState | null
}

/** One of the member's devices as the server lists them, this one included. */
export interface OrgDevice {
  id: string
  name: string
  state: OrgDeviceState
  /** The enrollment key's fingerprint, grouped the way `orgEnrollmentFingerprint` spells it. */
  fingerprint: string
  thisDevice: boolean
  createdAtMs: number
}

export interface OrgLockbox {
  id: string
  name: string
  /** The workspace it already is on this device, or null when it is only on the server. */
  workspaceId: string | null
}

/**
 * How a connect is going, as `org:connect` reports it. The command itself
 * resolves the moment the browser is open; the sign-in finishes out there, so
 * the outcome can only arrive as an event. `attempt` is the id `orgConnect`
 * resolved with, so the store can tell the live attempt's news from a late
 * word of one it has cancelled or replaced — Rust emits nothing for those,
 * but what was already on its way still lands.
 */
export interface OrgConnectStatus {
  url: string
  attempt: number
  status: 'pending' | 'done' | 'error'
  error?: string
}

/**
 * This device's enrollment key, as a fingerprint for a person to read out —
 * what another device or an admin compares before approving this one.
 */
export const orgEnrollmentFingerprint = (): Promise<string> => call('org_enrollment_fingerprint')

export const orgConnections = (): Promise<OrgConnection[]> => call('org_connections')

/**
 * Sign in to the server at `url` in the browser and enroll this device.
 * Resolves, once the browser is open, with the id of the attempt it began;
 * `org:connect` says how that attempt ended.
 */
export const orgConnect = (url: string): Promise<number> => call('org_connect', { url })

/** Give up on a sign-in still out with the browser. */
export const orgCancelConnect = (): Promise<void> => call('org_cancel_connect')

/**
 * Forget the server: the sign-in and the enrollment go. Org lockboxes already
 * on this device stay as they are, but stop syncing.
 */
export const orgDisconnect = (url: string): Promise<void> => call('org_disconnect', { url })

export const orgDevices = (url: string): Promise<OrgDevice[]> => call('org_devices', { url })

/** Approve another of the member's devices from this one, which has to be approved itself. */
export const orgApproveDevice = (url: string, deviceId: string): Promise<void> =>
  call('org_approve_device', { url, deviceId })

export const orgLockboxes = (url: string): Promise<OrgLockbox[]> => call('org_lockboxes', { url })

/**
 * Take the org lockbox `lockboxId` onto this device as a workspace of its own.
 * Resolves to the new workspace's id; the list of workspaces lives on
 * `app_status`, so the caller re-probes.
 */
export const orgAddLockbox = (url: string, lockboxId: string): Promise<string> =>
  call('org_add_lockbox', { url, lockboxId })

/** Fetch an org lockbox's key from its server again — after the server rotated it, say. */
export const orgRefreshLockboxKey = (workspaceId: string): Promise<void> =>
  call('org_refresh_lockbox_key', { workspaceId })

/**
 * A server as a person would name it: its host, without the scheme, port or
 * path the URL carries. For the places a lockbox says where it syncs.
 */
export const serverHost = (url: string): string => {
  try {
    return new URL(url).hostname
  } catch {
    return url
  }
}
