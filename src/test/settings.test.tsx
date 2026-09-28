import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import { act, render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import Settings from '@/components/Main/Sidebar/Settings'
import i18n, { changeLocale } from '@/i18n'
import { dates } from '@/utils/time'
import { open as openDialog } from '@tauri-apps/plugin-dialog'
import type { SyncStatus } from '@/api/sync'
import type { SetupDriveFile } from '@/api/setup'
import {
  auditDone,
  closeSettings,
  fileOpened,
  flowMain,
  initialApp,
  lockSettings,
  openSettings,
  setRemoteVaults,
  setSettingsSection,
  setSyncStatus,
  setupDriveFailed,
  setupDrivePending,
  setupDriveProbed,
  useApp,
  usePrefs,
  useUi
} from '@/store'
import DateField from '@/components/elements/fields/DateField'
import { FieldsProvider } from '@/components/elements/fields/context'
import Footer from '@/components/Main/Body/Aside/Show/Footer'
import { appStatusDefault, calls, mockCommand, mockCommandOnce } from './ipc'
import { loginMeta, seedApp, withEntries } from './utils'

beforeEach(() => vi.clearAllMocks())

const report = (s: Partial<SyncStatus>) => setSyncStatus({ ...initialApp.sync, ...s })

afterEach(() => changeLocale('en-US'))

// Only the biometric leaf of app_status matters here; the row reads nothing
// else. Seeded into the store as what the boot probe said, and returned by the
// mocked probe as what a re-run after a toggle says.
const enrolled = (available: boolean, mode: 'protected' | 'prompt' | null) => ({
  ...appStatusDefault(),
  biometric: { available, canEnroll: true, type: 'touch' as const, mode }
})

const open = async () => {
  const { container } = render(<Settings />)
  await userEvent.click(container.querySelector('.settings-button')!)
  return { container }
}

const go = (section: string) =>
  userEvent.click(screen.getByTestId(`settings-nav-${section}`))

// Settings opens on General; the sync tests want the Sync & backup pane.
const openSync = async () => {
  const opened = await open()
  await go('sync')
  return opened
}

// A workspace row's ⋯, which is where Edit and Delete live.
const openMenu = (id: string) => userEvent.click(screen.getByTestId(`workspace-menu-${id}`))

describe('Settings shell', () => {
  it('opens on General, the first section in the nav', async () => {
    await open()
    expect(screen.getByTestId('settings-modal')).toBeInTheDocument()
    expect(screen.getByRole('heading', { name: 'General' })).toBeInTheDocument()
    expect(
      screen.getByText('Appearance, language and how dates are shown.')
    ).toBeInTheDocument()
    expect(screen.getByTestId('settings-nav-language')).toHaveAttribute('aria-current', 'page')
  })

  it('groups the nav under App, Vault and Data', async () => {
    await open()
    const nav = screen.getByRole('navigation')
    const order = [...nav.querySelectorAll('[data-testid^="settings-nav-"]')].map(node =>
      node.getAttribute('data-testid')
    )
    expect(order).toEqual([
      'settings-nav-language',
      'settings-nav-browser',
      'settings-nav-security',
      'settings-nav-audit',
      'settings-nav-workspaces',
      'settings-nav-sync',
      'settings-nav-import'
    ])
    for (const group of ['App', 'Vault', 'Data'])
      expect(within(nav).getByText(group)).toBeInTheDocument()
  })

  it('badges the audit row with the open issue count', async () => {
    // One weak-and-reused entry counts twice, as the audit section's tallies do.
    usePrefs.setState({ breachCheck: true })
    auditDone({
      a: { score: 1, isWeak: true, isRepeating: true, breached: false },
      b: { score: 4, isWeak: false, isRepeating: false, breached: true },
      c: { score: 4, isWeak: false, isRepeating: false, breached: false }
    })
    await open()
    expect(screen.getByTestId('settings-nav-audit-badge')).toHaveTextContent('3')
  })

  // The section shows no breach count while monitoring is off, and the badge
  // must agree with it — even over an audit that ran while it was on.
  it('leaves breaches out of the badge while monitoring is off', async () => {
    usePrefs.setState({ breachCheck: false })
    auditDone({
      a: { score: 1, isWeak: true, isRepeating: false, breached: false },
      b: { score: 4, isWeak: false, isRepeating: false, breached: true }
    })
    await open()
    expect(screen.getByTestId('settings-nav-audit-badge')).toHaveTextContent('1')
  })

  it('shows no audit badge when nothing is open', async () => {
    auditDone({ a: { score: 4, isWeak: false, isRepeating: false, breached: false } })
    await open()
    expect(screen.queryByTestId('settings-nav-audit-badge')).not.toBeInTheDocument()
  })

  it('switches sections from the nav and remembers the last one', async () => {
    await open()

    await go('audit')
    expect(screen.getByRole('heading', { name: 'Vault audit' })).toBeInTheDocument()
    expect(useUi.getState().settingsSection).toBe('audit')

    await go('language')
    expect(
      screen.getByRole('heading', { name: 'General' })
    ).toBeInTheDocument()
    expect(screen.getByTestId('settings-nav-language')).toHaveAttribute(
      'aria-current',
      'page'
    )
  })

  it('deep-links to a section through openSettings', async () => {
    render(<Settings />)
    openSettings('security')
    expect(await screen.findByRole('heading', { name: 'Security' })).toBeInTheDocument()
  })

  it('closes from the header X', async () => {
    await open()
    const close = screen.getByTestId('modal-close')
    expect(close).toHaveAccessibleName('Close')
    expect(close).toHaveTextContent('esc')
    await userEvent.click(close)
    expect(useUi.getState().settings).toBe(false)
  })
})

describe('Settings › sync', () => {
  it('connects Google Drive', async () => {
    await openSync()
    await userEvent.click(screen.getByTestId('settings-drive-connect'))
    expect(calls('sync_connect')).toHaveLength(1)
  })

  // Every workspace holds its own Drive connection and syncs its own pack now,
  // so a second one is offered the same controls as the primary rather than a
  // notice telling it where to go instead.
  it('offers Drive in a workspace that is not the primary', async () => {
    seedApp({
      workspaces: [
        { id: 'default', name: null },
        { id: 'w2', name: 'Work' }
      ],
      activeWorkspace: 'w2'
    })
    await openSync()

    expect(screen.queryByTestId('settings-sync-primary-only')).not.toBeInTheDocument()
    await userEvent.click(screen.getByTestId('settings-drive-connect'))
    expect(calls('sync_connect')).toHaveLength(1)
  })

  // The mobile shape of the same flow: `sync_connect` resolves as soon as
  // Safari has the screen, so the row waits on the backend's events — the
  // click itself claims nothing.
  it('waits for Google after a connect that resolved early', async () => {
    await openSync()
    await userEvent.click(screen.getByTestId('settings-drive-connect'))
    expect(useApp.getState().sync.pending).toBe(false)

    report({ pending: true })
    expect(await screen.findByText('Waiting for Google…')).toBeInTheDocument()

    report({ configured: true })
    expect(await screen.findByText('Connected')).toBeInTheDocument()
  })

  it('reports a consent that failed, and stays disconnected', async () => {
    await openSync()
    await userEvent.click(screen.getByTestId('settings-drive-connect'))
    report({ pending: true })
    report({ error: 'access_denied' })

    expect(await screen.findByTestId('settings-sync-error')).toHaveTextContent(
      'access_denied'
    )
    expect(useApp.getState().sync.configured).toBe(false)
  })

  it('surfaces a connect that could not even start', async () => {
    mockCommandOnce('sync_connect', () => {
      report({ error: 'no OAuth client configured' })
      return Promise.reject({ kind: 'other', message: 'no OAuth client configured' })
    })
    await openSync()
    await userEvent.click(screen.getByTestId('settings-drive-connect'))

    await waitFor(() => expect(useApp.getState().sync.pending).toBe(false))
    expect(screen.getByTestId('settings-sync-error')).toHaveTextContent(
      'no OAuth client configured'
    )
  })

  // Every connect is keyless and answered on the probe's events rather than on
  // `sync:status`: the backend probes the account, and `sync_adopt_pending`
  // then says whether this vault may join it — an empty account, or one that
  // already holds this vault — or whether its vaults are to be restored instead.
  describe('connecting a vault that is not connected', () => {
    // The backend's refusal: the account holds vaults, none of them this one.
    const refuse = () =>
      mockCommandOnce('sync_adopt_pending', () =>
        Promise.reject({
          kind: 'vaultNotInAccount',
          message: 'this Google account holds other vaults'
        })
      )

    it('waits on the probe, then takes an empty account as this vault', async () => {
      await openSync()
      await userEvent.click(screen.getByTestId('settings-drive-connect'))
      await act(async () => setupDrivePending())
      expect(await screen.findByText('Waiting for Google…')).toBeInTheDocument()

      await act(async () => setupDriveProbed([]))

      await waitFor(() => expect(calls('sync_adopt_pending')).toHaveLength(1))
      // The probe's state is spent once the account is this vault's; from here
      // on `sync:status` says the rest.
      await waitFor(() => expect(useApp.getState().setupDrive.status).toBe('idle'))
      report({ configured: true })
      expect(await screen.findByText('Connected')).toBeInTheDocument()
    })

    // A second device of a vault the account already holds: the backend adopts
    // and the picker is never shown.
    it('joins an account that already holds this vault, with no picker', async () => {
      await openSync()
      await userEvent.click(screen.getByTestId('settings-drive-connect'))
      await act(async () => setupDriveProbed([VAULT, OTHER_VAULT]))

      await waitFor(() => expect(calls('sync_adopt_pending')).toHaveLength(1))
      await waitFor(() => expect(useApp.getState().setupDrive.status).toBe('idle'))
      expect(screen.queryByTestId('settings-drive-found')).not.toBeInTheDocument()
      report({ configured: true })
      expect(await screen.findByText('Connected')).toBeInTheDocument()
    })

    // Until the backend has said, the row waits rather than offering vaults
    // the user may not need to choose from.
    it('waits while the backend decides, then follows its answer', async () => {
      let decide: () => void = () => {}
      mockCommandOnce(
        'sync_adopt_pending',
        () => new Promise<void>(resolve => (decide = resolve))
      )
      await openSync()
      await userEvent.click(screen.getByTestId('settings-drive-connect'))
      await act(async () => setupDriveProbed([VAULT]))

      await waitFor(() => expect(calls('sync_adopt_pending')).toHaveLength(1))
      expect(screen.getByText('Waiting for Google…')).toBeInTheDocument()
      expect(screen.queryByTestId('settings-drive-found')).not.toBeInTheDocument()

      await act(async () => decide())
      await waitFor(() => expect(useApp.getState().setupDrive.status).toBe('idle'))
    })

    // The account already says which vaults exist, so this vault does not add
    // to them: the user restores one, with the same picker and form Settings ›
    // Workspaces uses, and the vault that was open stays as its own workspace.
    it("offers the account's vaults to restore when it holds only other ones", async () => {
      refuse()
      await openSync()
      await userEvent.click(screen.getByTestId('settings-drive-connect'))
      await act(async () => setupDriveProbed([VAULT, OTHER_VAULT]))

      expect(await screen.findByTestId('settings-drive-found')).toBeInTheDocument()
      expect(screen.getByText('This account already holds a vault')).toBeInTheDocument()
      expect(screen.getByTestId(`drive-vault-${VAULT.id}`)).toBeInTheDocument()
      // Asked once, and refused; the tokens are still pending for the restore.
      expect(calls('sync_adopt_pending')).toHaveLength(1)
      expect(useApp.getState().setupDrive.status).toBe('found')

      await userEvent.click(screen.getByTestId(`drive-vault-${OTHER_VAULT.id}`))
      await userEvent.type(screen.getByTestId('workspace-restore-name'), 'Personal')
      await userEvent.type(screen.getByTestId('workspace-restore-password'), 'other-device-pass')
      await act(async () => {
        await userEvent.click(screen.getByTestId('workspace-restore-submit'))
      })

      expect(calls('workspace_restore_from_drive')).toContainEqual({
        name: 'Personal',
        password: 'other-device-pass',
        fileId: OTHER_VAULT.id
      })
    })

    it('cancels out of the offer by forgetting the account', async () => {
      refuse()
      await openSync()
      await userEvent.click(screen.getByTestId('settings-drive-connect'))
      await act(async () => setupDriveProbed([VAULT]))

      await userEvent.click(await screen.findByTestId('settings-drive-cancel'))

      expect(calls('setup_drive_disconnect')).toHaveLength(1)
      expect(screen.queryByTestId('settings-drive-found')).not.toBeInTheDocument()
      expect(screen.getByTestId('settings-drive-connect')).toBeInTheDocument()
    })

    // Any other reason the backend could not adopt (the account did not answer,
    // the vault locked) is a failure like the probe's, not an offer.
    it('reports an adopt that failed for another reason', async () => {
      mockCommandOnce('sync_adopt_pending', () =>
        Promise.reject({ kind: 'other', message: 'Google Drive did not answer' })
      )
      await openSync()
      await userEvent.click(screen.getByTestId('settings-drive-connect'))
      await act(async () => setupDriveProbed([]))

      expect(await screen.findByTestId('settings-sync-error')).toHaveTextContent(
        'Google Drive did not answer'
      )
      expect(screen.queryByTestId('settings-drive-found')).not.toBeInTheDocument()
      expect(screen.getByTestId('settings-drive-connect')).toBeInTheDocument()
    })

    it('reports a probe that failed, and offers to try again', async () => {
      await openSync()
      await userEvent.click(screen.getByTestId('settings-drive-connect'))
      await act(async () => setupDriveFailed('Google Drive did not answer'))

      expect(await screen.findByTestId('settings-sync-error')).toHaveTextContent(
        'Google Drive did not answer'
      )
      expect(screen.getByTestId('settings-drive-connect')).toBeInTheDocument()
      expect(useApp.getState().sync.configured).toBe(false)
    })
  })

  it('offers the encrypted backup behind its own control', async () => {
    await openSync()
    expect(screen.getByTestId('settings-backup-row')).toHaveTextContent('.rowel')
    expect(document.querySelector('input[name="export_password"]')).toBeNull()
    await userEvent.click(screen.getByText('Save…'))
    expect(document.querySelector('input[name="export_password"]')).toBeInTheDocument()
  })

  // The portable export lives with Import now; this section keeps only the
  // encrypted backup.
  it('leaves the unencrypted export out of this section', async () => {
    await openSync()
    expect(screen.queryByTestId('settings-export-run')).not.toBeInTheDocument()
  })

  // The pill beside the title says where the connection stands, one tone per
  // state, with a sync running outranking the last one's failure.
  it('says where Drive stands in the status pill', async () => {
    await openSync()
    const pill = () => screen.getByTestId('settings-drive-status')

    expect(pill()).toHaveTextContent('Not connected')
    expect(pill()).toHaveAttribute('data-tone', 'idle')

    act(() => report({ configured: true }))
    expect(pill()).toHaveTextContent('Up to date')
    expect(pill()).toHaveAttribute('data-tone', 'good')

    act(() => report({ configured: true, inProgress: true }))
    expect(pill()).toHaveTextContent('Syncing')
    expect(pill()).toHaveAttribute('data-tone', 'busy')

    act(() => report({ configured: true, error: 'Drive API 503' }))
    expect(pill()).toHaveTextContent('Last attempt failed')
    expect(pill()).toHaveAttribute('data-tone', 'bad')
    expect(screen.getByTestId('settings-sync-error')).toHaveTextContent('Drive API 503')
  })

  it('shows what a connected account holds, and the way out', async () => {
    await openSync()
    expect(screen.queryByTestId('settings-sync-now')).not.toBeInTheDocument()
    expect(screen.queryByText('End-to-end')).not.toBeInTheDocument()

    act(() => report({ configured: true }))
    expect(screen.queryByTestId('settings-drive-connect')).not.toBeInTheDocument()
    expect(screen.getByText('End-to-end')).toBeInTheDocument()

    await userEvent.click(screen.getByTestId('settings-sync-now'))
    expect(calls('sync_now')).toHaveLength(1)
    await userEvent.click(screen.getByTestId('settings-drive-disconnect'))
    expect(calls('sync_disconnect')).toHaveLength(1)
  })
})

describe('Settings › security', () => {
  it('changes the master password', async () => {
    // Nothing came back: the change reached every workspace on the device.
    mockCommand('change_master_password', () => [])
    await open()
    await go('security')
    await userEvent.click(screen.getByText('Change…'))

    await userEvent.type(document.querySelector('input[name="current_password"]')!, 'old')
    await userEvent.type(document.querySelector('input[name="new_password"]')!, 'newpass')
    await userEvent.type(
      document.querySelector('input[name="new_password_repeat"]')!,
      'newpass'
    )
    await userEvent.click(screen.getByTestId('change-password-submit'))

    expect(calls('change_master_password')).toContainEqual({ current: 'old', new: 'newpass' })
    expect(await screen.findByText('Successfully changed password')).toBeInTheDocument()
  })

  // A workspace on a password of its own is left on it, and so is one whose own
  // re-key failed. The change itself succeeded, so it is still the success line
  // that is shown — it just says the new password does not open everything.
  it('says so when the change did not reach every workspace', async () => {
    mockCommand('change_master_password', () => ['a1b2c3d4'])
    await open()
    await go('security')
    await userEvent.click(screen.getByText('Change…'))

    await userEvent.type(document.querySelector('input[name="current_password"]')!, 'old')
    await userEvent.type(document.querySelector('input[name="new_password"]')!, 'newpass')
    await userEvent.type(
      document.querySelector('input[name="new_password_repeat"]')!,
      'newpass'
    )
    await userEvent.click(screen.getByTestId('change-password-submit'))

    expect(
      await screen.findByText(
        'Password changed. Some workspaces on this device kept their own password.'
      )
    ).toBeInTheDocument()
  })

  it('reports a rejected master password change', async () => {
    mockCommand('change_master_password', () => Promise.reject({ kind: 'invalidPassword', message: 'invalid master password' }))
    await open()
    await go('security')
    await userEvent.click(screen.getByText('Change…'))

    await userEvent.type(document.querySelector('input[name="current_password"]')!, 'bad')
    await userEvent.type(document.querySelector('input[name="new_password"]')!, 'newpass')
    await userEvent.type(
      document.querySelector('input[name="new_password_repeat"]')!,
      'newpass'
    )
    await userEvent.click(screen.getByTestId('change-password-submit'))

    expect(await screen.findByTestId('change-password-error')).toBeInTheDocument()
  })

  // The row is redrawn from a fresh probe after the toggle, not from a guess.
  it('enables biometric unlock and redraws from the refreshed status', async () => {
    seedApp(enrolled(false, null))
    mockCommand('app_status', () => enrolled(true, 'protected'))
    mockCommand('enable_biometric', () => 'protected')
    await open()
    await go('security')

    const toggle = await screen.findByTestId('settings-biometric-toggle')
    expect(toggle).toHaveAttribute('aria-checked', 'false')
    await userEvent.click(toggle)

    expect(calls('enable_biometric')).toHaveLength(1)
    expect(await screen.findByTestId('settings-biometric-toggle')).toHaveAttribute(
      'aria-checked',
      'true'
    )
  })

  it('disables biometric unlock from the toggle', async () => {
    seedApp(enrolled(true, 'prompt'))
    mockCommand('app_status', () => enrolled(false, null))
    mockCommand('disable_biometric', () => undefined)
    await open()
    await go('security')

    await userEvent.click(await screen.findByTestId('settings-biometric-toggle'))

    expect(calls('disable_biometric')).toHaveLength(1)
    await waitFor(() =>
      expect(screen.getByTestId('settings-biometric-toggle')).toHaveAttribute(
        'aria-checked',
        'false'
      )
    )
  })

  // The copy must name the gate actually in force: an OS-enforced Secure Enclave
  // item and an app-enforced verify-then-read item are different promises.
  it('describes the OS-enforced gate when enrolled in protected mode', async () => {
    seedApp(enrolled(true, 'protected'))
    await open()
    await go('security')
    expect(await screen.findByText(/Secure Enclave/)).toBeInTheDocument()
  })

  it('switches the copy to the mode enrollment settled on', async () => {
    // An unentitled build falls back to prompt mode; the description must follow
    // the enable response rather than keep advertising the generic offer.
    seedApp(enrolled(false, null))
    mockCommand('app_status', () => enrolled(true, 'prompt'))
    mockCommand('enable_biometric', () => 'prompt')
    await open()
    await go('security')
    await userEvent.click(await screen.findByTestId('settings-biometric-toggle'))

    expect(
      await screen.findByText(/released after a biometric check by Rowel/)
    ).toBeInTheDocument()
  })

  // One write: Rust owns the file and re-arms the auto-lock from it, so there
  // is no second command to push the value with.
  it('stores the auto-lock choice through the settings file', async () => {
    await open()
    await go('security')
    await userEvent.click(screen.getByTestId('settings-autolock-300'))

    expect(usePrefs.getState().autolockSecs).toBe(300)
    expect(calls('set_settings')).toContainEqual({ patch: { autolockSecs: 300 } })
  })

  it('stores the clipboard delay, "Never" included', async () => {
    await open()
    await go('security')

    await userEvent.click(screen.getByTestId('settings-clipboard-15000'))
    expect(usePrefs.getState().clipboardTimeoutMs).toBe(15000)

    await userEvent.click(screen.getByTestId('settings-clipboard-0'))
    expect(usePrefs.getState().clipboardTimeoutMs).toBe(0)
  })

  it('names both session radiogroups after their rows', async () => {
    await open()
    await go('security')

    expect(screen.getByRole('radiogroup', { name: 'Lock vault after' })).toBeInTheDocument()
    expect(screen.getByRole('radiogroup', { name: 'Clear clipboard' })).toBeInTheDocument()
  })

  it('writes the generator defaults the dialog reads', async () => {
    await open()
    await go('security')

    await userEvent.click(screen.getByTestId('settings-generator-symbols'))
    expect(usePrefs.getState().generator.symbols).toBe(false)

    await userEvent.click(screen.getByTestId('settings-generator-uppercase'))
    expect(usePrefs.getState().generator.uppercase).toBe(false)
    expect(screen.getByTestId('settings-generator-uppercase')).toHaveAttribute(
      'aria-pressed',
      'false'
    )
  })

  it('keeps the last character class on', async () => {
    await open()
    await go('security')

    for (const flag of ['uppercase', 'numbers', 'symbols'])
      await userEvent.click(screen.getByTestId(`settings-generator-${flag}`))

    const lowercase = screen.getByTestId('settings-generator-lowercase')
    expect(lowercase).toHaveAttribute('aria-disabled', 'true')
    await userEvent.click(lowercase)
    expect(usePrefs.getState().generator.lowercase).toBe(true)
  })

  it('draws a sample from the defaults and redraws it on every change', async () => {
    let draws = 0
    mockCommand('generate_password', () => `Sample${++draws}!`)
    await open()
    await go('security')

    const sample = screen.getByTestId('settings-generator-sample')
    await waitFor(() => expect(sample).toHaveTextContent('Sample1!'))

    await userEvent.click(screen.getByTestId('settings-generator-regenerate'))
    await waitFor(() => expect(sample).toHaveTextContent('Sample2!'))

    await userEvent.click(screen.getByTestId('settings-generator-numbers'))
    await waitFor(() => expect(sample).toHaveTextContent('Sample3!'))
    expect(usePrefs.getState().generator.numbers).toBe(false)
    expect(calls('generate_password').slice(-1)[0]).toMatchObject({
      options: expect.objectContaining({ numbers: false })
    })
  })

  it('labels the change form and flags a repeat that does not match', async () => {
    await open()
    await go('security')
    await userEvent.click(screen.getByText('Change…'))

    await userEvent.type(screen.getByLabelText('New Password'), 'newpass')
    await userEvent.type(screen.getByLabelText('Repeat New Password'), 'newpa')
    expect(screen.getByText("Passwords don't match yet.")).toBeInTheDocument()
    expect(screen.getByTestId('change-password-submit')).toBeDisabled()

    await userEvent.type(screen.getByLabelText('Repeat New Password'), 'ss')
    expect(screen.queryByText("Passwords don't match yet.")).not.toBeInTheDocument()

    const field = screen.getByLabelText('New Password')
    expect(field).toHaveAttribute('type', 'password')
    await userEvent.click(screen.getByRole('button', { name: 'Show' }))
    expect(field).toHaveAttribute('type', 'text')
  })
})

describe('Settings › vault audit', () => {
  it('toggles breach monitoring and re-runs the audit', async () => {
    await open()
    await go('audit')

    expect(usePrefs.getState().breachCheck).toBe(false)
    await userEvent.click(screen.getByTestId('settings-breach-toggle'))

    expect(usePrefs.getState().breachCheck).toBe(true)
    expect(calls('get_audit')).toContainEqual({ checkBreaches: true })
  })

  it('leaves the always-on monitors without a fake control', async () => {
    await open()
    await go('audit')
    expect(screen.getByText('Weak & reused passwords')).toBeInTheDocument()
    expect(screen.getByText('Always on')).toBeInTheDocument()
    // Breach monitoring is the only switch on this section.
    expect(screen.getAllByRole('switch')).toHaveLength(1)
  })

  it('jumps to the Vault Health view and closes', async () => {
    await open()
    await go('audit')
    await userEvent.click(screen.getByTestId('settings-open-health'))

    expect(useUi.getState().view).toBe('health')
    expect(useUi.getState().settings).toBe(false)
  })

  it('reviews reused passwords from the stat strip too', async () => {
    await open()
    await go('audit')
    await userEvent.click(screen.getByTestId('settings-open-health-reused'))

    expect(useUi.getState().view).toBe('health')
    expect(useUi.getState().settings).toBe(false)
  })

  it('says nothing has been scanned before the first audit', async () => {
    await open()
    await go('audit')
    expect(screen.getByTestId('settings-audit-verdict')).toHaveTextContent('Not scanned yet')
    expect(screen.getByTestId('settings-audit-score')).toHaveTextContent('—')
  })

  it('puts the score in a word and counts what needs review', async () => {
    withEntries([loginMeta({ id: 'l1' }), loginMeta({ id: 'l2' })], {
      l1: { score: 0, isWeak: true, isRepeating: false, breached: false },
      l2: { score: 2, isWeak: false, isRepeating: true, breached: true }
    })
    await open()
    await go('audit')

    expect(screen.getByTestId('settings-audit-verdict')).toHaveTextContent('Weak')
    expect(screen.getByText('2 items')).toBeInTheDocument()
    const strip = screen.getByTestId('settings-audit-counts')
    expect(strip).toHaveTextContent('1weak')
    expect(strip).toHaveTextContent('1reused')
    // Breaches are not counted while monitoring is off; the strip offers it.
    expect(strip).toHaveTextContent('—breached')
    expect(screen.getByTestId('settings-breach-enable')).toHaveTextContent('Turn on monitoring')
  })

  it('calls a healthy vault strong', async () => {
    withEntries([loginMeta()], {
      l1: { score: 4, isWeak: false, isRepeating: false, breached: false }
    })
    await open()
    await go('audit')
    expect(screen.getByTestId('settings-audit-verdict')).toHaveTextContent('Strong')
  })

  it('turns breach monitoring on from the stat strip', async () => {
    await open()
    await go('audit')
    await userEvent.click(screen.getByTestId('settings-breach-enable'))

    expect(usePrefs.getState().breachCheck).toBe(true)
    expect(calls('get_audit')).toContainEqual({ checkBreaches: true })
    expect(screen.getByTestId('settings-breach-toggle')).toHaveAttribute('aria-checked', 'true')
    expect(screen.queryByTestId('settings-breach-enable')).not.toBeInTheDocument()
  })

  it('re-runs the audit from Run now', async () => {
    await open()
    await go('audit')
    await userEvent.click(screen.getByTestId('settings-audit-run'))
    expect(calls('get_audit')).toContainEqual({ checkBreaches: false })
  })

  it('unfolds how breach monitoring works and folds it away again', async () => {
    await open()
    await go('audit')
    const explain = screen.getByTestId('settings-breach-explain')
    expect(explain).toHaveTextContent('How it works')
    expect(screen.queryByTestId('settings-breach-explainer')).not.toBeInTheDocument()

    await userEvent.click(explain)
    expect(explain).toHaveAttribute('aria-expanded', 'true')
    expect(explain).toHaveTextContent('Hide details')
    const explainer = screen.getByTestId('settings-breach-explainer')
    expect(explainer).toHaveTextContent('5BAA61E4C9B93F3F0682250B6CF8331B7EE68FD8')
    expect(explainer).toHaveTextContent(/k-anonymity/)

    await userEvent.click(explain)
    expect(screen.queryByTestId('settings-breach-explainer')).not.toBeInTheDocument()
  })
})

describe('Settings › import', () => {
  it('imports a .swftx file with the source password', async () => {
    vi.mocked(openDialog).mockResolvedValue('/tmp/other.swftx')
    mockCommand('import_swftx', () => 3)
    await open()
    await go('import')

    await userEvent.click(screen.getByTestId('import-tile-swftx'))
    expect(await screen.findByText('other.swftx')).toBeInTheDocument()

    await userEvent.type(
      document.querySelector('input[name="import_password"]')!,
      'source-pw'
    )
    await userEvent.click(screen.getByTestId('import-run-backup'))

    expect(calls('import_swftx')).toContainEqual({ path: '/tmp/other.swftx', password: 'source-pw' })
    expect(await screen.findByText(/Imported/)).toBeInTheDocument()
  })

  it('shows an error when the backup password is wrong', async () => {
    vi.mocked(openDialog).mockResolvedValue('/tmp/other.swftx')
    mockCommand('import_swftx', () => Promise.reject({ kind: 'invalidPassword', message: 'invalid master password' }))
    await open()
    await go('import')

    await userEvent.click(screen.getByTestId('import-tile-swftx'))
    await screen.findByText('other.swftx')
    await userEvent.click(screen.getByTestId('import-run-backup'))

    expect(await screen.findByText('Invalid password for backup')).toBeInTheDocument()
  })

  // A legacy vault double-clicked while this one is open: Settings opens on
  // Import with it picked, as if from the tile, waiting for its password.
  it('picks a .swftx the OS opened with the app and asks for its password', async () => {
    flowMain()
    render(<Settings />)

    fileOpened('/tmp/old-vault.swftx')

    expect(useUi.getState().settings).toBe(true)
    expect(useUi.getState().settingsSection).toBe('import')
    expect(await screen.findByText('old-vault.swftx')).toBeInTheDocument()
    expect(document.querySelector('input[name="import_password"]')).toBeInTheDocument()
    expect(useApp.getState().openedFile).toBeNull()
  })

  // One of our own backups is a whole sealed database; with a vault already
  // open there is nothing to merge it into, and the section says so instead
  // of asking for a password it could not use.
  it('names a .rowel the OS opened and explains it is not imported', async () => {
    flowMain()
    render(<Settings />)

    fileOpened('/tmp/Rowel backup 2026-09-15.rowel')

    expect(await screen.findByText('Rowel backup 2026-09-15.rowel')).toBeInTheDocument()
    expect(screen.getByTestId('import-rowel-notice')).toBeInTheDocument()
    expect(document.querySelector('input[name="import_password"]')).not.toBeInTheDocument()
    expect(calls('import_swftx')).toEqual([])
  })

  it('offers a tile per source and no format select', async () => {
    await open()
    await go('import')
    for (const key of [
      'bitwarden',
      'cxf',
      'chrome',
      'lastpass',
      'keepass',
      'csv',
      'swftx'
    ])
      expect(screen.getByTestId(`import-tile-${key}`)).toBeInTheDocument()
    expect(screen.queryByRole('combobox')).not.toBeInTheDocument()
  })

  // The picker is on the drop target itself, on every platform; a file chosen
  // there has no tile behind it, so the backend sniffs its format.
  it('chooses an export file from the drop target', async () => {
    vi.mocked(openDialog).mockResolvedValue('/tmp/export.csv')
    await open()
    await go('import')

    await userEvent.click(screen.getByTestId('import-choose-file'))

    await waitFor(() =>
      expect(calls('import_entries')).toContainEqual({
        path: '/tmp/export.csv',
        format: 'auto',
        dryRun: true
      })
    )
  })

  it('switches between the import and export panes', async () => {
    await open()
    await go('import')
    expect(screen.getByTestId('settings-io-import')).toHaveAttribute('aria-checked', 'true')
    expect(screen.getByTestId('import-dropzone')).toBeInTheDocument()
    expect(screen.queryByTestId('settings-export-run')).not.toBeInTheDocument()

    await userEvent.click(screen.getByTestId('settings-io-export'))
    expect(screen.getByTestId('settings-io-export')).toHaveAttribute('aria-checked', 'true')
    expect(screen.queryByTestId('import-dropzone')).not.toBeInTheDocument()
    expect(screen.getByTestId('settings-export-run')).toBeInTheDocument()

    await userEvent.click(screen.getByTestId('settings-io-import'))
    expect(screen.getByTestId('import-tile-bitwarden')).toBeInTheDocument()
  })
})

describe('Settings › export', () => {
  const exportPane = async () => {
    await open()
    await go('import')
    await userEvent.click(screen.getByTestId('settings-io-export'))
  }

  // One warning covers every format, CXF included — it is as plaintext as the
  // other two — and nothing is written until it has been acknowledged.
  it('exports to CXF once the risk is acknowledged', async () => {
    mockCommand('export_entries', () => '/tmp/rowel-export.json')
    await exportPane()

    expect(screen.getByTestId('settings-export-format-bitwarden')).toHaveAttribute(
      'aria-checked',
      'true'
    )
    await userEvent.click(screen.getByTestId('settings-export-format-cxf'))
    expect(screen.getByTestId('settings-export-format-cxf')).toHaveAttribute(
      'aria-checked',
      'true'
    )

    expect(screen.getByTestId('settings-export-run')).toBeDisabled()
    await userEvent.click(screen.getByTestId('settings-export-ack'))
    expect(screen.getByTestId('settings-export-ack')).toHaveAttribute('aria-checked', 'true')
    await userEvent.click(screen.getByTestId('settings-export-run'))

    expect(calls('export_entries')).toContainEqual({ path: null, format: 'cxf' })
    expect(await screen.findByText(/rowel-export\.json/)).toBeInTheDocument()
  })

  it('stays disabled again once the acknowledgement is taken back', async () => {
    await exportPane()
    await userEvent.click(screen.getByTestId('settings-export-ack'))
    expect(screen.getByTestId('settings-export-run')).toBeEnabled()
    await userEvent.click(screen.getByTestId('settings-export-ack'))
    expect(screen.getByTestId('settings-export-run')).toBeDisabled()
    expect(calls('export_entries')).toEqual([])
  })

  it('moves the format choice with the arrow keys', async () => {
    await exportPane()
    screen.getByTestId('settings-export-format-bitwarden').focus()
    await userEvent.keyboard('{ArrowRight}')
    expect(screen.getByTestId('settings-export-format-cxf')).toHaveAttribute(
      'aria-checked',
      'true'
    )
    expect(screen.getByTestId('settings-export-format-cxf')).toHaveFocus()
  })

  it('counts what it will export on the button', async () => {
    await exportPane()
    expect(screen.getByTestId('settings-export-run')).toHaveTextContent('Export 0 items')
  })

  it('points a device move at the encrypted backup instead', async () => {
    await exportPane()
    await userEvent.click(screen.getByTestId('settings-export-backup-link'))
    expect(useUi.getState().settingsSection).toBe('sync')
  })
})

describe('Settings › language & region', () => {
  it('picks a language from the menu', async () => {
    await open()
    await go('language')

    const trigger = screen.getByTestId('settings-locale-trigger')
    expect(trigger).toHaveTextContent('English')
    expect(trigger).toHaveAttribute('aria-expanded', 'false')
    await userEvent.click(trigger)
    expect(trigger).toHaveAttribute('aria-expanded', 'true')
    expect(screen.getByTestId('settings-locale-en-US')).toHaveAttribute('aria-checked', 'true')

    await userEvent.click(screen.getByTestId('settings-locale-de-DE'))
    // Switching is async now: the de-DE catalogue is a dynamic import, fetched
    // on demand rather than bundled with the app.
    await waitFor(() => expect(i18n.resolvedLanguage).toBe('de-DE'))
    // Picking closes the menu, and the trigger names the language in its own tongue.
    expect(screen.queryByTestId('settings-locale-de-DE')).not.toBeInTheDocument()
    expect(screen.getByTestId('settings-locale-trigger')).toHaveTextContent('Deutsch')
  })

  // The micro labels are uppercased by CSS, and `text-transform` follows the
  // document language: under `lang="en"` Turkish "i" becomes "I" rather than
  // "İ", misspelling every label in the Turkish UI.
  it('tells the document what language it is in', async () => {
    await open()
    await go('language')
    await userEvent.click(screen.getByTestId('settings-locale-trigger'))
    await userEvent.click(screen.getByTestId('settings-locale-tr-TR'))
    await waitFor(() => expect(document.documentElement.lang).toBe('tr-TR'))
  })

  it('sets the theme from the preview cards', async () => {
    await open()
    await go('language')

    const cards = screen.getAllByRole('radio', { name: /^(System|Light|Dark)/ })
    expect(cards.map(card => card.dataset.testid)).toEqual([
      'settings-theme-system',
      'settings-theme-light',
      'settings-theme-dark'
    ])

    await userEvent.click(screen.getByTestId('settings-theme-dark'))

    expect(usePrefs.getState().theme).toBe('dark')
    expect(document.documentElement.getAttribute('data-theme')).toBe('dark')
    expect(screen.getByTestId('settings-theme-dark')).toHaveAttribute('aria-checked', 'true')
    expect(screen.getByTestId('settings-theme-light')).toHaveAttribute('aria-checked', 'false')
    expect(screen.getByTestId('settings-theme-system')).toHaveAttribute('aria-checked', 'false')
  })

  it('moves the theme with the arrow keys', async () => {
    await open()
    await go('language')
    await userEvent.click(screen.getByTestId('settings-theme-light'))

    await userEvent.keyboard('{ArrowRight}')
    expect(usePrefs.getState().theme).toBe('dark')
    expect(screen.getByTestId('settings-theme-dark')).toHaveFocus()
  })

  it('says which way System resolves right now', async () => {
    await open()
    await go('language')
    expect(screen.getByTestId('settings-theme-system')).toHaveTextContent(/now (dark|light)/)
  })

  it('picks an accent from the swatches and paints the root with it', async () => {
    await open()
    await go('language')

    expect(screen.getByRole('radiogroup', { name: 'Accent' })).toBeInTheDocument()
    expect(screen.getByTestId('settings-accent-ink')).toHaveAttribute('aria-checked', 'true')

    await userEvent.click(screen.getByTestId('settings-accent-petrol'))

    expect(usePrefs.getState().accent).toBe('petrol')
    expect(document.documentElement.getAttribute('data-accent')).toBe('petrol')
    expect(screen.getByTestId('settings-accent-petrol')).toHaveAttribute('aria-checked', 'true')
  })

  it('offers System as a theme', async () => {
    await open()
    await go('language')
    await userEvent.click(screen.getByTestId('settings-theme-system'))
    expect(usePrefs.getState().theme).toBe('system')
  })

  it('hands focus back to the date format field after a pick', async () => {
    await open()
    await go('language')

    const trigger = screen.getByTestId('settings-date-format-trigger')
    await userEvent.click(trigger)
    await userEvent.click(screen.getByTestId('settings-date-format-YYYY-MM-DD'))
    expect(trigger).toHaveFocus()
  })

  it('closes a menu on Tab and moves on from its field, either way', async () => {
    await open()
    await go('language')
    const language = screen.getByTestId('settings-locale-trigger')
    const format = screen.getByTestId('settings-date-format-trigger')

    await userEvent.click(language)
    await userEvent.tab()
    expect(screen.queryByTestId('dropdown-scrim')).not.toBeInTheDocument()
    expect(format).toHaveFocus()

    await userEvent.click(format)
    await userEvent.tab({ shift: true })
    expect(screen.queryByTestId('dropdown-scrim')).not.toBeInTheDocument()
    expect(language).toHaveFocus()
  })

  it('names the theme radiogroup and the date format menu after their rows', async () => {
    await open()
    await go('language')

    expect(screen.getByTestId('settings-date-format-trigger')).toHaveAccessibleName(
      /^Date format: /
    )
    expect(screen.getByRole('radiogroup', { name: 'Theme' })).toBeInTheDocument()
  })

  it('applies the date format to rendered timestamps', async () => {
    const iso = new Date(2024, 0, 2, 10, 30).toISOString()
    await open()
    await go('language')

    expect(dates(usePrefs.getState().dateFormat).dateTime(iso)).toMatch(/^01\/02\/2024/)

    const trigger = screen.getByTestId('settings-date-format-trigger')
    expect(trigger).toHaveTextContent('MM/DD/YYYY')

    await userEvent.click(trigger)
    await userEvent.click(screen.getByTestId('settings-date-format-DD.MM.YYYY'))
    expect(dates(usePrefs.getState().dateFormat).dateTime(iso)).toMatch(/^02\.01\.2024/)
    expect(trigger).toHaveTextContent('DD.MM.YYYY')

    await userEvent.click(trigger)
    await userEvent.click(screen.getByTestId('settings-date-format-YYYY-MM-DD'))
    expect(dates(usePrefs.getState().dateFormat).dateTime(iso)).toMatch(/^2024-01-02/)
  })
})

describe('Settings › date format', () => {
  // Not just stored: a date already on screen has to be re-read in the new
  // pattern. The format used to be read at call time by helpers nothing
  // subscribed to, so every rendered date kept the pattern it was first drawn
  // in. Two consumers, on purpose: the date field, and the entry footer's
  // "Created" stamp, which formats a timestamp through `shortDate` rather than
  // through a field.
  it('re-renders every shown date when the format changes', async () => {
    render(
      <>
        <Settings />
        <FieldsProvider
          value={{
            entry: { type: 'apikey', title: '', expiry_date: '2035-06-01' },
            set: null,
            attempted: false
          }}
        >
          <DateField name="expiry_date" label="Expires" />
        </FieldsProvider>
        {/* Midday UTC, so the local date is the 15th in every zone a test runs in. */}
        <Footer tags={[]} createdAt="2024-01-15T12:00:00.000Z" />
      </>
    )
    await userEvent.click(document.querySelector('.settings-button')!)
    await go('language')

    expect(screen.getByText('06/01/2035')).toBeInTheDocument()
    expect(screen.getByText('01/15/2024')).toBeInTheDocument()

    await userEvent.click(screen.getByTestId('settings-date-format-trigger'))
    await userEvent.click(screen.getByTestId('settings-date-format-DD.MM.YYYY'))
    expect(await screen.findByText('01.06.2035')).toBeInTheDocument()
    expect(screen.getByText('15.01.2024')).toBeInTheDocument()

    await userEvent.click(screen.getByTestId('settings-date-format-trigger'))
    await userEvent.click(screen.getByTestId('settings-date-format-YYYY-MM-DD'))
    expect(await screen.findByText('2035-06-01')).toBeInTheDocument()
    expect(screen.getByText('2024-01-15')).toBeInTheDocument()
    expect(usePrefs.getState().dateFormat).toBe('YYYY-MM-DD')
  })
})

// What the probe reports for a vault in `Rowel/Vaults/` — the same payload the
// first run's picker is built from, because it is the same probe.
const VAULT: SetupDriveFile = {
  id: 'drive-file-1',
  name: '9f3c1a2b4d5e6f708192a3b4c5d6e7f8.rowel',
  vaultId: '9f3c1a2b4d5e6f708192a3b4c5d6e7f8',
  size: 1_258_291,
  modifiedTime: '2024-01-01T00:00:00.000Z'
}

// Another install's primary in the same account: a second pack, older, so the
// probe lists it after the first.
const OTHER_VAULT: SetupDriveFile = {
  id: 'drive-file-2',
  name: '11223344556677889900112233445566.rowel',
  vaultId: '11223344556677889900112233445566',
  size: 64_512,
  modifiedTime: '2023-06-02T00:00:00.000Z'
}

// Every device on an account is meant to hold every vault in it. A sync run
// reports the ones this device lacks (`workspaces:remote`), and the section
// offers them with the account the open workspace already has.
describe('Settings › workspaces › vaults in the account', () => {
  it('draws nothing while every vault in the account is here', async () => {
    await open()
    await go('workspaces')
    expect(screen.queryByTestId('workspace-remote-row')).not.toBeInTheDocument()
  })

  it('restores a vault the account holds and this device does not, with no sign-in', async () => {
    await open()
    await act(async () => setRemoteVaults([VAULT, OTHER_VAULT]))
    await go('workspaces')

    expect(screen.getByTestId('workspace-remote-row')).toBeInTheDocument()
    // Two vaults is a choice; the newest is picked to start with.
    expect(screen.getByTestId('remote-vault-drive-file-1')).toHaveAttribute('aria-checked', 'true')
    await userEvent.click(screen.getByTestId('remote-vault-drive-file-2'))
    await userEvent.type(screen.getByTestId('workspace-remote-name'), 'Personal')
    await userEvent.type(screen.getByTestId('workspace-remote-password'), 'other-device-pass')
    await act(async () => {
      await userEvent.click(screen.getByTestId('workspace-remote-submit'))
    })

    expect(calls('workspace_restore_from_account')).toEqual([
      { name: 'Personal', password: 'other-device-pass', fileId: 'drive-file-2' }
    ])
    // The open workspace's own tokens do the work: nothing is signed into.
    expect(calls('workspace_drive_connect')).toHaveLength(0)
    expect(calls('sync_connect')).toHaveLength(0)
    // It lands like every restore: active, unlocked, re-probed, first sync run.
    await waitFor(() => expect(useApp.getState().flow).toBe('main'))
    expect(calls('sync_now')).toHaveLength(1)
    // The list was the previous workspace's account's; the new one's run says again.
    expect(useApp.getState().remoteVaults).toEqual([])
  })

  // One vault is shown, not offered — and its list sits beside the probe's
  // picker without the two answering to the same ids.
  it("names the account's only missing vault without asking which", async () => {
    await open()
    await act(async () => setRemoteVaults([VAULT]))
    await go('workspaces')

    expect(screen.getByTestId('remote-found-file')).toBeInTheDocument()
    expect(screen.queryByTestId('remote-vault-drive-file-1')).not.toBeInTheDocument()
    expect(screen.queryByTestId('drive-found-file')).not.toBeInTheDocument()
  })

  it('holds Settings on the section while the restore runs', async () => {
    mockCommandOnce('workspace_restore_from_account', () => new Promise(() => {}))
    await open()
    await act(async () => setRemoteVaults([VAULT]))
    await go('workspaces')

    await userEvent.type(screen.getByTestId('workspace-remote-name'), 'Personal')
    await userEvent.type(screen.getByTestId('workspace-remote-password'), 'pass')
    await userEvent.click(screen.getByTestId('workspace-remote-submit'))

    expect(screen.getByTestId('workspace-remote-submit')).toHaveTextContent('Restoring…')
    expect(screen.getByTestId('modal-close')).toBeDisabled()
    expect(screen.getByTestId('settings-nav-sync')).toBeDisabled()
  })

  it('hands a wrong password back to the form', async () => {
    mockCommandOnce('workspace_restore_from_account', () =>
      Promise.reject({ kind: 'invalidPassword', message: 'wrong' })
    )
    await open()
    await act(async () => setRemoteVaults([VAULT]))
    await go('workspaces')

    await userEvent.type(screen.getByTestId('workspace-remote-name'), 'Personal')
    await userEvent.type(screen.getByTestId('workspace-remote-password'), 'wrong')
    await userEvent.click(screen.getByTestId('workspace-remote-submit'))

    expect(
      await screen.findByText(
        "That isn't the password this was sealed with. Try the one you use on your other devices."
      )
    ).toBeInTheDocument()
    expect(screen.getByTestId('workspace-remote-submit')).toHaveTextContent('Restore')
    expect(useUi.getState().settingsLocked).toBe(false)
  })
})

describe('Settings › workspaces › list', () => {
  const two = () =>
    seedApp({
      workspaces: [
        { id: 'default', name: null },
        { id: 'w2', name: 'Work', vaultId: 'cafe', synced: true, itemCount: 12 }
      ],
      activeWorkspace: 'default'
    })

  it('marks the open workspace and offers a switch to the others', async () => {
    two()
    await open()
    await go('workspaces')

    expect(screen.getByText('On this device · 2')).toBeInTheDocument()
    expect(screen.getByTestId('workspace-row-default')).toHaveTextContent('Open now')
    // Which one is primary is said outright: a rename would otherwise hide it.
    expect(screen.getByTestId('workspace-primary-default')).toBeInTheDocument()
    expect(screen.queryByTestId('workspace-primary-w2')).not.toBeInTheDocument()
    expect(screen.queryByTestId('workspace-switch-default')).not.toBeInTheDocument()
    expect(screen.getByTestId('workspace-row-w2')).not.toHaveTextContent('Open now')
    expect(screen.getByTestId('workspace-row-w2')).toHaveTextContent('12 items · Google Drive')

    await userEvent.click(screen.getByTestId('workspace-switch-w2'))
    expect(calls('workspace_select')).toEqual([{ id: 'w2' }])
  })

  it('keeps Edit and Delete behind the row’s menu', async () => {
    two()
    await open()
    await go('workspaces')

    expect(screen.queryByTestId('workspace-edit-w2')).not.toBeInTheDocument()
    expect(screen.queryByTestId('workspace-delete-w2')).not.toBeInTheDocument()

    await openMenu('w2')
    expect(screen.getByTestId('workspace-menu-w2')).toHaveAttribute('aria-expanded', 'true')
    expect(screen.getByTestId('workspace-edit-w2')).toHaveTextContent('Edit…')
    expect(screen.getByTestId('workspace-delete-w2')).toBeEnabled()
  })
})

describe('Settings › workspaces › edit', () => {
  const edit = async (id: string) => {
    const status = {
      workspaces: [
        { id: 'default', name: null },
        { id: 'w2', name: 'Work', vaultId: 'cafe', synced: true, itemCount: 12 }
      ],
      activeWorkspace: 'default'
    }
    seedApp(status)
    // The probe a save re-reads the list through answers with the same two
    // workspaces, as it would; the bare default would make the one being
    // edited vanish and the page step out.
    mockCommand('app_status', () => ({ ...appStatusDefault(), ...status }))
    await open()
    await go('workspaces')
    await openMenu(id)
    await userEvent.click(screen.getByTestId(`workspace-edit-${id}`))
  }

  const hint = () => screen.getByTestId('settings-subpage-hint')
  const tile = () => screen.getByTestId('workspace-edit-preview').querySelector('[aria-hidden]')

  it('opens on the workspace as it is, with nothing to save yet', async () => {
    await edit('w2')

    expect(screen.getByRole('heading', { name: 'Edit workspace' })).toBeInTheDocument()
    expect(screen.getByTestId('workspace-edit-name')).toHaveValue('Work')
    expect(screen.getByTestId('workspace-edit-preview')).toHaveTextContent(
      '12 items · Google Drive'
    )
    // No colour picked yet: the tile is still its hashed hue.
    expect(tile()).toHaveClass('monogram')
    expect(screen.getByTestId('workspace-edit-save')).toBeDisabled()
    expect(hint()).toHaveTextContent('No changes yet')
  })

  it('renames a workspace and steps back to the list', async () => {
    await edit('w2')
    const input = screen.getByTestId('workspace-edit-name')
    await userEvent.clear(input)
    await userEvent.type(input, 'Clients')
    expect(hint()).toHaveTextContent('Ready')
    await userEvent.click(screen.getByTestId('workspace-edit-save'))

    expect(calls('workspace_rename')).toEqual([{ id: 'w2', name: 'Clients' }])
    expect(calls('workspace_set_color')).toHaveLength(0)
    await waitFor(() =>
      expect(screen.queryByTestId('workspace-edit-name')).not.toBeInTheDocument()
    )
    expect(screen.getByTestId('workspace-new-row')).toBeInTheDocument()
  })

  it('recolours a workspace without renaming it', async () => {
    await edit('w2')
    await userEvent.click(screen.getByTestId('workspace-edit-color-rose'))

    expect(screen.getByTestId('workspace-edit-color-rose')).toHaveAttribute('aria-checked', 'true')
    expect(tile()).toHaveClass('bg-ws-rose')
    await userEvent.click(screen.getByTestId('workspace-edit-save'))

    expect(calls('workspace_set_color')).toEqual([{ id: 'w2', color: 'rose' }])
    expect(calls('workspace_rename')).toHaveLength(0)
    await waitFor(() =>
      expect(screen.queryByTestId('workspace-edit-name')).not.toBeInTheDocument()
    )
  })

  it('refuses another workspace’s name, whatever its case', async () => {
    await edit('w2')
    const input = screen.getByTestId('workspace-edit-name')
    await userEvent.clear(input)
    await userEvent.type(input, 'personal')

    expect(screen.getByTestId('workspace-edit-duplicate')).toHaveTextContent(
      'You already have a workspace with this name.'
    )
    expect(screen.getByTestId('workspace-edit-save')).toBeDisabled()
    await userEvent.keyboard('{Enter}')
    expect(calls('workspace_rename')).toHaveLength(0)
  })

  it('shows what went wrong and stays open', async () => {
    mockCommandOnce('workspace_rename', () =>
      Promise.reject({ kind: 'other', message: 'registry is read-only' })
    )
    await edit('w2')
    await userEvent.type(screen.getByTestId('workspace-edit-name'), ' Ltd')
    await userEvent.click(screen.getByTestId('workspace-edit-save'))

    expect(await screen.findByTestId('workspace-edit-error')).toHaveTextContent(
      'registry is read-only'
    )
    expect(screen.getByTestId('workspace-edit-name')).toBeEnabled()
  })

  // A rename that landed before the colour failed is not undone by the
  // failure: the list is re-read so it shows, and a retry sends only the part
  // still unsaved rather than renaming twice.
  it('keeps a rename that landed when the colour after it fails', async () => {
    mockCommandOnce('workspace_set_color', () =>
      Promise.reject({ kind: 'other', message: 'registry is read-only' })
    )
    await edit('w2')
    // What the re-read finds once the rename has landed.
    mockCommand('app_status', () => ({
      ...appStatusDefault(),
      workspaces: [
        { id: 'default', name: null },
        { id: 'w2', name: 'Work Ltd', vaultId: 'cafe', synced: true, itemCount: 12 }
      ]
    }))
    await userEvent.type(screen.getByTestId('workspace-edit-name'), ' Ltd')
    await userEvent.click(screen.getByTestId('workspace-edit-color-rose'))
    await userEvent.click(screen.getByTestId('workspace-edit-save'))

    expect(await screen.findByTestId('workspace-edit-error')).toHaveTextContent(
      'registry is read-only'
    )
    expect(calls('workspace_rename')).toEqual([{ id: 'w2', name: 'Work Ltd' }])
    expect(screen.getByTestId('workspace-edit-name')).toHaveValue('Work Ltd')

    await userEvent.click(screen.getByTestId('workspace-edit-save'))
    await waitFor(() => expect(calls('workspace_set_color')).toHaveLength(2))
    expect(calls('workspace_rename')).toHaveLength(1)
  })

  it('lets the edit go with Escape, leaving Settings open', async () => {
    await edit('default')
    await userEvent.type(screen.getByTestId('workspace-edit-name'), ' vault')
    await userEvent.keyboard('{Escape}')

    expect(screen.queryByTestId('workspace-edit-name')).not.toBeInTheDocument()
    expect(screen.getByTestId('workspace-new-row')).toBeInTheDocument()
    expect(useUi.getState().settings).toBe(true)
    expect(calls('workspace_rename')).toHaveLength(0)
    expect(calls('workspace_set_color')).toHaveLength(0)
  })
})

describe('Settings › workspaces › new', () => {
  const openNew = async () => {
    await open()
    await go('workspaces')
    await userEvent.click(screen.getByTestId('workspace-new-row'))
  }

  const create = () => screen.getByTestId('workspace-create')
  const hint = () => screen.getByTestId('settings-subpage-hint')
  const tile = () => screen.getByTestId('workspace-new-preview').querySelector('[aria-hidden]')

  it('opens the sub-page from the list’s last row and creates one', async () => {
    await openNew()

    expect(screen.getByRole('heading', { name: 'New workspace' })).toBeInTheDocument()
    await userEvent.type(screen.getByTestId('workspace-new-name'), 'Family')
    await userEvent.type(screen.getByTestId('workspace-new-password'), 'master-pass')
    expect(create()).toBeEnabled()
    expect(hint()).toHaveTextContent('Ready')
    await act(async () => {
      await userEvent.click(create())
    })

    expect(calls('workspace_create')).toEqual([
      { name: 'Family', password: 'master-pass', color: 'indigo' }
    ])
  })

  it('keeps Create disabled until both the name and the password are in', async () => {
    await openNew()
    const name = screen.getByTestId('workspace-new-name')

    expect(create()).toBeDisabled()
    expect(hint()).toHaveTextContent('Name and master password required')
    await userEvent.type(name, 'Family')
    expect(create()).toBeDisabled()
    await userEvent.clear(name)
    await userEvent.type(screen.getByTestId('workspace-new-password'), 'master-pass')
    expect(create()).toBeDisabled()
    await userEvent.type(name, 'Family')
    expect(create()).toBeEnabled()
  })

  it('refuses a name another workspace already has', async () => {
    seedApp({ workspaces: [{ id: 'default', name: 'Work' }], activeWorkspace: 'default' })
    await openNew()
    await userEvent.type(screen.getByTestId('workspace-new-name'), ' work ')
    await userEvent.type(screen.getByTestId('workspace-new-password'), 'master-pass')

    expect(screen.getByTestId('workspace-new-duplicate')).toHaveTextContent(
      'You already have a workspace with this name.'
    )
    expect(create()).toBeDisabled()
    await userEvent.keyboard('{Enter}')
    expect(calls('workspace_create')).toHaveLength(0)
  })

  it('starts on the first colour no workspace has, and redraws the tile as one is picked', async () => {
    seedApp({
      workspaces: [
        { id: 'default', name: null, color: 'indigo' },
        { id: 'w2', name: 'Work', color: 'violet' }
      ],
      activeWorkspace: 'default'
    })
    await openNew()

    expect(screen.getByTestId('workspace-new-color-green')).toHaveAttribute('aria-checked', 'true')
    expect(tile()).toHaveClass('bg-ws-green')
    await userEvent.click(screen.getByTestId('workspace-new-color-teal'))
    expect(tile()).toHaveClass('bg-ws-teal')
    expect(tile()).not.toHaveClass('bg-ws-green')
  })

  it('says a wrong master password in the words the unlock uses', async () => {
    mockCommandOnce('workspace_create', () =>
      Promise.reject({ kind: 'invalidPassword', message: 'invalid master password' })
    )
    await openNew()
    await userEvent.type(screen.getByTestId('workspace-new-name'), 'Family')
    await userEvent.type(screen.getByTestId('workspace-new-password'), 'wrong{Enter}')

    // Named: the password wanted is the primary's, which need not be the open
    // workspace's.
    expect(await screen.findByTestId('workspace-new-error')).toHaveTextContent(
      'That is not the master password of Personal.'
    )
    expect(calls('workspace_create')).toHaveLength(1)
  })

  it('goes back to the list', async () => {
    await openNew()
    await userEvent.click(screen.getByTestId('settings-subpage-back'))

    expect(screen.queryByTestId('workspace-new-name')).not.toBeInTheDocument()
    expect(screen.getByTestId('workspace-new-row')).toBeInTheDocument()
  })
})

// Local only: the vault leaves this device and whatever it has on Drive stays,
// so the confirmation asks for two proofs — the workspace's own master password
// and its label typed out — before anything is removed.
describe('Settings › workspaces › delete', () => {
  const two = () =>
    seedApp({
      workspaces: [
        { id: 'default', name: null },
        { id: 'w2', name: 'Work' }
      ],
      activeWorkspace: 'default'
    })

  const openDelete = async (id: string) => {
    two()
    await open()
    await go('workspaces')
    await openMenu(id)
    await userEvent.click(screen.getByTestId(`workspace-delete-${id}`))
    return screen.getByTestId('workspace-delete-page')
  }

  it('opens the confirmation on the workspace that was chosen', async () => {
    await openDelete('w2')

    // A page under Workspaces, like Edit, not a dialog over the list: the
    // header names the step and the preview says which workspace.
    expect(screen.getByTestId('workspace-delete-page')).toBeInTheDocument()
    expect(screen.getByRole('heading', { name: 'Delete workspace' })).toBeInTheDocument()
    expect(screen.getByTestId('workspace-delete-preview')).toHaveTextContent('Work')
    expect(screen.queryByTestId('workspace-new-row')).not.toBeInTheDocument()
    expect(
      screen.getByText(/A copy on Google Drive is left where it is/)
    ).toBeInTheDocument()
  })

  it('holds Delete until the label is typed back', async () => {
    await openDelete('w2')

    const submit = screen.getByTestId('workspace-delete-submit')
    expect(submit).toBeDisabled()

    await userEvent.type(screen.getByTestId('workspace-delete-password'), 'work-pass')
    expect(submit).toBeDisabled()

    await userEvent.type(screen.getByTestId('workspace-delete-confirm'), 'Wor')
    expect(submit).toBeDisabled()

    await userEvent.type(screen.getByTestId('workspace-delete-confirm'), 'k')
    expect(submit).toBeEnabled()
  })

  it('deletes with the id and that workspace’s password', async () => {
    mockCommand('workspace_delete', () => undefined)
    await openDelete('w2')

    await userEvent.type(screen.getByTestId('workspace-delete-password'), 'work-pass')
    await userEvent.type(screen.getByTestId('workspace-delete-confirm'), 'Work')
    await act(async () => {
      await userEvent.click(screen.getByTestId('workspace-delete-submit'))
    })

    expect(calls('workspace_delete')).toEqual([
      { id: 'w2', password: 'work-pass', everywhere: false }
    ])
    // The probe is what carries the list, so a delete ends by re-reading it.
    expect(calls('app_status').length).toBeGreaterThan(0)
    await waitFor(() =>
      expect(screen.queryByTestId('workspace-delete-page')).not.toBeInTheDocument()
    )
  })

  // The open workspace may go too: the backend ends its session and announces
  // the lock, which is what lands on the survivor's lock screen.
  it('offers the delete on the current workspace as well', async () => {
    await openDelete('default')
    expect(screen.getByTestId('workspace-delete-preview')).toHaveTextContent('Personal')
  })

  it('steps back to the list on Escape without deleting', async () => {
    await openDelete('w2')
    await userEvent.keyboard('{Escape}')

    expect(screen.queryByTestId('workspace-delete-page')).not.toBeInTheDocument()
    expect(screen.getByTestId('workspace-row-w2')).toBeInTheDocument()
    expect(calls('workspace_delete')).toHaveLength(0)
    expect(useUi.getState().settings).toBe(true)
  })

  it('holds every way out while a deletion is in progress', async () => {
    let finish!: () => void
    mockCommandOnce(
      'workspace_delete',
      () => new Promise<void>(resolve => (finish = resolve))
    )
    await openDelete('w2')
    await userEvent.type(screen.getByTestId('workspace-delete-password'), 'work-pass')
    await userEvent.type(screen.getByTestId('workspace-delete-confirm'), 'Work')
    await userEvent.click(screen.getByTestId('workspace-delete-submit'))

    const cancel = screen.getByTestId('settings-subpage-cancel')
    const back = screen.getByTestId('settings-subpage-back')
    const modalClose = screen.getByTestId('modal-close')
    const security = screen.getByTestId('settings-nav-security')
    expect(cancel).toBeDisabled()
    expect(back).toBeDisabled()
    expect(modalClose).toBeDisabled()
    expect(security).toBeDisabled()

    // The lock is acquired before React redraws the disabled controls too: each
    // route still refuses if its handler was captured by the preceding render.
    for (const control of [cancel, back, modalClose, security]) {
      control.removeAttribute('disabled')
      await userEvent.click(control)
    }
    await userEvent.keyboard('{Escape}')
    expect(screen.getByTestId('workspace-delete-page')).toBeInTheDocument()
    expect(useUi.getState().settings).toBe(true)

    await act(async () => finish())
    await waitFor(() =>
      expect(screen.queryByTestId('workspace-delete-page')).not.toBeInTheDocument()
    )
    expect(useUi.getState().settingsLocked).toBe(false)
  })

  it('blames the password only when the backend does', async () => {
    mockCommandOnce('workspace_delete', () =>
      Promise.reject({ kind: 'invalidPassword', message: 'invalid master password' })
    )
    await openDelete('w2')

    await userEvent.type(screen.getByTestId('workspace-delete-password'), 'wrong')
    await userEvent.type(screen.getByTestId('workspace-delete-confirm'), 'Work')
    await userEvent.click(screen.getByTestId('workspace-delete-submit'))

    expect(await screen.findByTestId('workspace-delete-error')).toHaveTextContent(
      'That is not this workspace’s master password'
    )
    expect(screen.getByTestId('workspace-delete-page')).toBeInTheDocument()
  })

  // The delete proves a password on demand and takes no session, so it runs
  // under the target workspace's failed-attempt backoff exactly as an unlock
  // does. Once that escalates the user is told how long, not just "wrong".
  it('reports the backoff rather than the password once attempts escalate', async () => {
    mockCommandOnce('workspace_delete', () =>
      Promise.reject({ kind: 'tooManyAttempts', message: 'too many attempts', retryAfterSecs: 8 })
    )
    await openDelete('w2')

    await userEvent.type(screen.getByTestId('workspace-delete-password'), 'wrong')
    await userEvent.type(screen.getByTestId('workspace-delete-confirm'), 'Work')
    await userEvent.click(screen.getByTestId('workspace-delete-submit'))

    expect(await screen.findByTestId('workspace-delete-error')).toHaveTextContent(
      'Too many failed attempts. Try again in 8s'
    )
    expect(screen.getByTestId('workspace-delete-page')).toBeInTheDocument()
  })

  // A device always has a vault to open, and the control says so rather than
  // letting the user find out from a rejection.
  it('withdraws the delete when there is only one workspace', async () => {
    await open()
    await go('workspaces')

    expect(screen.getByTestId('workspace-delete-last')).toBeInTheDocument()
    await openMenu('default')
    expect(screen.getByTestId('workspace-delete-default')).toBeDisabled()
    await userEvent.click(screen.getByTestId('workspace-delete-default'))
    expect(screen.queryByTestId('workspace-delete-page')).not.toBeInTheDocument()
  })

  // A workspace with a vault id has a pack in the account, so it is offered the
  // wider scope — and the wider scope is what the backend is told.
  it('deletes everywhere when that scope is chosen', async () => {
    mockCommand('workspace_delete', () => undefined)
    seedApp({
      workspaces: [
        { id: 'default', name: null },
        { id: 'w2', name: 'Work', vaultId: 'cafe', synced: true }
      ],
      activeWorkspace: 'default'
    })
    await open()
    await go('workspaces')
    await openMenu('w2')
    await userEvent.click(screen.getByTestId('workspace-delete-w2'))

    await userEvent.click(screen.getByTestId('workspace-delete-scope-everywhere'))
    expect(screen.getByText(/deletes its copy in Google Drive/)).toBeInTheDocument()

    await userEvent.type(screen.getByTestId('workspace-delete-password'), 'work-pass')
    await userEvent.type(screen.getByTestId('workspace-delete-confirm'), 'Work')
    await act(async () => {
      await userEvent.click(screen.getByTestId('workspace-delete-submit'))
    })

    expect(calls('workspace_delete')).toEqual([
      { id: 'w2', password: 'work-pass', everywhere: true }
    ])
  })

  // A vault id alone is a pack on Drive, not a connection: a workspace that was
  // disconnected and then locked keeps its id and must read as local — in the
  // list, and in the delete it is offered.
  it('reads a disconnected workspace as local despite its vault id', async () => {
    seedApp({
      workspaces: [
        { id: 'default', name: null },
        { id: 'w2', name: 'Work', vaultId: 'cafe', synced: false, itemCount: 3 }
      ],
      activeWorkspace: 'default'
    })
    await open()
    await go('workspaces')

    expect(screen.getByTestId('workspace-row-w2')).toHaveTextContent('3 items · This device')

    await openMenu('w2')
    await userEvent.click(screen.getByTestId('workspace-delete-w2'))
    expect(
      screen.queryByTestId('workspace-delete-scope-everywhere')
    ).not.toBeInTheDocument()
  })

  // No vault id and no connection: there is nothing in an account to delete, so
  // the choice is not put on screen at all.
  it('offers no scope for a workspace that does not sync', async () => {
    await openDelete('w2')

    expect(
      screen.queryByTestId('workspace-delete-scope-everywhere')
    ).not.toBeInTheDocument()
    expect(
      screen.getByText(/A copy on Google Drive is left where it is/)
    ).toBeInTheDocument()
  })

  // Another device ran the delete: this one's runs stop with the typed error,
  // and the row offers the local delete that finishes the job here.
  it('offers the local delete when the vault was deleted on another device', async () => {
    two()
    await open()
    await go('workspaces')
    await act(async () =>
      report({
        configured: true,
        error: 'this vault was deleted from Google Drive on another device',
        errorKind: 'vaultDeletedRemotely'
      })
    )

    expect(screen.getByTestId('workspace-deleted-remotely')).toHaveTextContent(
      'This vault was deleted from Google Drive on another device.'
    )
    await userEvent.click(screen.getByTestId('workspace-delete-stranded'))

    expect(screen.getByTestId('workspace-delete-preview')).toHaveTextContent('Personal')
    expect(
      screen.queryByTestId('workspace-delete-scope-everywhere')
    ).not.toBeInTheDocument()
  })
})

describe('Settings › workspaces › restore from Drive', () => {
  // Consent, then the probe's answer — which arrives as an event, never through
  // the connect's promise.
  const connect = async (files: SetupDriveFile[]) => {
    await open()
    await go('workspaces')
    await userEvent.click(screen.getByTestId('workspace-restore-connect'))
    await act(async () => setupDriveProbed(files))
  }

  it('restores the vault the user picked as a workspace of its own', async () => {
    await connect([VAULT, OTHER_VAULT])

    expect(calls('workspace_drive_connect')).toHaveLength(1)
    // Two vaults is a choice, so both are offered rather than one picked.
    expect(screen.getByTestId('drive-vault-drive-file-1')).toBeInTheDocument()
    await userEvent.click(screen.getByTestId('drive-vault-drive-file-2'))

    await userEvent.type(screen.getByTestId('workspace-restore-name'), 'Work')
    await userEvent.type(screen.getByTestId('workspace-restore-password'), 'other-device-pass')
    await act(async () => {
      await userEvent.click(screen.getByTestId('workspace-restore-submit'))
    })

    expect(calls('workspace_restore_from_drive')).toEqual([
      { name: 'Work', password: 'other-device-pass', fileId: 'drive-file-2' }
    ])
    // It arrives active and unlocked, so the app lands in main and re-probes —
    // which is what brings the new workspace on screen.
    await waitFor(() => expect(useApp.getState().flow).toBe('main'))
    expect(calls('app_status').length).toBeGreaterThan(0)
    // Connected on arrival, so the first sync runs exactly as it does after the
    // first run's restore. (`sync.configured` is not asserted: the re-probe
    // that follows overwrites it from the fake backend's status, which knows
    // nothing about the workspace that was just restored.)
    expect(calls('sync_now')).toHaveLength(1)
  })

  // The vault already has a name, given wherever it was made, and it travels in
  // the pack — so leaving the field blank is a restore, not a validation error.
  it('restores without a name and lets the pack supply one', async () => {
    await connect([VAULT])

    await userEvent.type(screen.getByTestId('workspace-restore-password'), 'other-device-pass')
    await act(async () => {
      await userEvent.click(screen.getByTestId('workspace-restore-submit'))
    })

    expect(calls('workspace_restore_from_drive')).toEqual([
      { name: '', password: 'other-device-pass', fileId: VAULT.id }
    ])
  })

  // One vault in the account is not a choice: it is shown, not offered.
  it("names the account's only vault without asking which", async () => {
    await connect([VAULT])

    expect(screen.getByTestId('drive-found-file')).toBeInTheDocument()
    expect(screen.queryByTestId('drive-vault-drive-file-1')).not.toBeInTheDocument()
  })

  // Backing out has to reach the backend: tokens left pending would otherwise
  // be adopted by whatever create or restore came next.
  it('forgets the connected account when the dialog is cancelled', async () => {
    await connect([VAULT])

    await userEvent.click(screen.getByTestId('workspace-restore-cancel'))

    expect(calls('setup_drive_disconnect')).toHaveLength(1)
    expect(useApp.getState().setupDrive.status).toBe('idle')
    expect(screen.getByTestId('workspace-restore-connect')).toBeInTheDocument()
  })

  // Once the restore is running the backend holds the account for its length
  // and would refuse to drop it, so nothing on screen may offer to: a Cancel
  // that did nothing would read as a cancellation that happened.
  it('withdraws Cancel and Switch account while the restore runs', async () => {
    mockCommandOnce('workspace_restore_from_drive', () => new Promise(() => {}))
    await connect([VAULT])

    await userEvent.type(screen.getByTestId('workspace-restore-name'), 'Work')
    await userEvent.type(screen.getByTestId('workspace-restore-password'), 'pass')
    await userEvent.click(screen.getByTestId('workspace-restore-submit'))

    expect(useApp.getState().setupDrive.status).toBe('restoring')
    expect(screen.queryByTestId('workspace-restore-cancel')).not.toBeInTheDocument()
    expect(screen.queryByTestId('workspace-restore-switch-account')).not.toBeInTheDocument()
    expect(screen.getByTestId('workspace-restore-submit')).toHaveTextContent('Restoring…')
    expect(calls('setup_drive_disconnect')).toHaveLength(0)
  })

  // Leaving Settings would unmount the row and run the same refused disconnect,
  // then the restore would finish and switch workspaces behind the user's back.
  // So Settings stays shut on its section — from the X, from the nav, and from
  // anything else that reaches the store — until the restore lands.
  it('holds Settings on the section while the restore runs', async () => {
    mockCommandOnce('workspace_restore_from_drive', () => new Promise(() => {}))
    await connect([VAULT])

    await userEvent.type(screen.getByTestId('workspace-restore-name'), 'Work')
    await userEvent.type(screen.getByTestId('workspace-restore-password'), 'pass')
    await userEvent.click(screen.getByTestId('workspace-restore-submit'))

    expect(screen.getByTestId('modal-close')).toBeDisabled()
    expect(screen.getByTestId('settings-nav-sync')).toBeDisabled()
    // The store refuses too, whoever asks.
    closeSettings()
    setSettingsSection('sync')
    expect(useUi.getState().settings).toBe(true)
    expect(useUi.getState().settingsSection).toBe('workspaces')
  })

  // A failed restore gives Settings back with the form.
  it('releases Settings when the restore fails', async () => {
    mockCommandOnce('workspace_restore_from_drive', () =>
      Promise.reject({ kind: 'invalidPassword', message: 'invalid master password' })
    )
    await connect([VAULT])

    await userEvent.type(screen.getByTestId('workspace-restore-name'), 'Work')
    await userEvent.type(screen.getByTestId('workspace-restore-password'), 'wrong')
    await userEvent.click(screen.getByTestId('workspace-restore-submit'))

    await waitFor(() => expect(useUi.getState().settingsLocked).toBe(false))
    expect(screen.getByTestId('modal-close')).toBeEnabled()
    await userEvent.click(screen.getByTestId('modal-close'))
    expect(useUi.getState().settings).toBe(false)
  })

  // A failed restore hands the form back: the list and the pick are still
  // there, and so are Cancel and Switch account.
  it('hands the form back when the restore fails', async () => {
    mockCommandOnce('workspace_restore_from_drive', () =>
      Promise.reject({ kind: 'invalidPassword', message: 'invalid master password' })
    )
    await connect([VAULT])

    await userEvent.type(screen.getByTestId('workspace-restore-name'), 'Work')
    await userEvent.type(screen.getByTestId('workspace-restore-password'), 'wrong')
    await userEvent.click(screen.getByTestId('workspace-restore-submit'))

    await waitFor(() => expect(useApp.getState().setupDrive.status).toBe('found'))
    expect(screen.getByTestId('workspace-restore-cancel')).toBeInTheDocument()
    expect(screen.getByTestId('workspace-restore-switch-account')).toBeInTheDocument()
  })

  it('blames the password only when the backend does', async () => {
    mockCommandOnce('workspace_restore_from_drive', () =>
      Promise.reject({ kind: 'invalidPassword', message: 'invalid master password' })
    )
    await connect([VAULT])

    await userEvent.type(screen.getByTestId('workspace-restore-name'), 'Work')
    await userEvent.type(screen.getByTestId('workspace-restore-password'), 'wrong')
    await userEvent.click(screen.getByTestId('workspace-restore-submit'))

    expect(
      await screen.findByText(
        "That isn't the password this was sealed with. Try the one you use on your other devices."
      )
    ).toBeInTheDocument()
    expect(useApp.getState().flow).not.toBe('main')
  })

  // The pack is already a workspace here — the open one, or a locked one the
  // registry remembers; the backend says which is not the point, only that it is.
  it('says so when the chosen vault is already a workspace on this device', async () => {
    mockCommandOnce('workspace_restore_from_drive', () =>
      Promise.reject({
        kind: 'vaultAlreadyOpen',
        message: 'this vault is already a workspace on this device'
      })
    )
    await connect([VAULT])

    await userEvent.type(screen.getByTestId('workspace-restore-name'), 'Copy')
    await userEvent.type(screen.getByTestId('workspace-restore-password'), 'pass')
    await userEvent.click(screen.getByTestId('workspace-restore-submit'))

    expect(
      await screen.findByText('This vault is already a workspace on this device')
    ).toBeInTheDocument()
  })

  it('reports a consent that never came back', async () => {
    await open()
    await go('workspaces')
    await userEvent.click(screen.getByTestId('workspace-restore-connect'))
    expect(screen.getByTestId('workspace-restore-waiting')).toBeInTheDocument()

    await act(async () => setupDriveFailed('access_denied'))

    expect(await screen.findByTestId('workspace-restore-error')).toHaveTextContent(
      'access_denied'
    )
    expect(calls('workspace_restore_from_drive')).toHaveLength(0)
  })
})

describe('Settings sub-pages', () => {
  const openSubpage = async () => {
    await open()
    await go('workspaces')
    await userEvent.click(screen.getByTestId('workspace-new-row'))
  }

  const onSection = () => {
    expect(screen.getByRole('heading', { name: 'Workspaces' })).toBeInTheDocument()
    expect(screen.queryByTestId('settings-subpage-back')).not.toBeInTheDocument()
    expect(screen.getByTestId('workspace-new-row')).toBeInTheDocument()
  }

  it('heads the sub-page in the section’s place and comes back from it', async () => {
    await openSubpage()

    // The title takes the section's line: one heading, nothing stacked above
    // it, so the header keeps its height on the way in and out.
    expect(screen.getAllByRole('heading', { level: 1 })).toHaveLength(1)
    expect(screen.getByRole('heading', { name: 'New workspace' })).toBeInTheDocument()
    expect(
      screen.getByText('Its own encrypted database, unlocked alongside your others.')
    ).toBeInTheDocument()
    // It replaces the section's body rather than stacking on it.
    expect(screen.queryByTestId('workspace-new-row')).not.toBeInTheDocument()
    expect(screen.getByTestId('settings-subpage-back')).toHaveAccessibleName('Back')

    await userEvent.click(screen.getByTestId('settings-subpage-back'))
    onSection()
  })

  it('steps back out on Escape, leaving Settings open', async () => {
    await openSubpage()

    await userEvent.keyboard('{Escape}')
    onSection()
    expect(useUi.getState().settings).toBe(true)
  })

  it('drops the sub-page when another section is picked', async () => {
    await openSubpage()

    await go('security')
    expect(screen.getByRole('heading', { name: 'Security' })).toBeInTheDocument()
    expect(screen.queryByTestId('settings-subpage-back')).not.toBeInTheDocument()

    // Coming back lands on the section, not on the sub-page left behind.
    await go('workspaces')
    onSection()
  })

  it('goes back from the footer’s Cancel', async () => {
    await openSubpage()

    await userEvent.click(screen.getByTestId('settings-subpage-cancel'))
    onSection()
  })

  it('holds the sub-page while Settings is locked', async () => {
    await openSubpage()

    act(() => lockSettings(true))
    expect(screen.getByTestId('settings-subpage-back')).toBeDisabled()
    await userEvent.keyboard('{Escape}')
    await userEvent.click(screen.getByTestId('settings-subpage-cancel'))
    await userEvent.click(screen.getByTestId('settings-nav-workspaces'))
    expect(screen.getByRole('heading', { name: 'New workspace' })).toBeInTheDocument()
    expect(useUi.getState().settings).toBe(true)
  })
})
