import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import { act, render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import Settings from '@/components/Main/Sidebar/Settings'
import i18n, { changeLocale } from '@/i18n'
import { dates } from '@/utils/time'
import { setSettings } from '@/api/app'
import type { SyncStatus } from '@/api/sync'
import type { RemotePack } from '@/api/setup'
import {
  auditDone,
  closeSettings,
  exchangeReceived,
  fileOpened,
  flowMain,
  initialApp,
  lockSettings,
  openSettings,
  setRemoteVaults,
  setSettingsSection,
  setSyncStatus,
  setupAccountFailed,
  setupAccountPending,
  setupAccountProbed,
  useApp,
  usePrefs,
  useUi
} from '@/store'
import DateField from '@/components/elements/fields/DateField'
import { PICKED } from '@/components/elements/tokens'
import { FieldsProvider } from '@/components/elements/fields/context'
import Footer from '@/components/Main/Body/Aside/Show/Footer'
import { EVENTS } from '@/api/events'
import { subscribeToEvents } from '@/store/events'
import { emitEvent } from './events'
import { appStatusDefault, calls, clearCalls, mockCommand, mockCommandOnce } from './ipc'
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

const heard = (supported: boolean, seq: number) =>
  act(() => emitEvent(EVENTS.oslockStatus, { supported, seq }))

// Settings opens on General; the sync tests want the Sync & backup overview,
// or one of the pages a row of it opens.
const openSync = async () => {
  const opened = await open()
  await go('sync')
  return opened
}

const openSyncPage = async (page: 'nearby' | 'cloud' | 'backup') => {
  const opened = await openSync()
  await userEvent.click(screen.getByTestId(`settings-sync-open-${page}`))
  return opened
}
const openCloud = () => openSyncPage('cloud')
const openBackup = () => openSyncPage('backup')

// An ISO stamp whole days before now, as a backup saved then is recorded.
const daysAgo = (days: number) => new Date(Date.now() - days * 86_400_000).toISOString()

// A workspace row's ⋯, which is where Edit and Delete live.
// The ways to one more lockbox are the menu the list's one Add row opens.
const openAdd = async (way: 'new-row' | 'restore-open' | 'backup-open') => {
  await userEvent.click(screen.getByTestId('workspace-add-row'))
  await userEvent.click(screen.getByTestId(`workspace-${way}`))
}

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
      'settings-nav-keyboard',
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

describe('Settings › sync overview', () => {
  const summary = () => screen.getByTestId('settings-sync-summary')
  const cloudStatus = () => screen.getByTestId('settings-sync-open-cloud-status')

  it('lists each way to sync or back up, and connects nothing itself', async () => {
    await openSync()
    expect(screen.getByRole('heading', { name: 'Sync & backup' })).toBeInTheDocument()
    expect(screen.getByTestId('settings-sync-open-cloud')).toHaveTextContent('Your cloud storage')
    // Saving a copy and bringing one back are two things to do, so two rows.
    expect(screen.getByTestId('settings-sync-open-backup')).toHaveTextContent('Back up the vault')
    expect(screen.getByTestId('settings-sync-open-restore')).toHaveTextContent(
      'Restore from a backup'
    )
    // No nearby row until the backend says this build can do it.
    expect(screen.queryByTestId('settings-sync-open-nearby')).not.toBeInTheDocument()
    expect(screen.queryByTestId('settings-drive-connect')).not.toBeInTheDocument()
    expect(screen.queryByTestId('settings-backup-saves')).not.toBeInTheDocument()
  })

  it('says a vault that syncs nowhere is on this device, and offers a backup', async () => {
    await openSync()
    expect(summary()).toHaveAttribute('data-tone', 'local')
    expect(summary()).toHaveTextContent('On this device only')
    expect(cloudStatus()).toHaveTextContent('Not connected')

    await userEvent.click(screen.getByTestId('settings-sync-backup'))
    expect(screen.getByRole('heading', { name: 'Back up the vault' })).toBeInTheDocument()
    expect(screen.getByTestId('settings-backup-saves')).toBeInTheDocument()
  })

  // The backup row says where backups stand across every lockbox that can
  // have one: none saved, some, or all — then how far back the oldest goes.
  // The org lockbox is not one of them, never saved or not.
  it('says on the backup row whether each lockbox has been saved', async () => {
    const backupStatus = () => screen.getByTestId('settings-sync-open-backup-status')
    const lockboxes = (personal?: string, work?: string) =>
      seedApp({
        workspaces: [
          { id: 'default', name: null, backedUpAt: personal },
          { id: 'w2', name: 'Work', backedUpAt: work },
          { id: 'w-org', name: 'Deploy', server: 'https://rowel.acme.example' }
        ],
        activeWorkspace: 'default'
      })

    lockboxes()
    await openSync()
    expect(backupStatus()).toHaveTextContent('Never saved')

    act(() => lockboxes(daysAgo(1)))
    expect(backupStatus()).toHaveTextContent('Some lockboxes never saved')

    act(() => lockboxes(daysAgo(1), daysAgo(3)))
    expect(backupStatus()).toHaveTextContent('Oldest backup 3 days ago')
  })

  // A vault of one lockbox has no "oldest" to speak of: its own last save.
  it('says when the one lockbox was last saved', async () => {
    seedApp({ workspaces: [{ id: 'default', name: null, backedUpAt: daysAgo(2) }] })
    await openSync()
    expect(screen.getByTestId('settings-sync-open-backup-status')).toHaveTextContent(
      'Last saved 2 days ago'
    )
  })

  // The offer is for a vault with nothing to fall back on: once every
  // lockbox has a backup saved, the card says where it stands and no more.
  it('stops offering a backup once every lockbox has one', async () => {
    const lockboxes = (work?: string) =>
      seedApp({
        workspaces: [
          { id: 'default', name: null, backedUpAt: daysAgo(1) },
          { id: 'w2', name: 'Work', backedUpAt: work }
        ],
        activeWorkspace: 'default'
      })

    lockboxes()
    await openSync()
    expect(screen.getByTestId('settings-sync-backup')).toBeInTheDocument()

    act(() => lockboxes(daysAgo(5)))
    expect(summary()).toHaveTextContent('On this device only')
    expect(screen.queryByTestId('settings-sync-backup')).not.toBeInTheDocument()
  })

  it('says where the cloud account stands on its row and in the summary', async () => {
    await openSync()

    act(() =>
      report({ configured: true, provider: 'dropbox', lastSyncedAt: '2026-01-01T00:00:00Z' })
    )
    expect(cloudStatus()).toHaveTextContent('Dropbox')
    expect(cloudStatus()).toHaveAttribute('data-tone', 'good')

    // Connected a moment ago, with no run landed: not yet the green name.
    act(() => report({ configured: true, provider: 'dropbox' }))
    expect(cloudStatus()).toHaveTextContent('Waiting to sync')
    expect(cloudStatus()).toHaveAttribute('data-tone', 'neutral')
    act(() =>
      report({ configured: true, provider: 'dropbox', lastSyncedAt: '2026-01-01T00:00:00Z' })
    )
    expect(summary()).toHaveTextContent('Your vault is in sync')
    expect(summary()).toHaveTextContent('Through Dropbox')
    expect(screen.queryByTestId('settings-sync-backup')).not.toBeInTheDocument()

    act(() => report({ configured: true, provider: 'dropbox', inProgress: true }))
    expect(cloudStatus()).toHaveTextContent('Syncing…')

    act(() => report({ configured: true, provider: 'dropbox', error: 'Dropbox API 503' }))
    expect(cloudStatus()).toHaveTextContent('Last attempt failed')
    expect(cloudStatus()).toHaveAttribute('data-tone', 'bad')
    expect(summary()).toHaveTextContent('Dropbox API 503')

    // The failure's own page is one press away.
    await userEvent.click(screen.getByTestId('settings-sync-review'))
    expect(screen.getByRole('heading', { name: 'Your cloud storage' })).toBeInTheDocument()
    expect(screen.getByTestId('settings-sync-error')).toHaveTextContent('Dropbox API 503')
  })

  // A failure whose controls are elsewhere is sent there, not to a page that
  // could only repeat the sentence.
  it.each<[NonNullable<SyncStatus['errorKind']>]>([
    ['vaultDeletedRemotely'],
    ['vaultUnderRetiredPassword']
  ])('sends a %s failure to Lockboxes, where its choices are', async errorKind => {
    await openSync()
    act(() => report({ configured: true, error: 'stopped', errorKind }))

    await userEvent.click(screen.getByTestId('settings-sync-review'))
    expect(useUi.getState().settingsSection).toBe('workspaces')
    expect(screen.getByRole('heading', { name: 'Lockboxes' })).toBeInTheDocument()
  })

  it('asks for the new master password when another device changed it', async () => {
    await openSync()
    act(() =>
      report({ configured: true, error: 'changed', errorKind: 'passwordChangedRemotely' })
    )

    await userEvent.click(screen.getByTestId('settings-sync-review'))
    expect(useUi.getState().passwordPrompt).toBe(true)
    expect(useUi.getState().settingsSection).toBe('sync')
  })

  // `openSettings` is called from outside the modal (a backup the OS opened
  // goes to Import), and knows nothing of the page the modal is on.
  it('leaves a page when something outside moves Settings to another section', async () => {
    await openSync()
    await userEvent.click(screen.getByTestId('settings-sync-open-backup'))
    expect(screen.getByRole('heading', { name: 'Back up the vault' })).toBeInTheDocument()

    act(() => openSettings('import'))
    expect(screen.getByRole('heading', { name: 'Import & export' })).toBeInTheDocument()
    expect(screen.queryByTestId('settings-subpage-back')).not.toBeInTheDocument()

    // And it is not waiting there on the way back.
    act(() => openSettings('sync'))
    expect(screen.getByRole('heading', { name: 'Sync & backup' })).toBeInTheDocument()
    expect(screen.getByTestId('settings-sync-open-backup')).toBeInTheDocument()
  })

  it('opens a page per row and comes back to the overview', async () => {
    await openSync()
    await userEvent.click(screen.getByTestId('settings-sync-open-cloud'))

    expect(screen.getAllByRole('heading', { level: 1 })).toHaveLength(1)
    expect(screen.getByRole('heading', { name: 'Your cloud storage' })).toBeInTheDocument()
    expect(screen.getByTestId('settings-cloud')).toBeInTheDocument()
    expect(screen.queryByTestId('settings-sync-open-cloud')).not.toBeInTheDocument()

    await userEvent.click(screen.getByTestId('settings-subpage-back'))
    expect(screen.getByRole('heading', { name: 'Sync & backup' })).toBeInTheDocument()
    expect(screen.getByTestId('settings-sync-open-cloud')).toBeInTheDocument()
  })
})

describe('Settings › sync', () => {
  it('connects Google Drive', async () => {
    await openCloud()
    await userEvent.click(screen.getByTestId('settings-drive-connect'))
    expect(calls('sync_connect')).toEqual([{ provider: 'googleDrive' }])
  })

  // Not connected, the card is about no provider yet: it offers each, and the
  // one pressed is the one the wait and the probe are about.
  it('offers every provider, and connects the one picked', async () => {
    await openCloud()
    const providers = screen.getByTestId('settings-providers')
    expect(providers).toHaveTextContent('Choose a provider')
    expect(providers).toHaveTextContent('Dropbox')
    // No account yet, so no account card: the list stands in its place.
    expect(screen.queryByTestId('settings-drive-row')).not.toBeInTheDocument()
    expect(screen.getByTestId('settings-drive-connect')).toHaveTextContent('Connect Google Drive')
    // Only the two every build has: a third needs the build to report it.
    expect(screen.queryByTestId('settings-icloud-connect')).not.toBeInTheDocument()

    await userEvent.click(screen.getByTestId('settings-dropbox-connect'))

    expect(calls('sync_connect')).toEqual([{ provider: 'dropbox' }])
    expect(screen.getByText('Waiting for Dropbox…')).toBeInTheDocument()
    expect(screen.getByTestId('settings-account-name')).toHaveTextContent('Dropbox')
    expect(screen.queryByTestId('settings-drive-connect')).not.toBeInTheDocument()
  })

  // The picker draws what the build reported it can connect: an Apple build
  // signed for iCloud reports a third, after the two every build has.
  it('offers iCloud on a build that reports it', async () => {
    seedApp({ providers: ['googleDrive', 'dropbox', 'iCloud'] })
    await openCloud()

    const row = screen.getByTestId('settings-providers')
    const ids = [...row.querySelectorAll('[data-testid$="-connect"]')].map(node =>
      node.getAttribute('data-testid')
    )
    expect(ids).toEqual([
      'settings-drive-connect',
      'settings-dropbox-connect',
      'settings-icloud-connect'
    ])
    expect(screen.getByTestId('settings-icloud-connect')).toHaveTextContent('Connect iCloud')
  })

  // iCloud asks no browser for consent, so the wait is a check of the account
  // rather than a wait on a page the user has to come back from.
  it('says it is checking the account while an iCloud connect is out', async () => {
    seedApp({ providers: ['googleDrive', 'dropbox', 'iCloud'] })
    await openCloud()
    await userEvent.click(screen.getByTestId('settings-icloud-connect'))

    expect(calls('sync_connect')).toEqual([{ provider: 'iCloud' }])
    expect(screen.getByText('Checking your iCloud account…')).toBeInTheDocument()
    expect(screen.getByTestId('settings-account-name')).toHaveTextContent('iCloud')
  })

  // Connected, the card names the account's provider — and Sharing, which only
  // Drive can hold yet, is left out for any other.
  it('names a Dropbox connection and leaves Sharing out', async () => {
    await openCloud()
    act(() => report({ configured: true, provider: 'dropbox' }))

    expect(screen.getByTestId('settings-account-name')).toHaveTextContent('Dropbox')
    expect(screen.getByText('Connected')).toBeInTheDocument()
    expect(screen.queryByTestId('settings-dropbox-connect')).not.toBeInTheDocument()
    expect(screen.queryByTestId('settings-shares-row')).not.toBeInTheDocument()

    act(() => report({ configured: true, provider: 'googleDrive' }))
    expect(screen.getByTestId('settings-account-name')).toHaveTextContent('Google Drive')
    expect(screen.getByTestId('settings-shares-row')).toBeInTheDocument()
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
    await openCloud()

    expect(screen.queryByTestId('settings-sync-primary-only')).not.toBeInTheDocument()
    await userEvent.click(screen.getByTestId('settings-drive-connect'))
    expect(calls('sync_connect')).toHaveLength(1)
  })

  // The mobile shape of the same flow: `sync_connect` resolves as soon as
  // Safari has the screen, so the row waits on the backend's events — the
  // click itself claims nothing.
  it('waits for Google after a connect that resolved early', async () => {
    await openCloud()
    await userEvent.click(screen.getByTestId('settings-drive-connect'))
    expect(useApp.getState().sync.pending).toBe(false)

    report({ pending: true })
    expect(await screen.findByText('Waiting for Google Drive…')).toBeInTheDocument()

    report({ configured: true })
    expect(await screen.findByText('Connected')).toBeInTheDocument()
  })

  it('reports a consent that failed, and stays disconnected', async () => {
    await openCloud()
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
    await openCloud()
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
      await openCloud()
      await userEvent.click(screen.getByTestId('settings-drive-connect'))
      await act(async () => setupAccountPending())
      expect(await screen.findByText('Waiting for Google Drive…')).toBeInTheDocument()

      await act(async () => setupAccountProbed([]))

      await waitFor(() => expect(calls('sync_adopt_pending')).toHaveLength(1))
      // The probe's state is spent once the account is this vault's; from here
      // on `sync:status` says the rest.
      await waitFor(() => expect(useApp.getState().setupAccount.status).toBe('idle'))
      report({ configured: true })
      expect(await screen.findByText('Connected')).toBeInTheDocument()
    })

    // A second device of a vault the account already holds: the backend adopts
    // and the picker is never shown.
    it('joins an account that already holds this vault, with no picker', async () => {
      await openCloud()
      await userEvent.click(screen.getByTestId('settings-drive-connect'))
      await act(async () => setupAccountProbed([VAULT, OTHER_VAULT]))

      await waitFor(() => expect(calls('sync_adopt_pending')).toHaveLength(1))
      await waitFor(() => expect(useApp.getState().setupAccount.status).toBe('idle'))
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
      await openCloud()
      await userEvent.click(screen.getByTestId('settings-drive-connect'))
      await act(async () => setupAccountProbed([VAULT]))

      await waitFor(() => expect(calls('sync_adopt_pending')).toHaveLength(1))
      expect(screen.getByText('Waiting for Google Drive…')).toBeInTheDocument()
      expect(screen.queryByTestId('settings-drive-found')).not.toBeInTheDocument()

      await act(async () => decide())
      await waitFor(() => expect(useApp.getState().setupAccount.status).toBe('idle'))
    })

    // The account already says which vaults exist, so this vault does not add
    // to them: the user restores one, with the same picker and form Settings ›
    // Workspaces uses, and the vault that was open stays as its own workspace.
    it("offers the account's vaults to restore when it holds only other ones", async () => {
      refuse()
      await openCloud()
      await userEvent.click(screen.getByTestId('settings-drive-connect'))
      await act(async () => setupAccountProbed([VAULT, OTHER_VAULT]))

      expect(await screen.findByTestId('settings-drive-found')).toBeInTheDocument()
      expect(screen.getByText('This account already holds a lockbox')).toBeInTheDocument()
      expect(screen.getByTestId(`drive-vault-${VAULT.id}`)).toBeInTheDocument()
      // Asked once, and refused; the tokens are still pending for the restore.
      expect(calls('sync_adopt_pending')).toHaveLength(1)
      expect(useApp.getState().setupAccount.status).toBe('found')

      await userEvent.click(screen.getByTestId(`drive-vault-${OTHER_VAULT.id}`))
      await userEvent.type(screen.getByTestId('workspace-restore-name'), 'Personal')
      await userEvent.type(screen.getByTestId('workspace-restore-password'), 'other-device-pass')
      await act(async () => {
        await userEvent.click(screen.getByTestId('workspace-restore-submit'))
      })

      expect(calls('workspace_restore_from_pending')).toContainEqual({
        name: 'Personal',
        password: 'other-device-pass',
        fileId: OTHER_VAULT.id
      })
    })

    it('cancels out of the offer by forgetting the account', async () => {
      refuse()
      await openCloud()
      await userEvent.click(screen.getByTestId('settings-drive-connect'))
      await act(async () => setupAccountProbed([VAULT]))

      await userEvent.click(await screen.findByTestId('settings-drive-cancel'))

      expect(calls('setup_disconnect')).toHaveLength(1)
      expect(screen.queryByTestId('settings-drive-found')).not.toBeInTheDocument()
      expect(screen.getByTestId('settings-drive-connect')).toBeInTheDocument()
    })

    // The rule restoring under Lockboxes keeps too: a connect walked away from
    // goes with the page, and a page left with none started calls nothing.
    it('forgets a connect the page is left with, and calls nothing without one', async () => {
      await openCloud()
      await userEvent.click(screen.getByTestId('settings-subpage-back'))
      expect(calls('setup_disconnect')).toHaveLength(0)

      await userEvent.click(screen.getByTestId('settings-sync-open-cloud'))
      await userEvent.click(screen.getByTestId('settings-drive-connect'))
      await userEvent.click(screen.getByTestId('settings-subpage-back'))

      expect(calls('setup_disconnect')).toHaveLength(1)
      expect(useApp.getState().setupAccount.status).toBe('idle')
    })

    // Any other reason the backend could not adopt (the account did not answer,
    // the vault locked) is a failure like the probe's, not an offer.
    it('reports an adopt that failed for another reason', async () => {
      mockCommandOnce('sync_adopt_pending', () =>
        Promise.reject({ kind: 'other', message: 'Google Drive did not answer' })
      )
      await openCloud()
      await userEvent.click(screen.getByTestId('settings-drive-connect'))
      await act(async () => setupAccountProbed([]))

      expect(await screen.findByTestId('settings-sync-error')).toHaveTextContent(
        'Google Drive did not answer'
      )
      expect(screen.queryByTestId('settings-drive-found')).not.toBeInTheDocument()
      expect(screen.getByTestId('settings-drive-connect')).toBeInTheDocument()
    })

    it('reports a probe that failed, and offers to try again', async () => {
      await openCloud()
      await userEvent.click(screen.getByTestId('settings-drive-connect'))
      await act(async () => setupAccountFailed('Google Drive did not answer'))

      expect(await screen.findByTestId('settings-sync-error')).toHaveTextContent(
        'Google Drive did not answer'
      )
      expect(screen.getByTestId('settings-drive-connect')).toBeInTheDocument()
      expect(useApp.getState().sync.configured).toBe(false)
    })
  })

  // One lockbox, one row: the page still reads as saving the vault there is.
  it('offers the encrypted backup behind its own control', async () => {
    await openBackup()
    const saves = screen.getByTestId('settings-backup-saves')
    expect(saves).toHaveTextContent('.rowel')
    expect(saves).toHaveTextContent('One file per lockbox, sealed with your master password.')
    // It says which lockbox the file holds, and that it has none yet.
    expect(screen.getByTestId('settings-backup-row-default')).toHaveTextContent('Personal')
    expect(screen.getByTestId('settings-backup-row-default')).toHaveTextContent('Never saved')
    expect(document.querySelector('input[name="export_password"]')).toBeNull()
    await userEvent.click(screen.getByText('Save…'))
    expect(document.querySelector('input[name="export_password"]')).toBeInTheDocument()
  })

  describe('with several lockboxes', () => {
    const status = (work?: string, activeWorkspace = 'default') => ({
      workspaces: [
        { id: 'default', name: null, backedUpAt: '2024-01-15T12:00:00.000Z' },
        { id: 'w-org', name: 'Deploy', server: 'https://rowel.acme.example' },
        { id: 'w2', name: 'Work', backedUpAt: work }
      ],
      activeWorkspace
    })
    const rows = () =>
      [
        ...screen
          .getByTestId('settings-backup-saves')
          .querySelectorAll('[data-testid^="settings-backup-row-"]')
      ]
        .map(node => node.getAttribute('data-testid'))
        .filter(id => !id?.endsWith('-toggle'))

    const saveWork = async (password: string) => {
      await userEvent.click(screen.getByTestId('settings-backup-row-w2-toggle'))
      await userEvent.type(
        within(screen.getByTestId('settings-backup-row-w2')).getByLabelText('Master password'),
        password
      )
      await userEvent.click(screen.getByTestId('settings-backup-save-w2'))
    }

    // Every lockbox that can be saved, the open one first, each saying when
    // it last was; the org lockbox is its server's, and is not offered.
    it('lists each lockbox with when it was last saved', async () => {
      seedApp(status(undefined, 'w2'))
      await openBackup()

      expect(rows()).toEqual(['settings-backup-row-w2', 'settings-backup-row-default'])
      expect(screen.queryByTestId('settings-backup-row-w-org')).not.toBeInTheDocument()
      const work = screen.getByTestId('settings-backup-row-w2')
      expect(work).toHaveTextContent('Work')
      expect(work).toHaveTextContent('Never saved')
      const personal = screen.getByTestId('settings-backup-row-default')
      expect(personal).toHaveTextContent('Personal')
      expect(personal).toHaveTextContent('Last saved 01/15/2024')
      // The chip is said once, over the list, not on every row.
      expect(screen.getAllByText('.rowel')).toHaveLength(1)
    })

    // A lockbox that is not open is saved by its id, with the password typed
    // beside it, and its row says so as soon as the probe has it.
    it('saves a lockbox that is not open, and says when', async () => {
      seedApp(status())
      mockCommand('export_vault', () => '/Users/me/Rowel backup Work 2026-11-23.rowel')
      mockCommand('app_status', () => ({ ...appStatusDefault(), ...status(daysAgo(0)) }))
      await openBackup()
      expect(screen.getByTestId('settings-backup-row-w2')).toHaveTextContent('Never saved')

      await saveWork('vault password')

      expect(calls('export_vault')).toEqual([{ workspaceId: 'w2', password: 'vault password' }])
      const row = screen.getByTestId('settings-backup-row-w2')
      expect(await within(row).findByText('Last saved just now')).toBeInTheDocument()
      expect(within(row).getByText('Saved')).toBeInTheDocument()
    })

    // A dialog dismissed saved nothing: nothing is said, nothing re-read, and
    // the password stays for another go.
    it('says nothing was saved when the dialog is dismissed', async () => {
      seedApp(status())
      await openBackup()
      clearCalls('app_status')

      await saveWork('vault password')

      expect(calls('export_vault')).toHaveLength(1)
      const row = screen.getByTestId('settings-backup-row-w2')
      await waitFor(() => expect(screen.getByTestId('settings-backup-save-w2')).toBeEnabled())
      expect(row).toHaveTextContent('Never saved')
      expect(within(row).queryByText('Saved')).not.toBeInTheDocument()
      expect(calls('app_status')).toHaveLength(0)
      expect(within(row).getByLabelText('Master password')).toHaveValue('vault password')
    })

    // Why a lockbox cannot be saved is the backend's to say, in its words.
    it('shows why a lockbox could not be saved', async () => {
      seedApp(status())
      mockCommand('export_vault', () =>
        Promise.reject({
          kind: 'unsupported',
          message: 'this lockbox is not unlocked on this device'
        })
      )
      await openBackup()

      await saveWork('vault password')

      expect(
        await within(screen.getByTestId('settings-backup-row-w2')).findByText(
          'this lockbox is not unlocked on this device'
        )
      ).toBeInTheDocument()
    })
  })

  // The portable export lives with Import now; this section keeps only the
  // encrypted backup.
  it('leaves the unencrypted export out of this section', async () => {
    await openBackup()
    expect(screen.queryByTestId('settings-export-run')).not.toBeInTheDocument()
  })

  // A backup restores as a lockbox of its own, on the page Lockboxes offers
  // for it too. Asked for here it opens under this section: the section does
  // not move, and Back leads to the overview it was asked from. The page that
  // saves backups offers none of it.
  it('opens the restore page from the overview, and steps back to it', async () => {
    await openBackup()
    expect(screen.queryByTestId('workspace-backup-pick')).not.toBeInTheDocument()
    await userEvent.click(screen.getByTestId('settings-subpage-back'))

    await userEvent.click(screen.getByTestId('settings-sync-open-restore'))

    expect(screen.getByRole('heading', { name: 'Restore from a backup' })).toBeInTheDocument()
    expect(screen.getByTestId('workspace-backup-pick')).toBeInTheDocument()
    expect(screen.queryByTestId('settings-backup-saves')).not.toBeInTheDocument()
    expect(useUi.getState().settingsSection).toBe('sync')

    await userEvent.click(screen.getByTestId('settings-subpage-back'))
    expect(screen.getByRole('heading', { name: 'Sync & backup' })).toBeInTheDocument()
    expect(screen.queryByTestId('settings-subpage-back')).not.toBeInTheDocument()
  })

  // The pill beside the title says where the connection stands, one tone per
  // state, with a sync running outranking the last one's failure.
  it('says where Drive stands in the status pill', async () => {
    await openCloud()
    const pill = () => screen.getByTestId('settings-drive-status')

    // No account, no card to carry a pill: the list of providers stands there.
    expect(screen.queryByTestId('settings-drive-status')).not.toBeInTheDocument()

    // Connected, but no run has landed: nothing is anywhere yet, so neither
    // the pill nor the card's figure claims the account is up to date.
    act(() => report({ configured: true }))
    expect(pill()).toHaveTextContent('Waiting to sync')
    expect(pill()).toHaveAttribute('data-tone', 'neutral')
    expect(screen.getByTestId('settings-drive-row')).not.toHaveTextContent('Up to date')

    act(() => report({ configured: true, lastSyncedAt: '2026-01-01T00:00:00Z' }))
    expect(pill()).toHaveTextContent('Up to date')
    expect(pill()).toHaveAttribute('data-tone', 'good')

    act(() => report({ configured: true, inProgress: true }))
    expect(pill()).toHaveTextContent('Syncing')
    expect(pill()).toHaveAttribute('data-tone', 'accent')

    act(() => report({ configured: true, error: 'Drive API 503' }))
    expect(pill()).toHaveTextContent('Last attempt failed')
    expect(pill()).toHaveAttribute('data-tone', 'bad')
    expect(screen.getByTestId('settings-sync-error')).toHaveTextContent('Drive API 503')
  })

  // The card's count is of the lockboxes this account's provider holds, read
  // the way Settings › Lockboxes reads each one, so the two pages agree: not
  // every lockbox on the device, and not the open one once the account's copy
  // of it is gone.
  it('counts the lockboxes that live in the account’s provider, as Lockboxes does', async () => {
    seedApp({
      workspaces: [
        { id: 'default', name: null },
        { id: 'w2', name: 'Work', synced: true, provider: 'googleDrive' },
        { id: 'w3', name: 'Family', synced: true, provider: 'dropbox' },
        { id: 'w4', name: 'Scratch', synced: false }
      ],
      activeWorkspace: 'default'
    })
    await openCloud()
    const lockboxes = () =>
      within(screen.getByTestId('settings-drive-row')).getByText('Lockboxes').nextElementSibling

    act(() => report({ configured: true, provider: 'googleDrive' }))
    expect(lockboxes()).toHaveTextContent(/^2$/)

    act(() =>
      report({
        configured: true,
        provider: 'googleDrive',
        error: 'gone',
        errorKind: 'vaultDeletedRemotely'
      })
    )
    expect(lockboxes()).toHaveTextContent(/^1$/)

    act(() => report({ configured: true, provider: 'googleDrive' }))
    await go('workspaces')
    const onDrive = screen
      .getAllByTestId(/^workspace-row-/)
      .filter(row => row.textContent?.includes('Google Drive'))
    expect(onDrive.map(row => row.dataset.testid)).toEqual([
      'workspace-row-default',
      'workspace-row-w2'
    ])
  })

  it('shows what a connected account holds, and the way out', async () => {
    await openCloud()
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

// The cloud page lists the lockboxes beside the account: the ones it holds
// (here or not yet here) and the ones that sync nowhere. Lockboxes in another
// provider's account, and org lockboxes, are not this account's.
describe('Settings › sync › lockboxes beside the account', () => {
  const pack = (id: string, provider: RemotePack['provider']): RemotePack => ({
    id,
    name: `${id}.rowel`,
    vaultId: id,
    size: 64_512,
    modifiedTime: '2024-01-01T00:00:00.000Z',
    provider
  })

  const several = () =>
    seedApp({
      workspaces: [
        { id: 'default', name: null },
        { id: 'w2', name: 'Work', synced: true, provider: 'googleDrive' },
        { id: 'w3', name: 'Family', synced: true, provider: 'dropbox' },
        { id: 'w4', name: 'Scratch', synced: false },
        { id: 'w5', name: 'Acme', server: 'https://vault.acme.test' }
      ],
      activeWorkspace: 'default'
    })

  const list = () => screen.getByTestId('settings-cloud-lockboxes')
  // The rows in the order drawn, by test id.
  const rows = () =>
    within(list())
      .getAllByTestId(/^settings-cloud-(lockbox|remote)-/)
      .map(row => row.dataset.testid!)
      .filter(id => !id.endsWith('-status'))
  const status = (row: string) => screen.getByTestId(`${row}-status`)
  const figure = () =>
    within(screen.getByTestId('settings-drive-row')).getByText('Lockboxes').nextElementSibling

  // The listing a run reported is that account's. After a disconnect, the
  // next account connected starts with none of it, on the list and the figure.
  it('lists nothing of an account that was disconnected', async () => {
    several()
    await openCloud()
    act(() => report({ configured: true, provider: 'googleDrive' }))
    await act(async () => setRemoteVaults([pack('drive-file-1', 'googleDrive')]))
    expect(rows()).toContain('settings-cloud-remote-drive-file-1')
    expect(figure()).toHaveTextContent('3')

    act(() => report({ configured: false }))
    act(() => report({ configured: true, provider: 'googleDrive' }))

    expect(rows()).not.toContain('settings-cloud-remote-drive-file-1')
    expect(figure()).toHaveTextContent('2')
  })

  it('says where each lockbox stands, the open one first', async () => {
    several()
    await openCloud()
    act(() => report({ configured: true, provider: 'googleDrive' }))
    await act(async () => setRemoteVaults([pack('drive-file-1', 'googleDrive')]))

    expect(rows()).toEqual([
      'settings-cloud-lockbox-default',
      'settings-cloud-lockbox-w2',
      'settings-cloud-lockbox-w4',
      'settings-cloud-remote-drive-file-1'
    ])

    expect(screen.getByTestId('settings-cloud-lockbox-default')).toHaveTextContent('Personal')
    expect(screen.getByTestId('settings-cloud-lockbox-default')).toHaveTextContent('Open now')
    expect(status('settings-cloud-lockbox-default')).toHaveTextContent(/^Google Drive$/)

    expect(screen.getByTestId('settings-cloud-lockbox-w2')).not.toHaveTextContent('Open now')
    expect(status('settings-cloud-lockbox-w2')).toHaveTextContent(/^Google Drive$/)

    // Syncs nowhere: said in the words Lockboxes uses, and nothing to press —
    // no command brings a lockbox that is not open into the account.
    expect(status('settings-cloud-lockbox-w4')).toHaveTextContent(/^This device$/)
    expect(
      within(screen.getByTestId('settings-cloud-lockbox-w4')).queryByRole('button')
    ).not.toBeInTheDocument()

    // In the account and not here: told apart by date and size.
    expect(screen.getByTestId('settings-cloud-remote-drive-file-1')).toHaveTextContent(
      'Rowel lockbox'
    )
    expect(status('settings-cloud-remote-drive-file-1')).toHaveTextContent(
      /^Not on this device yet · .+/
    )
  })

  // One reading feeds both: the account holds what is in its provider here,
  // and what is in it and not here yet — never what syncs nowhere.
  it('agrees with the card on how many lockboxes the account holds', async () => {
    several()
    await openCloud()
    act(() => report({ configured: true, provider: 'googleDrive' }))
    await act(async () => setRemoteVaults([pack('drive-file-1', 'googleDrive')]))

    const held = () => rows().filter(row => status(row).textContent !== 'This device')
    expect(held()).toHaveLength(3)
    expect(figure()).toHaveTextContent(/^3$/)

    // The account's copy of the open one is gone: it is on this device alone,
    // on the card and in the list alike.
    act(() =>
      report({
        configured: true,
        provider: 'googleDrive',
        error: 'gone',
        errorKind: 'vaultDeletedRemotely'
      })
    )
    expect(status('settings-cloud-lockbox-default')).toHaveTextContent(/^This device$/)
    expect(held()).toHaveLength(2)
    expect(figure()).toHaveTextContent(/^2$/)
  })

  it('leads from a lockbox not on this device to Lockboxes, where it is restored', async () => {
    several()
    await openCloud()
    act(() => report({ configured: true, provider: 'googleDrive' }))
    await act(async () => setRemoteVaults([pack('drive-file-1', 'googleDrive')]))

    await userEvent.click(screen.getByTestId('settings-cloud-remote-drive-file-1'))
    expect(useUi.getState().settingsSection).toBe('workspaces')
    expect(screen.getByRole('heading', { name: 'Lockboxes' })).toBeInTheDocument()
    expect(screen.getByTestId('workspace-remote-row')).toBeInTheDocument()
  })

  it('lists nothing before an account is connected, nor the open lockbox alone', async () => {
    several()
    await openCloud()
    await act(async () => setRemoteVaults([pack('drive-file-1', 'googleDrive')]))

    // The choice of provider stands alone.
    expect(screen.getByTestId('settings-providers')).toBeInTheDocument()
    expect(screen.queryByTestId('settings-cloud-lockboxes')).not.toBeInTheDocument()

    // Connected, with nothing on the device or in the account but the open
    // one: the card already says everything the list would.
    act(() => {
      seedApp({ workspaces: [{ id: 'default', name: null }], activeWorkspace: 'default' })
      setRemoteVaults([])
      report({ configured: true, provider: 'googleDrive' })
    })
    expect(figure()).toHaveTextContent(/^1$/)
    expect(screen.queryByTestId('settings-cloud-lockboxes')).not.toBeInTheDocument()

    // Something else in the account is worth the list.
    await act(async () => setRemoteVaults([pack('drive-file-1', 'googleDrive')]))
    expect(rows()).toEqual(['settings-cloud-lockbox-default', 'settings-cloud-remote-drive-file-1'])
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
    expect(await screen.findByText('Password changed')).toBeInTheDocument()
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
        'Password changed. Some lockboxes on this device kept their own password.'
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

  it('offers no OS lock where the OS locking is not heard', async () => {
    await open()
    await go('security')

    expect(screen.queryByTestId('settings-autolock-os')).not.toBeInTheDocument()
    expect(screen.getByText('Idle time before the vault seals itself')).toBeInTheDocument()
  })

  it('offers the OS lock once the listener comes up after the launch probe', async () => {
    subscribeToEvents()
    await open()
    await go('security')
    expect(screen.queryByTestId('settings-autolock-os')).not.toBeInTheDocument()

    heard(true, 1)

    expect(screen.getByTestId('settings-autolock-os')).toBeInTheDocument()
    expect(screen.getByText(/it also seals whenever this computer locks or sleeps/)).toBeInTheDocument()
  })

  it('follows the OS lock, keeping the minutes chosen before', async () => {
    subscribeToEvents()
    heard(true, 1)
    await open()
    await go('security')
    expect(screen.getByText(/it also seals whenever this computer locks or sleeps/)).toBeInTheDocument()

    await userEvent.click(screen.getByTestId('settings-autolock-os'))

    expect(calls('set_settings')).toContainEqual({ patch: { autolockFollowsOs: true } })
    expect(usePrefs.getState().autolockSecs).toBe(60)
    expect(screen.getByTestId('settings-autolock-os')).toHaveAttribute('aria-checked', 'true')
    expect(
      screen.getByText('Seals whenever this computer locks or sleeps, or after a day idle')
    ).toBeInTheDocument()
  })

  it('goes back to a timeout from following the OS', async () => {
    subscribeToEvents()
    heard(true, 1)
    await open()
    await go('security')
    await userEvent.click(screen.getByTestId('settings-autolock-os'))

    await userEvent.click(screen.getByTestId('settings-autolock-900'))

    expect(calls('set_settings')).toContainEqual({ patch: { autolockSecs: 900 } })
    expect(calls('set_settings')).toContainEqual({ patch: { autolockFollowsOs: false } })
    expect(screen.getByTestId('settings-autolock-900')).toHaveAttribute('aria-checked', 'true')
  })

  it('shows the minutes Rust falls back to when the listener is lost', async () => {
    subscribeToEvents()
    heard(true, 1)
    await open()
    await go('security')
    await userEvent.click(screen.getByTestId('settings-autolock-os'))

    heard(false, 2)

    expect(screen.queryByTestId('settings-autolock-os')).not.toBeInTheDocument()
    expect(screen.getByTestId('settings-autolock-60')).toHaveAttribute('aria-checked', 'true')
    expect(screen.getByText('Idle time before the vault seals itself')).toBeInTheDocument()
    expect(usePrefs.getState().autolockFollowsOs).toBe(true)
  })

  it('shows the minutes in force when the OS is not heard after all', async () => {
    const stored = { autolockSecs: 900, autolockFollowsOs: true }
    await setSettings(stored)
    usePrefs.setState(stored)
    await open()
    await go('security')

    expect(screen.getByTestId('settings-autolock-900')).toHaveAttribute('aria-checked', 'true')
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

    await userEvent.type(screen.getByLabelText('New password'), 'newpass')
    await userEvent.type(screen.getByLabelText('Repeat new password'), 'newpa')
    expect(screen.getByText("Passwords don't match yet.")).toBeInTheDocument()
    expect(screen.getByTestId('change-password-submit')).toBeDisabled()

    await userEvent.type(screen.getByLabelText('Repeat new password'), 'ss')
    expect(screen.queryByText("Passwords don't match yet.")).not.toBeInTheDocument()

    const field = screen.getByLabelText('New password')
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
    expect(strip).toHaveTextContent('weak1')
    expect(strip).toHaveTextContent('reused1')
    // The label sits over its figure. Breaches are not counted while
    // monitoring is off; the strip offers it.
    expect(strip).toHaveTextContent('breached—')
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
  // What `import_exchange` answers for three rows: a preview, or the write.
  const exchanged = (dryRun: boolean) => ({
    total: 3,
    imported: dryRun ? 0 : 3,
    skipped: 0,
    duplicates: 0,
    dryRun,
    errors: [],
    warnings: [],
    entries: []
  })

  it('imports a .swftx file with the source password', async () => {
    mockCommand('pick_file', () => '/tmp/other.swftx')
    mockCommand('import_swftx', () => 3)
    await open()
    await go('import')

    await userEvent.click(screen.getByTestId('import-tile-swftx'))
    expect(await screen.findByText('other.swftx')).toBeInTheDocument()
    // Picked in Rust, which grants the path for the merge that reads it.
    expect(calls('pick_file')).toContainEqual({ kind: 'importBackup', label: 'Rowel backup' })

    await userEvent.type(
      document.querySelector('input[name="import_password"]')!,
      'source-pw'
    )
    await userEvent.click(screen.getByTestId('import-run-backup'))

    expect(calls('import_swftx')).toContainEqual({ path: '/tmp/other.swftx', password: 'source-pw' })
    expect(await screen.findByText(/Imported/)).toBeInTheDocument()
  })

  it('shows an error when the backup password is wrong', async () => {
    mockCommand('pick_file', () => '/tmp/other.swftx')
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
  // open there is nothing to merge it into, and the section says what it is
  // instead of asking for a password it could not use.
  it('names a .rowel the OS opened and explains it is not imported', async () => {
    flowMain()
    render(<Settings />)

    fileOpened('/tmp/Rowel backup 2026-09-15.rowel')

    expect(await screen.findByText('Rowel backup 2026-09-15.rowel')).toBeInTheDocument()
    expect(screen.getByTestId('import-rowel-notice')).toHaveTextContent(
      'This is a Rowel backup. It is restored as a lockbox of its own, not imported.'
    )
    expect(document.querySelector('input[name="import_password"]')).not.toBeInTheDocument()
    expect(calls('import_swftx')).toEqual([])
  })

  // The link takes the file along: the open granted it, so the restore page
  // restores it without a second pick.
  it('restores a .rowel the OS opened as a lockbox, from the link', async () => {
    flowMain()
    render(<Settings />)
    fileOpened('/tmp/Rowel backup 2026-09-15.rowel')
    await userEvent.click(await screen.findByTestId('import-rowel-restore'))

    expect(screen.getByRole('heading', { name: 'Restore from a backup' })).toBeInTheDocument()
    expect(screen.getByTestId('workspace-backup-file')).toHaveTextContent(
      'Rowel backup 2026-09-15.rowel'
    )
    await userEvent.type(screen.getByTestId('workspace-backup-password'), 'backup-pass')
    await act(async () => {
      await userEvent.click(screen.getByTestId('workspace-backup-submit'))
    })

    expect(calls('pick_file')).toEqual([])
    expect(calls('workspace_restore_from_file')).toEqual([
      { name: '', password: 'backup-pass', path: '/tmp/Rowel backup 2026-09-15.rowel' }
    ])
  })

  // The restore page sits over Import, which is already the section a second
  // opened file asks for, so nothing moves to dismiss the page: it steps aside
  // itself, and Import takes the new file rather than leaving the first one's
  // form up.
  it('steps the restore page aside for another backup the OS opens', async () => {
    flowMain()
    render(<Settings />)
    fileOpened('/tmp/Rowel backup 2026-09-15.rowel')
    await userEvent.click(await screen.findByTestId('import-rowel-restore'))
    expect(screen.getByTestId('workspace-backup-file')).toHaveTextContent('2026-09-15')

    act(() => fileOpened('/tmp/Rowel backup 2026-10-01.rowel'))

    expect(screen.getByRole('heading', { name: 'Import & export' })).toBeInTheDocument()
    expect(screen.queryByTestId('settings-subpage-back')).not.toBeInTheDocument()
    expect(await screen.findByText('Rowel backup 2026-10-01.rowel')).toBeInTheDocument()
    expect(useApp.getState().openedFile).toBeNull()

    await userEvent.click(screen.getByTestId('import-rowel-restore'))
    expect(screen.getByTestId('workspace-backup-file')).toHaveTextContent('2026-10-01')
  })

  // A restore that is running holds Settings, and its page with it: the file
  // waits, and is taken once the restore lets go.
  it('keeps a locked restore page until the lock is released, then takes the file', async () => {
    flowMain()
    render(<Settings />)
    fileOpened('/tmp/Rowel backup 2026-09-15.rowel')
    await userEvent.click(await screen.findByTestId('import-rowel-restore'))

    act(() => lockSettings(true))
    act(() => fileOpened('/tmp/Rowel backup 2026-10-01.rowel'))
    expect(screen.getByRole('heading', { name: 'Restore from a backup' })).toBeInTheDocument()
    expect(screen.getByTestId('workspace-backup-file')).toHaveTextContent('2026-09-15')

    act(() => lockSettings(false))
    expect(screen.getByRole('heading', { name: 'Import & export' })).toBeInTheDocument()
    expect(await screen.findByText('Rowel backup 2026-10-01.rowel')).toBeInTheDocument()
  })

  // Another password manager handed its data over while this vault is open:
  // Settings opens on Import with a preview of it, from the backend that holds
  // it. It came from the OS, so no tile is lit and there is no file to name.
  it('previews what another app handed over, and imports it', async () => {
    mockCommand('import_exchange', args => exchanged(args.dryRun === true))
    flowMain()
    render(<Settings />)

    exchangeReceived()

    expect(useUi.getState().settings).toBe(true)
    expect(useUi.getState().settingsSection).toBe('import')
    const result = await screen.findByTestId('import-result')
    expect(result).toHaveTextContent('From another app')
    await waitFor(() => expect(result).toHaveTextContent('Ready to import: 3'))
    expect(calls('import_exchange')).toEqual([{ dryRun: true }])
    expect(calls('import_entries')).toEqual([])
    expect(useApp.getState().exchangePending).toBe(false)
    for (const tile of screen.getAllByTestId(/^import-tile-/)) expect(tile).not.toHaveClass(PICKED)
    // Nobody tapped anything, and on a phone the card is below the tiles.
    expect(vi.mocked(Element.prototype.scrollIntoView).mock.contexts).toContain(result)

    await userEvent.click(screen.getByTestId('import-commit'))

    await waitFor(() => expect(result).toHaveTextContent('Imported 3'))
    expect(calls('import_exchange')).toEqual([{ dryRun: true }, { dryRun: false }])
    expect(screen.queryByTestId('import-commit')).not.toBeInTheDocument()
    // Published like any other import.
    await waitFor(() => expect(calls('sync_now')).toHaveLength(1))
  })

  // The transfer itself can fail (the other app sent nothing, the user backed
  // out), and a lock drops one that was held: the backend says which.
  it('says why when what another app handed over cannot be previewed', async () => {
    mockCommand('import_exchange', () =>
      Promise.reject({ kind: 'other', message: 'the transfer did not complete' })
    )
    flowMain()
    render(<Settings />)

    exchangeReceived()

    expect(await screen.findByTestId('import-error')).toHaveTextContent(
      'the transfer did not complete'
    )
    expect(screen.getByTestId('import-result')).toHaveTextContent('From another app')
    expect(screen.queryByTestId('import-commit')).not.toBeInTheDocument()
  })

  // A page over Import steps aside for it, as it does for an opened file.
  it('steps the restore page aside for what another app hands over', async () => {
    mockCommand('import_exchange', args => exchanged(args.dryRun === true))
    flowMain()
    render(<Settings />)
    fileOpened('/tmp/Rowel backup 2026-09-15.rowel')
    await userEvent.click(await screen.findByTestId('import-rowel-restore'))
    expect(screen.getByRole('heading', { name: 'Restore from a backup' })).toBeInTheDocument()

    act(() => exchangeReceived())

    expect(screen.getByRole('heading', { name: 'Import & export' })).toBeInTheDocument()
    expect(await screen.findByTestId('import-result')).toHaveTextContent('From another app')
    expect(useApp.getState().exchangePending).toBe(false)
  })

  // On an iPhone that takes such a transfer there is no CSV to pick: Passwords
  // hands its data over itself, starting from its own menu. The tile says so
  // where a result would show, and still takes its export file from there.
  it('explains the transfer on the Apple tile where the device takes one', async () => {
    mockCommand('exchange_supported', () => true)
    mockCommand('pick_file', () => '/tmp/Passwords.csv')
    await open()
    await go('import')

    const tile = screen.getByTestId('import-tile-apple')
    await waitFor(() => expect(tile).toHaveTextContent('Export from the Passwords app'))
    await userEvent.click(tile)

    expect(screen.getByTestId('import-exchange-guide')).toHaveTextContent(
      'In the Passwords app, choose Export Data to Another App and pick Rowel. Rowel opens with a preview of what will be imported, passkeys included.'
    )
    expect(screen.getByTestId('import-result')).toHaveTextContent('Apple Passwords')
    expect(tile).toHaveClass(PICKED)
    expect(calls('pick_file')).toEqual([])
    expect(calls('import_entries')).toEqual([])

    const file = screen.getByTestId('import-exchange-file')
    expect(file).toHaveTextContent('Import a .csv file instead')
    await userEvent.click(file)

    await waitFor(() =>
      expect(calls('import_entries')).toEqual([
        { path: '/tmp/Passwords.csv', format: 'apple', dryRun: true }
      ])
    )
    expect(calls('pick_file')).toEqual([{ kind: 'import', label: 'Password exports' }])
    expect(screen.queryByTestId('import-exchange-guide')).not.toBeInTheDocument()
    expect(tile).toHaveClass(PICKED)
  })

  // Only the tile of the app that hands its data over changes.
  it('leaves every other tile a file picker on such a device', async () => {
    mockCommand('exchange_supported', () => true)
    mockCommand('pick_file', () => '/tmp/bitwarden.json')
    await open()
    await go('import')
    await waitFor(() =>
      expect(screen.getByTestId('import-tile-apple')).toHaveTextContent(
        'Export from the Passwords app'
      )
    )

    await userEvent.click(screen.getByTestId('import-tile-bitwarden'))

    await waitFor(() =>
      expect(calls('import_entries')).toContainEqual({
        path: '/tmp/bitwarden.json',
        format: 'bitwarden',
        dryRun: true
      })
    )
    expect(screen.queryByTestId('import-exchange-guide')).not.toBeInTheDocument()
  })

  // The backup card offers our own `.rowel` too, and takes it as the OS-opened
  // one above: named and explained, never sent to the legacy importer.
  it('explains a .rowel picked on the backup card instead of importing it', async () => {
    mockCommand('pick_file', () => '/tmp/Rowel backup 2026-09-15.rowel')
    await open()
    await go('import')

    await userEvent.click(screen.getByTestId('import-tile-swftx'))

    expect(await screen.findByTestId('import-rowel-notice')).toBeInTheDocument()
    expect(document.querySelector('input[name="import_password"]')).not.toBeInTheDocument()
    expect(calls('import_swftx')).toEqual([])

    // And offers the restore it is for, with the file already picked.
    await userEvent.click(screen.getByTestId('import-rowel-restore'))
    expect(screen.getByRole('heading', { name: 'Restore from a backup' })).toBeInTheDocument()
    expect(screen.getByTestId('workspace-backup-file')).toHaveTextContent(
      'Rowel backup 2026-09-15.rowel'
    )
    expect(calls('pick_file')).toHaveLength(1)

    // Over the section it was offered in, which Back leads to.
    expect(useUi.getState().settingsSection).toBe('import')
    await userEvent.click(screen.getByTestId('settings-subpage-back'))
    expect(screen.getByRole('heading', { name: 'Import & export' })).toBeInTheDocument()
  })

  // A legacy vault is still merged where it was picked, and offered no link.
  it('keeps a .swftx to the merge, with no restore link', async () => {
    mockCommand('pick_file', () => '/tmp/other.swftx')
    await open()
    await go('import')

    await userEvent.click(screen.getByTestId('import-tile-swftx'))

    expect(await screen.findByText('other.swftx')).toBeInTheDocument()
    expect(screen.queryByTestId('import-rowel-restore')).not.toBeInTheDocument()
    expect(screen.getByTestId('import-run-backup')).toBeInTheDocument()
  })

  // Imported rows are published like any other edit, so another device has
  // them without waiting for the next change made here.
  it('syncs after an export is imported, and not after one that added nothing', async () => {
    mockCommand('pick_file', () => '/tmp/bitwarden.json')
    let imported = 2
    mockCommand('import_entries', args => {
      const dryRun = args.dryRun === true
      return {
      total: 2,
      imported: dryRun ? 0 : imported,
      skipped: 0,
      duplicates: dryRun ? 0 : 2 - imported,
      dryRun,
      errors: [],
      warnings: [],
      entries: []
      }
    })
    await open()
    await go('import')

    await userEvent.click(screen.getByTestId('import-tile-bitwarden'))
    await userEvent.click(await screen.findByTestId('import-commit'))
    await waitFor(() => expect(calls('sync_now')).toHaveLength(1))

    imported = 0
    await userEvent.click(screen.getByTestId('import-tile-bitwarden'))
    await userEvent.click(await screen.findByTestId('import-commit'))
    await waitFor(() => expect(calls('import_entries')).toHaveLength(4))
    expect(calls('sync_now')).toHaveLength(1)
  })

  // Six tiles, three to a row: the apps most people are leaving, in order, and
  // a sixth whose menu holds the rest, so they are on offer without being
  // advertised on the same level.
  it('offers five app tiles and the rest behind a sixth', async () => {
    await open()
    await go('import')
    const tiles = screen.getAllByTestId(/^import-tile-/).map(el => el.dataset.testid)
    expect(tiles).toEqual([
      'import-tile-apple',
      'import-tile-1password',
      'import-tile-lastpass',
      'import-tile-bitwarden',
      'import-tile-keepass',
      'import-tile-other',
      'import-tile-swftx'
    ])

    await userEvent.click(screen.getByTestId('import-tile-other'))
    const menu = screen.getByRole('menu')
    for (const key of ['chrome', 'enpass', 'cxf', 'csv'])
      expect(within(menu).getByTestId(`import-tile-${key}`)).toBeInTheDocument()
  })

  // A row that imported but left something behind (an Enpass attachment) is
  // a note on the preview, apart from the rows that could not be read: the
  // totals still say every row is coming in. Two notes on one row are one
  // row in the heading.
  it('shows a warning row apart from the skipped count', async () => {
    mockCommand('pick_file', () => '/tmp/enpass.json')
    mockCommand('import_entries', () => ({
      total: 1,
      imported: 0,
      skipped: 0,
      duplicates: 0,
      dryRun: true,
      errors: [],
      warnings: [
        { row: 1, message: '2 attachment(s) not imported; keep the export' },
        { row: 1, message: 'unsupported or incomplete passkey' }
      ],
      entries: []
    }))
    await open()
    await go('import')

    await userEvent.click(screen.getByTestId('import-tile-other'))
    await userEvent.click(screen.getByTestId('import-tile-enpass'))
    const result = await screen.findByTestId('import-result')
    expect(result).toHaveTextContent('Ready to import: 1')
    expect(result).not.toHaveTextContent('skipped')
    expect(screen.queryByTestId('import-row-errors')).not.toBeInTheDocument()

    const note = screen.getByTestId('import-row-warnings')
    expect(note).toHaveTextContent('1 row imported with a note')
    await userEvent.click(within(note).getByText(/imported with a note/))
    expect(note).toHaveTextContent('Row 1: 2 attachment(s) not imported; keep the export')
    expect(note).toHaveTextContent('Row 1: unsupported or incomplete passkey')
  })

  // A tile is a file picker: the dry run names the format rather than
  // sniffing it, and the tile stays lit as the source. Apple's export is the
  // browser CSV, but under its own name, so its tile lights and not the sixth.
  it('imports an Apple Passwords export from its tile', async () => {
    mockCommand('pick_file', () => '/tmp/Passwords.csv')
    await open()
    await go('import')

    expect(screen.getByTestId('import-tile-apple')).toHaveTextContent(
      'iCloud Keychain, .csv export'
    )
    await userEvent.click(screen.getByTestId('import-tile-apple'))
    await waitFor(() =>
      expect(calls('import_entries')).toContainEqual({
        path: '/tmp/Passwords.csv',
        format: 'apple',
        dryRun: true
      })
    )
    expect(calls('pick_file')).toEqual([{ kind: 'import', label: 'Password exports' }])
    expect(screen.queryByTestId('import-exchange-guide')).not.toBeInTheDocument()
    expect(screen.getByTestId('import-tile-apple')).toHaveClass(PICKED)
    const other = screen.getByTestId('import-tile-other')
    expect(other).not.toHaveClass(PICKED)
    expect(other).toHaveTextContent('Another app')
    expect(other).toHaveTextContent('Chrome, Enpass, CXF or CSV')
  })

  // An app behind the sixth tile is picked by name just the same.
  it('imports an Enpass export from the other-apps menu', async () => {
    mockCommand('pick_file', () => '/tmp/enpass.json')
    await open()
    await go('import')

    await userEvent.click(screen.getByTestId('import-tile-other'))
    await userEvent.click(screen.getByTestId('import-tile-enpass'))
    await waitFor(() =>
      expect(calls('import_entries')).toContainEqual({
        path: '/tmp/enpass.json',
        format: 'enpass',
        dryRun: true
      })
    )
    expect(screen.getByTestId('import-tile-other')).toHaveTextContent('Enpass')
  })

  // An app from the sixth tile's menu is picked the same way, and the tile
  // takes that app's name while it is the source. Picking it again picks
  // another file: the menu items are buttons, not a selection.
  it('imports from an app in the other-apps menu, and again', async () => {
    mockCommand('pick_file', () => '/tmp/export.csv')
    await open()
    await go('import')

    await userEvent.click(screen.getByTestId('import-tile-other'))
    await userEvent.click(screen.getByTestId('import-tile-chrome'))
    await waitFor(() =>
      expect(calls('import_entries')).toContainEqual({
        path: '/tmp/export.csv',
        format: 'chrome',
        dryRun: true
      })
    )
    expect(screen.queryByRole('menu')).not.toBeInTheDocument()
    expect(screen.getByTestId('import-tile-other')).toHaveTextContent('Chrome / Safari')

    mockCommand('pick_file', () => '/tmp/export-2.csv')
    await userEvent.click(screen.getByTestId('import-tile-other'))
    await userEvent.click(screen.getByTestId('import-tile-chrome'))
    await waitFor(() =>
      expect(calls('import_entries')).toContainEqual({
        path: '/tmp/export-2.csv',
        format: 'chrome',
        dryRun: true
      })
    )
  })

  // The picker is on the drop target itself, on every platform; a file chosen
  // there has no tile behind it, so the backend sniffs its format.
  it('chooses an export file from the drop target', async () => {
    mockCommand('pick_file', () => '/tmp/export.csv')
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
    // Acknowledged, but the plaintext vault also takes the master password.
    expect(screen.getByTestId('settings-export-run')).toBeDisabled()
    await userEvent.type(screen.getByTestId('settings-export-password'), 'master-pw')
    await userEvent.click(screen.getByTestId('settings-export-run'))

    expect(calls('export_entries')).toContainEqual({
      path: null,
      format: 'cxf',
      password: 'master-pw'
    })
    expect(await screen.findByText(/rowel-export\.json/)).toBeInTheDocument()
    expect(screen.getByTestId('settings-export-password')).toHaveValue('')
  })

  it('says so when the master password is wrong', async () => {
    mockCommand('export_entries', () =>
      Promise.reject({ kind: 'invalidPassword', message: 'invalid password' })
    )
    await exportPane()
    await userEvent.click(screen.getByTestId('settings-export-ack'))
    await userEvent.type(screen.getByTestId('settings-export-password'), 'wrong')
    await userEvent.click(screen.getByTestId('settings-export-run'))

    expect(await screen.findByText('Incorrect Master Password')).toBeInTheDocument()
    expect(screen.queryByText(/Saved to/)).not.toBeInTheDocument()
  })

  it('stays disabled again once the acknowledgement is taken back', async () => {
    await exportPane()
    await userEvent.click(screen.getByTestId('settings-export-ack'))
    await userEvent.type(screen.getByTestId('settings-export-password'), 'master-pw')
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
    // Straight to the backup's own page, not the overview above it.
    expect(screen.getByRole('heading', { name: 'Back up the vault' })).toBeInTheDocument()
    expect(screen.getByTestId('settings-backup-saves')).toBeInTheDocument()
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
const VAULT: RemotePack = {
  id: 'drive-file-1',
  name: '9f3c1a2b4d5e6f708192a3b4c5d6e7f8.rowel',
  vaultId: '9f3c1a2b4d5e6f708192a3b4c5d6e7f8',
  size: 1_258_291,
  modifiedTime: '2024-01-01T00:00:00.000Z',
  provider: 'googleDrive'
}

// Another install's primary in the same account: a second pack, older, so the
// probe lists it after the first.
const OTHER_VAULT: RemotePack = {
  id: 'drive-file-2',
  name: '11223344556677889900112233445566.rowel',
  vaultId: '11223344556677889900112233445566',
  size: 64_512,
  modifiedTime: '2023-06-02T00:00:00.000Z',
  provider: 'googleDrive'
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
    expect(calls('workspace_connect')).toHaveLength(0)
    expect(calls('sync_connect')).toHaveLength(0)
    // It lands like every restore: active, unlocked, re-probed, first sync run.
    await waitFor(() => expect(useApp.getState().flow).toBe('main'))
    expect(calls('sync_now')).toHaveLength(1)
    // The list was the previous workspace's account's; the new one's run says again.
    expect(useApp.getState().remoteVaults).toEqual([])
  })

  // One vault is shown, not offered — under ids of its own, not the probe's
  // picker's.
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

  // A pack sealed under a password other than the vault password is installed
  // closed. The form asks for the vault password, moves the lockbox onto it,
  // and opens it with the switch every restore ends in.
  describe('a vault on a password of its own', () => {
    const offPassword = () =>
      mockCommandOnce('workspace_restore_from_account', () =>
        Promise.reject({
          kind: 'vaultPasswordRequired',
          message: 'this lockbox has a different password; enter the vault password to move it over',
          workspaceId: 'w9'
        })
      )

    const restoreOffPassword = async () => {
      offPassword()
      await open()
      await act(async () => setRemoteVaults([VAULT]))
      await go('workspaces')
      await userEvent.type(screen.getByTestId('workspace-remote-password'), 'its-own-pass')
      await userEvent.click(screen.getByTestId('workspace-remote-submit'))
      return screen.findByTestId('workspace-remote-vault-password')
    }

    it('asks for the vault password, then moves the lockbox and opens it', async () => {
      mockCommandOnce('workspace_select', () => ({ entries: [], syncConfigured: true }))
      const field = await restoreOffPassword()

      expect(screen.getByTestId('workspace-remote-vault-password-why')).toHaveTextContent(
        'Rowel opens every lockbox with your vault password. Enter it to move this lockbox over.'
      )
      expect(screen.getByLabelText('Vault password')).toBe(field)
      // The password that opened it is spent on the move, not retyped.
      expect(screen.getByTestId('workspace-remote-password')).toBeDisabled()

      await userEvent.type(field, 'the-vault-pass')
      await act(async () => {
        await userEvent.click(screen.getByTestId('workspace-remote-submit'))
      })

      expect(calls('workspace_adopt_vault_password')).toEqual([
        { workspaceId: 'w9', lockboxPassword: 'its-own-pass', vaultPassword: 'the-vault-pass' }
      ])
      expect(calls('workspace_select')).toEqual([{ id: 'w9' }])
      await waitFor(() => expect(useApp.getState().flow).toBe('main'))
      expect(calls('workspace_restore_from_account')).toHaveLength(1)
    })

    it('keeps asking when the vault password is wrong', async () => {
      mockCommandOnce('workspace_adopt_vault_password', () =>
        Promise.reject({ kind: 'invalidPassword', message: 'invalid master password' })
      )
      const field = await restoreOffPassword()

      await userEvent.type(field, 'not-it{Enter}')

      expect(await screen.findByText('Incorrect Master Password')).toBeInTheDocument()
      expect(screen.getByTestId('workspace-remote-vault-password')).toBeInTheDocument()
      expect(calls('workspace_select')).toHaveLength(0)
      expect(useUi.getState().settingsLocked).toBe(false)
    })

    // The form is shared by every restore, so the account's picker is held
    // the same way a backup's is: no other vault is picked under a move that
    // ends by opening the lockbox the first one made.
    it('holds the vault picker while the move runs', async () => {
      mockCommandOnce('workspace_adopt_vault_password', () => new Promise(() => {}))
      const field = await restoreOffPassword()
      expect(screen.getByTestId('workspace-remote-target')).not.toHaveAttribute('inert')

      await userEvent.type(field, 'the-vault-pass')
      await userEvent.click(screen.getByTestId('workspace-remote-submit'))

      expect(screen.getByTestId('workspace-remote-submit')).toHaveTextContent('Restoring…')
      expect(screen.getByTestId('workspace-remote-target')).toHaveAttribute('inert')
    })

    // The move re-keys the lockbox for good; a switch that fails after it must
    // not send the spent lockbox password back to the move.
    it('retries only the switch when it fails after the move', async () => {
      mockCommandOnce('workspace_select', () =>
        Promise.reject({ kind: 'syncBusy', message: 'a sync is already running' })
      )
      mockCommandOnce('workspace_select', () => ({ entries: [], syncConfigured: true }))
      const field = await restoreOffPassword()

      await userEvent.type(field, 'the-vault-pass')
      await act(async () => {
        await userEvent.click(screen.getByTestId('workspace-remote-submit'))
      })

      expect(
        await screen.findByText('Wait for the sync in progress to finish')
      ).toBeInTheDocument()
      // The lockbox password is spent: it no longer opens anything.
      expect(screen.getByTestId('workspace-remote-password')).toHaveValue('')
      expect(screen.getByTestId('workspace-remote-submit')).toHaveTextContent('Try again')

      await act(async () => {
        await userEvent.click(screen.getByTestId('workspace-remote-submit'))
      })

      await waitFor(() => expect(useApp.getState().flow).toBe('main'))
      expect(calls('workspace_adopt_vault_password')).toHaveLength(1)
      expect(calls('workspace_select')).toEqual([{ id: 'w9' }, { id: 'w9' }])
    })
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
    expect(screen.queryByTestId('workspace-switch-default')).not.toBeInTheDocument()
    expect(screen.getByTestId('workspace-row-w2')).not.toHaveTextContent('Open now')
    expect(screen.getByTestId('workspace-row-w2')).toHaveTextContent('12 items · Google Drive')

    await userEvent.click(screen.getByTestId('workspace-switch-w2'))
    expect(calls('workspace_select')).toEqual([{ id: 'w2' }])
  })

  it('names the provider a locked workspace syncs to', async () => {
    seedApp({
      workspaces: [
        { id: 'default', name: null },
        { id: 'w2', name: 'Work', synced: true, provider: 'dropbox', itemCount: 4 }
      ],
      activeWorkspace: 'default'
    })
    await open()
    await go('workspaces')

    expect(screen.getByTestId('workspace-row-w2')).toHaveTextContent('4 items · Dropbox')
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

    expect(screen.getByRole('heading', { name: 'Edit lockbox' })).toBeInTheDocument()
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
    expect(screen.getByTestId('workspace-add-row')).toBeInTheDocument()
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
      'You already have a lockbox with this name.'
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
    expect(screen.getByTestId('workspace-add-row')).toBeInTheDocument()
    expect(useUi.getState().settings).toBe(true)
    expect(calls('workspace_rename')).toHaveLength(0)
    expect(calls('workspace_set_color')).toHaveLength(0)
  })
})

describe('Settings › workspaces › new', () => {
  const openNew = async () => {
    await open()
    await go('workspaces')
    await openAdd('new-row')
  }

  const create = () => screen.getByTestId('workspace-create')
  const hint = () => screen.getByTestId('settings-subpage-hint')
  const tile = () => screen.getByTestId('workspace-new-preview').querySelector('[aria-hidden]')

  it('opens the sub-page from the ways to add one and creates one', async () => {
    await openNew()

    expect(screen.getByRole('heading', { name: 'New lockbox' })).toBeInTheDocument()
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
      'You already have a lockbox with this name.'
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

    expect(await screen.findByTestId('workspace-new-error')).toHaveTextContent(
      'Incorrect Master Password'
    )
    expect(calls('workspace_create')).toHaveLength(1)
  })

  it('goes back to the list', async () => {
    await openNew()
    await userEvent.click(screen.getByTestId('settings-subpage-back'))

    expect(screen.queryByTestId('workspace-new-name')).not.toBeInTheDocument()
    expect(screen.getByRole('heading', { name: 'Lockboxes' })).toBeInTheDocument()
    expect(screen.getByTestId('workspace-add-row')).toBeInTheDocument()
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
    expect(screen.getByRole('heading', { name: 'Delete lockbox' })).toBeInTheDocument()
    expect(screen.getByTestId('workspace-delete-preview')).toHaveTextContent('Work')
    expect(screen.queryByTestId('workspace-add-row')).not.toBeInTheDocument()
    expect(
      screen.getByText('This removes the lockbox and everything in it from this device.')
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
      'That is not this lockbox’s master password'
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
      screen.getByText('This removes the lockbox and everything in it from this device.')
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
      'This lockbox was marked as deleted from its account, but it may still be intact.'
    )
    await userEvent.click(screen.getByTestId('workspace-delete-stranded'))

    expect(screen.getByTestId('workspace-delete-preview')).toHaveTextContent('Personal')
    expect(
      screen.queryByTestId('workspace-delete-scope-everywhere')
    ).not.toBeInTheDocument()
  })

  // The marker may be planted, and the copy here is intact either way: the row
  // also offers to put it back, which the backend does by clearing the marker
  // and running a sync.
  it('offers to re-upload a vault the account says was deleted', async () => {
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

    await userEvent.click(screen.getByTestId('workspace-reupload-stranded'))

    expect(calls('sync_reupload')).toHaveLength(1)
  })
})

describe('Settings › workspaces › master password changed elsewhere', () => {
  const stopped = async (errorKind: SyncStatus['errorKind'], error: string) => {
    await open()
    await go('workspaces')
    await act(async () => report({ configured: true, error, errorKind }))
  }

  // Another device re-keyed the vault: the row says so and raises the shared
  // prompt (mounted in `Main`, see passwordChanged.test) for the new password.
  it('offers to enter the new password when it was changed on another device', async () => {
    await stopped(
      'passwordChangedRemotely',
      'the master password of this lockbox was changed on another device'
    )

    expect(screen.getByTestId('workspace-password-changed')).toHaveTextContent(
      'The master password of this lockbox was changed on another device.'
    )
    await userEvent.click(screen.getByTestId('workspace-adopt-password'))

    expect(useUi.getState().passwordPrompt).toBe(true)
  })

  // The user may not have changed anything: then the account's copy may not be
  // theirs, and the prompt's wrong-password copy sends them here to replace it.
  it('also offers to replace the account copy, behind the same confirm', async () => {
    await stopped(
      'passwordChangedRemotely',
      'the master password of this lockbox was changed on another device'
    )

    const row = screen.getByTestId('workspace-password-changed')
    await userEvent.click(within(row).getByTestId('workspace-replace-remote'))

    expect(screen.getByTestId('workspace-replace-remote-sheet')).toHaveTextContent(
      'Edits made only on the other device will be lost.'
    )
    expect(calls('sync_replace_remote')).toHaveLength(0)

    await userEvent.click(screen.getByTestId('workspace-replace-remote-confirm'))

    expect(calls('sync_replace_remote')).toHaveLength(1)
  })

  // The account's copy is under a password this device already retired: the
  // only way out from here overwrites it, so it asks first and names the loss.
  it('replaces the account copy only after a confirm', async () => {
    await stopped(
      'vaultUnderRetiredPassword',
      'the account holds this lockbox under a password this device has already replaced'
    )

    expect(screen.getByTestId('workspace-retired-password')).toHaveTextContent(
      'The account holds this lockbox under a master password this device has already replaced'
    )
    expect(screen.queryByTestId('workspace-adopt-password')).not.toBeInTheDocument()
    await userEvent.click(screen.getByTestId('workspace-replace-remote'))

    expect(screen.getByTestId('workspace-replace-remote-sheet')).toHaveTextContent(
      'Edits made only on the other device will be lost. The other device will be asked for the new password.'
    )
    expect(calls('sync_replace_remote')).toHaveLength(0)

    await userEvent.click(screen.getByTestId('workspace-replace-remote-confirm'))

    expect(calls('sync_replace_remote')).toHaveLength(1)
    expect(screen.queryByTestId('workspace-replace-remote-sheet')).not.toBeInTheDocument()
  })

  it('leaves the account copy alone when the confirm is cancelled', async () => {
    await stopped(
      'vaultUnderRetiredPassword',
      'the account holds this lockbox under a password this device has already replaced'
    )

    await userEvent.click(screen.getByTestId('workspace-replace-remote'))
    await userEvent.click(screen.getByTestId('confirm-cancel'))

    expect(screen.queryByTestId('workspace-replace-remote-sheet')).not.toBeInTheDocument()
    expect(screen.getByTestId('settings-modal')).toBeInTheDocument()
    expect(calls('sync_replace_remote')).toHaveLength(0)
  })
})

describe('Settings › workspaces › restore from an account', () => {
  // The flow is a page under Lockboxes, one of the ways to add one.
  const openRestore = async () => {
    await open()
    await go('workspaces')
    await openAdd('restore-open')
  }

  // Consent, then the probe's answer — which arrives as an event, never through
  // the connect's promise.
  const connect = async (files: RemotePack[]) => {
    await openRestore()
    await userEvent.click(screen.getByTestId('workspace-restore-drive-connect'))
    await act(async () => setupAccountProbed(files))
  }

  // On the Lockboxes page the account's Connect buttons would read as
  // connecting this vault, which is what the same buttons do under Sync &
  // backup; there the page's title says what they are for.
  it('keeps the sign-in on a page of its own, away from the list', async () => {
    await open()
    await go('workspaces')

    expect(screen.queryAllByTestId(/^workspace-restore-.+-connect$/)).toHaveLength(0)
    await userEvent.click(screen.getByTestId('workspace-add-row'))
    expect(screen.queryAllByTestId(/^workspace-restore-.+-connect$/)).toHaveLength(0)
    expect(screen.getByTestId('workspace-restore-open')).toHaveTextContent(
      'Restore from a cloud account'
    )

    await userEvent.click(screen.getByTestId('workspace-restore-open'))
    expect(
      screen.getByRole('heading', { name: 'Restore from a cloud account' })
    ).toBeInTheDocument()
    // The choice of a provider is the list Sync & backup asks with: a row per
    // provider, named, each with its own Connect.
    const providers = screen.getByTestId('workspace-restore-row')
    expect(providers).toHaveTextContent('Choose a provider')
    expect(within(providers).getByText('Google Drive', { selector: 'span' })).toBeInTheDocument()
    expect(screen.getByTestId('workspace-restore-drive-connect')).toHaveAccessibleName(
      'Connect Google Drive'
    )
    expect(screen.getByTestId('workspace-restore-dropbox-connect')).toHaveAccessibleName(
      'Connect Dropbox'
    )
    expect(screen.queryByTestId('workspace-restore-open')).not.toBeInTheDocument()
    // The title and its description are said once, by the page's header.
    expect(screen.getAllByText('Restore from a cloud account')).toHaveLength(1)
    expect(screen.getAllByText(/^Sign in and pick one of the lockboxes/)).toHaveLength(1)

    await userEvent.click(screen.getByTestId('settings-subpage-back'))
    expect(screen.getByRole('heading', { name: 'Lockboxes' })).toBeInTheDocument()
    expect(screen.getByTestId('workspace-add-row')).toBeInTheDocument()
  })

  // Going Back is backing out, the same as Cancel: the pending tokens go too.
  it('forgets the connected account when the page is left', async () => {
    await connect([VAULT])

    await userEvent.click(screen.getByTestId('settings-subpage-back'))

    expect(calls('setup_disconnect')).toHaveLength(1)
    expect(useApp.getState().setupAccount.status).toBe('idle')
  })

  // With no sign-in started there is nothing to forget, so the backend is not
  // asked to.
  it('leaves without a backend call when no sign-in was started', async () => {
    await openRestore()

    await userEvent.click(screen.getByTestId('settings-subpage-back'))

    expect(calls('setup_disconnect')).toHaveLength(0)
  })

  it('restores the vault the user picked as a workspace of its own', async () => {
    await connect([VAULT, OTHER_VAULT])

    expect(calls('workspace_connect')).toHaveLength(1)
    // Two vaults is a choice, so both are offered rather than one picked.
    expect(screen.getByTestId('drive-vault-drive-file-1')).toBeInTheDocument()
    await userEvent.click(screen.getByTestId('drive-vault-drive-file-2'))

    await userEvent.type(screen.getByTestId('workspace-restore-name'), 'Work')
    await userEvent.type(screen.getByTestId('workspace-restore-password'), 'other-device-pass')
    await act(async () => {
      await userEvent.click(screen.getByTestId('workspace-restore-submit'))
    })

    expect(calls('workspace_restore_from_pending')).toEqual([
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

    expect(calls('workspace_restore_from_pending')).toEqual([
      { name: '', password: 'other-device-pass', fileId: VAULT.id }
    ])
  })

  // The same form, after a sign-in: a pack on a password of its own asks for
  // the vault password, and the restore ends in the switch to the moved lockbox.
  it('moves a vault on a password of its own onto the vault password', async () => {
    mockCommandOnce('workspace_restore_from_pending', () =>
      Promise.reject({
        kind: 'vaultPasswordRequired',
        message: 'this lockbox has a different password; enter the vault password to move it over',
        workspaceId: 'w9'
      })
    )
    mockCommandOnce('workspace_select', () => ({ entries: [], syncConfigured: true }))
    await connect([VAULT])

    await userEvent.type(screen.getByTestId('workspace-restore-password'), 'its-own-pass{Enter}')
    const field = await screen.findByTestId('workspace-restore-vault-password')
    // The form is back in the user's hands, waiting on the second password.
    expect(useApp.getState().setupAccount.status).toBe('found')
    await userEvent.type(field, 'the-vault-pass')
    await act(async () => {
      await userEvent.click(screen.getByTestId('workspace-restore-submit'))
    })

    expect(calls('workspace_adopt_vault_password')).toEqual([
      { workspaceId: 'w9', lockboxPassword: 'its-own-pass', vaultPassword: 'the-vault-pass' }
    ])
    await waitFor(() => expect(useApp.getState().flow).toBe('main'))
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

    expect(calls('setup_disconnect')).toHaveLength(1)
    expect(useApp.getState().setupAccount.status).toBe('idle')
    expect(screen.getByTestId('workspace-restore-drive-connect')).toBeInTheDocument()
  })

  // Once the restore is running the backend holds the account for its length
  // and would refuse to drop it, so nothing on screen may offer to: a Cancel
  // that did nothing would read as a cancellation that happened.
  it('withdraws Cancel and Switch account while the restore runs', async () => {
    mockCommandOnce('workspace_restore_from_pending', () => new Promise(() => {}))
    await connect([VAULT])

    await userEvent.type(screen.getByTestId('workspace-restore-name'), 'Work')
    await userEvent.type(screen.getByTestId('workspace-restore-password'), 'pass')
    await userEvent.click(screen.getByTestId('workspace-restore-submit'))

    expect(useApp.getState().setupAccount.status).toBe('restoring')
    expect(screen.queryByTestId('workspace-restore-cancel')).not.toBeInTheDocument()
    expect(screen.queryByTestId('workspace-restore-switch-account')).not.toBeInTheDocument()
    expect(screen.getByTestId('workspace-restore-submit')).toHaveTextContent('Restoring…')
    expect(calls('setup_disconnect')).toHaveLength(0)
  })

  // Leaving Settings would unmount the row and run the same refused disconnect,
  // then the restore would finish and switch workspaces behind the user's back.
  // So Settings stays shut on its section — from the X, from the nav, and from
  // anything else that reaches the store — until the restore lands.
  it('holds Settings on the section while the restore runs', async () => {
    mockCommandOnce('workspace_restore_from_pending', () => new Promise(() => {}))
    await connect([VAULT])

    await userEvent.type(screen.getByTestId('workspace-restore-name'), 'Work')
    await userEvent.type(screen.getByTestId('workspace-restore-password'), 'pass')
    await userEvent.click(screen.getByTestId('workspace-restore-submit'))

    expect(screen.getByTestId('modal-close')).toBeDisabled()
    expect(screen.getByTestId('settings-nav-sync')).toBeDisabled()
    // The page is held too: its Back is inert and Escape does not step out.
    expect(screen.getByTestId('settings-subpage-back')).toBeDisabled()
    await userEvent.keyboard('{Escape}')
    expect(
      screen.getByRole('heading', { name: 'Restore from a cloud account' })
    ).toBeInTheDocument()
    // The store refuses too, whoever asks.
    closeSettings()
    setSettingsSection('sync')
    expect(useUi.getState().settings).toBe(true)
    expect(useUi.getState().settingsSection).toBe('workspaces')
  })

  // A failed restore gives Settings back with the form.
  it('releases Settings when the restore fails', async () => {
    mockCommandOnce('workspace_restore_from_pending', () =>
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
    mockCommandOnce('workspace_restore_from_pending', () =>
      Promise.reject({ kind: 'invalidPassword', message: 'invalid master password' })
    )
    await connect([VAULT])

    await userEvent.type(screen.getByTestId('workspace-restore-name'), 'Work')
    await userEvent.type(screen.getByTestId('workspace-restore-password'), 'wrong')
    await userEvent.click(screen.getByTestId('workspace-restore-submit'))

    await waitFor(() => expect(useApp.getState().setupAccount.status).toBe('found'))
    expect(screen.getByTestId('workspace-restore-cancel')).toBeInTheDocument()
    expect(screen.getByTestId('workspace-restore-switch-account')).toBeInTheDocument()
  })

  it('blames the password only when the backend does', async () => {
    mockCommandOnce('workspace_restore_from_pending', () =>
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
    mockCommandOnce('workspace_restore_from_pending', () =>
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
      await screen.findByText('This lockbox is already on this device')
    ).toBeInTheDocument()
  })

  it('reports a consent that never came back', async () => {
    await openRestore()
    await userEvent.click(screen.getByTestId('workspace-restore-drive-connect'))
    expect(screen.getByTestId('workspace-restore-waiting')).toBeInTheDocument()

    await act(async () => setupAccountFailed('access_denied'))

    expect(await screen.findByTestId('workspace-restore-error')).toHaveTextContent(
      'access_denied'
    )
    expect(calls('workspace_restore_from_pending')).toHaveLength(0)
  })

  // The page starts with the choice of provider under its own title, and
  // everything after the press names the one picked.
  it('restores from Dropbox, naming it from the press on', async () => {
    await openRestore()
    expect(
      screen.getByRole('heading', { name: 'Restore from a cloud account' })
    ).toBeInTheDocument()
    expect(screen.getByTestId('workspace-restore-row')).not.toHaveTextContent('Restore from')

    await userEvent.click(screen.getByTestId('workspace-restore-dropbox-connect'))

    expect(calls('workspace_connect')).toEqual([{ provider: 'dropbox' }])
    expect(screen.getByTestId('workspace-restore-row')).toHaveTextContent('Restore from Dropbox')
    expect(screen.getByTestId('workspace-restore-waiting')).toHaveTextContent(
      'Waiting for Dropbox…'
    )

    await act(async () => setupAccountProbed([]))
    expect(screen.getByTestId('workspace-restore-empty')).toHaveTextContent(
      'This Dropbox account has nothing to restore'
    )
  })
})

// A `.rowel` saved from Settings, restored as a lockbox beside the ones here:
// the account restore's form, with a file where the account's vaults were.
describe('Settings › workspaces › restore a backup', () => {
  const BACKUP = '/tmp/Rowel backup 2026-11-20.rowel'

  const openPage = async () => {
    await open()
    await go('workspaces')
    await openAdd('backup-open')
  }

  const pickAndType = async (password: string) => {
    mockCommand('pick_file', () => BACKUP)
    await openPage()
    await userEvent.click(screen.getByTestId('workspace-backup-pick'))
    await userEvent.type(await screen.findByTestId('workspace-backup-password'), password)
  }

  // The third way to one more lockbox, under the other two.
  it('opens its page from the last of the ways to add one', async () => {
    await open()
    await go('workspaces')
    await userEvent.click(screen.getByTestId('workspace-add-row'))

    const row = screen.getByTestId('workspace-backup-open')
    expect(row).toHaveTextContent('Restore from a backup')
    expect(row.parentElement?.lastElementChild).toBe(row)
    expect(row.previousElementSibling).toBe(screen.getByTestId('workspace-restore-open'))

    await userEvent.click(row)
    expect(screen.getByRole('heading', { name: 'Restore from a backup' })).toBeInTheDocument()
    expect(screen.getAllByText(/^Pick a \.rowel file saved from Settings/)).toHaveLength(1)
    // Nothing to restore until a file is chosen.
    expect(screen.queryByTestId('workspace-backup-password')).not.toBeInTheDocument()
  })

  it('restores the picked file as a lockbox of its own', async () => {
    mockCommand('pick_file', () => BACKUP)
    await openPage()

    await userEvent.click(screen.getByTestId('workspace-backup-pick'))
    // Picked in Rust, which grants the path for the restore that reads it;
    // offered `.rowel` alone, since a legacy vault is not a lockbox.
    expect(calls('pick_file')).toEqual([{ kind: 'lockboxBackup', label: 'Rowel backup' }])
    expect(await screen.findByTestId('workspace-backup-file')).toHaveTextContent(
      'Rowel backup 2026-11-20.rowel'
    )
    await userEvent.type(screen.getByTestId('workspace-backup-name'), 'Old laptop')
    await userEvent.type(screen.getByTestId('workspace-backup-password'), 'backup-pass')
    await act(async () => {
      await userEvent.click(screen.getByTestId('workspace-backup-submit'))
    })

    expect(calls('workspace_restore_from_file')).toEqual([
      { name: 'Old laptop', password: 'backup-pass', path: BACKUP }
    ])
    // It lands like every restore — active, unlocked, re-probed — but with no
    // account, so no first sync runs.
    await waitFor(() => expect(useApp.getState().flow).toBe('main'))
    expect(calls('app_status').length).toBeGreaterThan(0)
    expect(calls('sync_now')).toHaveLength(0)
  })

  it('hands a wrong password back to the form, with the file still chosen', async () => {
    mockCommandOnce('workspace_restore_from_file', () =>
      Promise.reject({ kind: 'invalidPassword', message: 'invalid master password' })
    )
    await pickAndType('wrong')
    await userEvent.click(screen.getByTestId('workspace-backup-submit'))

    expect(await screen.findByText('Invalid password for backup')).toBeInTheDocument()
    expect(screen.getByTestId('workspace-backup-submit')).toHaveTextContent('Restore')
    expect(screen.getByTestId('workspace-backup-file')).toBeInTheDocument()
    expect(screen.getByTestId('workspace-backup-password')).toHaveValue('wrong')
    expect(screen.getByTestId('workspace-backup-pick')).toHaveTextContent('Pick another file')
    expect(useUi.getState().settingsLocked).toBe(false)
    expect(useApp.getState().flow).not.toBe('main')
  })

  // The restore ends by switching lockboxes, so nothing may leave the page
  // while it runs — and the file it is reading is not swapped from under it.
  it('holds Settings while the restore runs', async () => {
    mockCommandOnce('workspace_restore_from_file', () => new Promise(() => {}))
    await pickAndType('backup-pass')
    await userEvent.click(screen.getByTestId('workspace-backup-submit'))

    expect(useUi.getState().settingsLocked).toBe(true)
    expect(screen.getByTestId('workspace-backup-submit')).toHaveTextContent('Restoring…')
    expect(screen.queryByTestId('workspace-backup-pick')).not.toBeInTheDocument()
    expect(screen.getByTestId('modal-close')).toBeDisabled()
    expect(screen.getByTestId('settings-subpage-back')).toBeDisabled()
    await userEvent.keyboard('{Escape}')
    expect(screen.getByRole('heading', { name: 'Restore from a backup' })).toBeInTheDocument()
  })

  it('says so when the backup is of a lockbox already here', async () => {
    mockCommandOnce('workspace_restore_from_file', () =>
      Promise.reject({ kind: 'vaultAlreadyOpen', message: 'this vault is already here' })
    )
    await pickAndType('backup-pass')
    await userEvent.click(screen.getByTestId('workspace-backup-submit'))

    expect(await screen.findByText('This lockbox is already on this device')).toBeInTheDocument()
  })

  // A backup sealed under another password is installed closed, and the same
  // second step as an account restore's moves it onto the vault password.
  it('moves a backup on a password of its own onto the vault password', async () => {
    mockCommandOnce('workspace_restore_from_file', () =>
      Promise.reject({
        kind: 'vaultPasswordRequired',
        message: 'this lockbox has a different password; enter the vault password to move it over',
        workspaceId: 'w9'
      })
    )
    mockCommandOnce('workspace_select', () => ({ entries: [], syncConfigured: false }))
    await pickAndType('its-own-pass{Enter}')

    const field = await screen.findByTestId('workspace-backup-vault-password')
    await userEvent.type(field, 'the-vault-pass')
    await act(async () => {
      await userEvent.click(screen.getByTestId('workspace-backup-submit'))
    })

    expect(calls('workspace_adopt_vault_password')).toEqual([
      { workspaceId: 'w9', lockboxPassword: 'its-own-pass', vaultPassword: 'the-vault-pass' }
    ])
    expect(calls('workspace_select')).toEqual([{ id: 'w9' }])
    expect(calls('workspace_restore_from_file')).toHaveLength(1)
  })

  // The move and the open after it are steps of the same restore: they end by
  // opening the lockbox the first file made, so no other file is picked under
  // them, and none while the lockbox waits, closed, for the vault password.
  it('offers no other file while the move onto the vault password runs', async () => {
    mockCommandOnce('workspace_restore_from_file', () =>
      Promise.reject({
        kind: 'vaultPasswordRequired',
        message: 'this lockbox has a different password; enter the vault password to move it over',
        workspaceId: 'w9'
      })
    )
    mockCommandOnce('workspace_adopt_vault_password', () => new Promise(() => {}))
    await pickAndType('its-own-pass{Enter}')

    const field = await screen.findByTestId('workspace-backup-vault-password')
    expect(screen.queryByTestId('workspace-backup-pick')).not.toBeInTheDocument()

    await userEvent.type(field, 'the-vault-pass')
    await userEvent.click(screen.getByTestId('workspace-backup-submit'))

    expect(screen.getByTestId('workspace-backup-submit')).toHaveTextContent('Restoring…')
    expect(screen.queryByTestId('workspace-backup-pick')).not.toBeInTheDocument()
    expect(screen.getByTestId('workspace-backup-target')).toHaveAttribute('inert')
    expect(screen.getByTestId('workspace-backup-file')).toHaveTextContent(
      'Rowel backup 2026-11-20.rowel'
    )
  })
})

describe('Settings sub-pages', () => {
  const openSubpage = async () => {
    await open()
    await go('workspaces')
    await openAdd('new-row')
  }

  const onSection = () => {
    expect(screen.getByRole('heading', { name: 'Lockboxes' })).toBeInTheDocument()
    expect(screen.queryByTestId('settings-subpage-back')).not.toBeInTheDocument()
    expect(screen.getByTestId('workspace-add-row')).toBeInTheDocument()
  }

  const ways = () =>
    ['new-row', 'restore-open', 'backup-open'].map(way => screen.queryByTestId(`workspace-${way}`))

  // The list's one way to one more lockbox, and the last row of its card. The
  // three ways there are stay out of the list, in the menu the row opens, and
  // the menu is not a page: the section stays where it is under it.
  it('offers one way to add a lockbox under the list, with the ways in its menu', async () => {
    await open()
    await go('workspaces')

    const row = screen.getByTestId('workspace-add-row')
    expect(row).toHaveTextContent('Add a lockbox')
    expect(row.parentElement?.parentElement?.lastElementChild).toBe(row.parentElement)
    expect(row).toHaveAttribute('aria-expanded', 'false')
    for (const way of ways()) expect(way).not.toBeInTheDocument()

    await userEvent.click(row)
    expect(row).toHaveAttribute('aria-expanded', 'true')
    expect(ways().map(way => way?.textContent)).toEqual([
      expect.stringMatching(/^New lockbox/),
      expect.stringMatching(/^Restore from a cloud account/),
      expect.stringMatching(/^Restore from a backup/)
    ])
    expect(screen.getByRole('heading', { name: 'Lockboxes' })).toBeInTheDocument()
    expect(screen.queryByTestId('settings-subpage-back')).not.toBeInTheDocument()

    // Escape is the menu's first: it closes, and Settings stays open under it.
    await userEvent.keyboard('{Escape}')
    for (const way of ways()) expect(way).not.toBeInTheDocument()
    expect(useUi.getState().settings).toBe(true)
    onSection()
  })

  it('heads the sub-page in the section’s place and comes back from it', async () => {
    await openSubpage()

    // The title takes the section's line: one heading, nothing stacked above
    // it, so the header keeps its height on the way in and out.
    expect(screen.getAllByRole('heading', { level: 1 })).toHaveLength(1)
    expect(screen.getByRole('heading', { name: 'New lockbox' })).toBeInTheDocument()
    expect(
      screen.getByText('Its own encrypted database, unlocked alongside your others.')
    ).toBeInTheDocument()
    // It replaces the section's body rather than stacking on it.
    expect(screen.queryByTestId('workspace-add-row')).not.toBeInTheDocument()
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
    expect(screen.getByRole('heading', { name: 'New lockbox' })).toBeInTheDocument()
    expect(useUi.getState().settings).toBe(true)
  })

  // `openSettings` comes from outside the shell — a backup the OS opened asks
  // for Import — and must not take a form away whose operation is still out.
  it('keeps a locked sub-page when something outside asks for another section', async () => {
    await openSubpage()
    act(() => lockSettings(true))

    act(() => openSettings('import'))
    expect(screen.getByRole('heading', { name: 'New lockbox' })).toBeInTheDocument()
    expect(useUi.getState().settingsSection).toBe('workspaces')

    // The section asked for is gone to once the operation lets go.
    act(() => lockSettings(false))
    expect(screen.getByRole('heading', { name: 'Import & export' })).toBeInTheDocument()
    expect(screen.queryByTestId('settings-subpage-back')).not.toBeInTheDocument()
  })
})

// The command line tool row on General: three states, all Rust's to say.
describe('Settings › General › command line tool', () => {
  const BUNDLED = '/Applications/Rowel.app/Contents/MacOS/rowel-cli'
  const LINK = '/usr/local/bin/rowel'
  const bundled = { bundled: BUNDLED, installed: null, installDir: '/usr/local/bin', onPath: true }
  const linked = { ...bundled, installed: LINK }

  it('says the tool ships with the packaged app when none is bundled', async () => {
    await open()
    expect(await screen.findByTestId('settings-cli-missing')).toHaveTextContent(
      'ships with the packaged app'
    )
    expect(screen.queryByTestId('settings-cli-install')).not.toBeInTheDocument()
    expect(screen.queryByTestId('settings-cli-path')).not.toBeInTheDocument()
  })

  it('offers Install for a bundled tool, and shows where it went', async () => {
    mockCommand('cli_status', () => bundled)
    mockCommand('cli_install', () => linked)
    await open()
    expect(await screen.findByTestId('settings-cli-bundled')).toHaveTextContent('rowel command')
    expect(screen.getByTestId('settings-cli-path')).toHaveTextContent(BUNDLED)

    await userEvent.click(screen.getByTestId('settings-cli-install'))

    expect(calls('cli_install')).toHaveLength(1)
    expect(await screen.findByTestId('settings-cli-installed')).toHaveTextContent(
      `Installed as ${LINK}`
    )
    expect(screen.getByTestId('settings-cli-installed')).not.toHaveTextContent('to your PATH')
    expect(screen.getByTestId('settings-cli-path')).toHaveTextContent(LINK)
    expect(screen.queryByTestId('settings-cli-install')).not.toBeInTheDocument()
  })

  // The path an agent's MCP config names is there to copy on every platform,
  // Windows's backslashes and all.
  it('copies the full path for an MCP config', async () => {
    const exe = 'C:\\Users\\ada\\AppData\\Local\\Rowel\\rowel-cli.exe'
    mockCommand('cli_status', () => ({ ...bundled, bundled: exe, installDir: null }))
    await open()
    await userEvent.click(await screen.findByTestId('settings-cli-copy'))
    await waitFor(() =>
      expect(calls('copy_to_clipboard')).toContainEqual(expect.objectContaining({ value: exe }))
    )
  })

  it('says to add a directory that is not on the PATH', async () => {
    const own = '/Users/ada/.local/bin'
    mockCommand('cli_status', () => ({
      ...bundled,
      installed: `${own}/rowel`,
      installDir: own,
      onPath: false
    }))
    await open()
    expect(await screen.findByTestId('settings-cli-installed')).toHaveTextContent(
      `add ${own} to your PATH`
    )
  })

  it('removes the command again', async () => {
    mockCommand('cli_status', () => linked)
    mockCommand('cli_uninstall', () => bundled)
    await open()
    expect(await screen.findByTestId('settings-cli-installed')).toHaveTextContent(LINK)

    await userEvent.click(screen.getByTestId('settings-cli-uninstall'))

    expect(calls('cli_uninstall')).toHaveLength(1)
    expect(await screen.findByTestId('settings-cli-bundled')).toBeInTheDocument()
    expect(screen.getByTestId('settings-cli-install')).toBeEnabled()
  })

  it('reports an install Rust refused and keeps the button', async () => {
    mockCommand('cli_status', () => bundled)
    mockCommand('cli_install', () =>
      Promise.reject({ kind: 'other', message: '/usr/local/bin/rowel exists and is not a link' })
    )
    await open()
    await userEvent.click(await screen.findByTestId('settings-cli-install'))

    expect(await screen.findByTestId('settings-cli-error')).toHaveTextContent('is not a link')
    expect(screen.getByTestId('settings-cli-install')).toBeEnabled()
  })
})
