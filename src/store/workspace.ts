import type { Workspace } from '@/api/types'
import {
  workspaceAdoptVaultPassword,
  workspaceCreate,
  workspaceDelete,
  workspaceRestoreFromAccount,
  workspaceRestoreFromPending,
  workspaceSelect
} from '@/api/workspace'
import { PRIMARY_WORKSPACE } from '@/lib/workspace'
import {
  clearSession,
  enterMain,
  refreshApp,
  forgetBiometricGate,
  setSwitching,
  useApp,
  setupAccountRestoring,
  setupAccountRestoreFailed,
  type AppState
} from './app'
import { lockSettings } from './ui'

/**
 * Workspaces are not state of their own: the launch probe reports which exist
 * and which is active, and `app.status` already holds that answer whole. These
 * are the selectors over it and the two moves — switching and creating — that
 * change it.
 */

const NONE: Workspace[] = []

export const selectWorkspaces = (state: AppState): Workspace[] =>
  state.status?.workspaces ?? NONE

export const selectActiveWorkspace = (state: AppState): string =>
  state.status?.activeWorkspace ?? PRIMARY_WORKSPACE

// Move to another workspace. The app is unlocked as a whole, so the backend
// usually opens the next database itself and answers like an unlock: the same
// landing as a create — the previous workspace's data goes first, then the
// probe brings the new active workspace on screen. Only a workspace that has
// never been opened with its password on this device answers with nothing:
// that switch is a lock, announced as `vault:locked` like any other, and the
// one handler lands on that workspace's lock screen with the picker still
// offering the way back.
//
// Nothing on the lock screen changes until the probe after the switch answers:
// the chip's name and the biometric gate are both read off it, so they change
// in the one render that lands it. Meanwhile the switch is flagged, and the
// lock screen takes no attempt — the gate it still draws is the previous
// workspace's, and an unlock now would reach the next one. However it ends
// (opened, locked, refused), the switch probes once itself, so the flag never
// outlives it; a probe that fails leaves the safe default, no gate offered.
//
// One at a time. A second pick while the first is in flight is dropped rather
// than raced: the backend refuses it anyway, and its ending would clear the
// flag — and take attempts again — while the first was still landing.
export const switchWorkspace = async (id: string) => {
  if (useApp.getState().switching) return
  setSwitching(true)
  try {
    const result = await workspaceSelect(id)
    if (result) {
      clearSession()
      await enterMain(result)
    }
  } finally {
    if (!(await refreshApp())) forgetBiometricGate()
    setSwitching(false)
  }
}

// Remove a workspace's vault from this device, and with `everywhere` from the
// account it syncs to as well. The probe is what carries the list, so re-reading
// it is the whole update here — and when the deleted one was the open one the
// backend has already announced the lock, which routes to the survivor's lock
// screen by the one path every lock takes. Settings stays pinned while the
// deletion runs; rejections are the caller's to show under the password field.
export const deleteWorkspace = async (
  id: string,
  password: string,
  everywhere: boolean
) => {
  // A delete may finish after an Argon2id proof and a request to the account. Hold every
  // way out of Settings until it has landed, or backing out only hides work that
  // is still going to remove the workspace.
  lockSettings(true)
  try {
    await workspaceDelete(id, password, everywhere)
    await refreshApp()
  } finally {
    lockSettings(false)
  }
}

// Create a workspace and open it. It arrives active and unlocked, so this is an
// unlock rather than a first run — but of a different database, so the session
// data of the one being left has to go first, exactly as a lock would drop it.
// The re-probe is what brings the new list on screen: which workspaces exist
// and which is active is the probe's answer, and this is the one move that
// changes it without passing through a lock.
export const createWorkspace = async (
  name: string,
  password: string,
  color?: string | null
) => {
  const result = await workspaceCreate(name, password, color)
  clearSession()
  await enterMain(result)
  await refreshApp()
}

// Add a workspace by restoring one of the connected account's other vaults.
// The same landing as a create — it arrives active and unlocked, so the
// previous workspace's data goes first and the probe brings the new list on
// screen — and the same landing as the first run's restore too: the result says
// `syncConfigured`, which is what has `enterMain` run the first sync.
//
// The store, not the form, says a restore is running: the backend has claimed
// the pending account for its length and refuses to give it up, so every
// control that would (Cancel, Switch account) has to know to stand down — and
// the form is not the only thing drawing them. Settings is held shut and on
// its section for the same reason: closing it or moving away would have run the
// same refused disconnect and then let the restore finish and switch
// workspaces behind the user's back.
export const restoreWorkspaceFromPending = async (
  name: string,
  password: string,
  fileId: string
) => {
  setupAccountRestoring()
  lockSettings(true)
  let result
  try {
    result = await workspaceRestoreFromPending(name, password, fileId)
  } catch (error) {
    setupAccountRestoreFailed()
    throw error
  } finally {
    lockSettings(false)
  }
  clearSession()
  await enterMain(result)
  await refreshApp()
}

// The end of a restore whose pack opened with a password other than the vault
// password. The backend kept the lockbox, closed, and named it in the
// `vaultPasswordRequired` it rejected with; moving it onto the vault password
// seals it under the app key, so the `switchWorkspace` the caller runs next
// opens it without asking — the same landing a restore that matched from the
// start has. Two steps the caller sees apart: the move cannot be taken back,
// so a switch that fails after it is retried on its own. Settings is held for
// the move, as for the restore it finishes.
export const moveRestoredLockbox = async (
  workspaceId: string,
  lockboxPassword: string,
  vaultPassword: string
) => {
  lockSettings(true)
  try {
    await workspaceAdoptVaultPassword(workspaceId, lockboxPassword, vaultPassword)
  } finally {
    lockSettings(false)
  }
}

// The same landing, for a vault the open workspace's account holds and this
// device does not. No probe state to keep: there was no sign-in, so nothing is
// pending and nothing has to be forgotten. Settings is held for the same reason
// as above — the restore ends by switching workspaces, and a navigation that
// looked like backing out would let that happen behind the user's back.
export const restoreWorkspaceFromAccount = async (
  name: string,
  password: string,
  fileId: string
) => {
  lockSettings(true)
  let result
  try {
    result = await workspaceRestoreFromAccount(name, password, fileId)
  } finally {
    lockSettings(false)
  }
  clearSession()
  await enterMain(result)
  await refreshApp()
}
