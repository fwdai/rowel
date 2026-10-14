import { describe, it, expect, beforeEach, vi } from 'vitest'
import { act, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import Start from '@/components/Start'
import type { RemotePack } from '@/api/setup'
import { open } from '@tauri-apps/plugin-dialog'
import { fileOpened, setupAccountProbed, setupAccountFailed, useApp } from '@/store'
import { evaluate, MIN_LENGTH, type Strength } from '@/services/strength'
import { calls, mockCommandOnce } from './ipc'
import { deferred, seedApp } from './utils'

// The real `evaluate` awaits an 848 kB zxcvbn chunk. The specs below have to
// hold that await open, and in one case fail it, so the scoring itself is stood
// in for — length is the only thing that separates the passwords used here.
vi.mock('@/services/strength', async importOriginal => ({
  ...(await importOriginal<typeof import('@/services/strength')>()),
  evaluate: vi.fn()
}))

const scored = (password: string): Strength => ({
  score: password.length >= MIN_LENGTH ? 4 : 0,
  warning: '',
  suggestions: [],
  tooShort: password.length < MIN_LENGTH,
  acceptable: password.length >= MIN_LENGTH
})

// Whether the device could enroll a biometric gate at all, which is what
// decides if the first run asks its last question.
const biometricStatus = (canEnroll: boolean) =>
  seedApp({ biometric: { available: false, canEnroll, type: 'touch', mode: null } })

// Must satisfy the setup strength gate (>= 12 chars, zxcvbn score >= 2).
const STRONG = 'my-strong-vault-passphrase-2026'

// What the probe reports for a pack in `Rowel/Vaults/`: the file name is the
// vault's own id, which says nothing to a user, so the screens title it
// "Rowel lockbox" and let the meta line do the distinguishing.
const REMOTE: RemotePack = {
  id: 'drive-file-1',
  name: '9f3c1a2b4d5e6f708192a3b4c5d6e7f8.rowel',
  vaultId: '9f3c1a2b4d5e6f708192a3b4c5d6e7f8',
  size: 1_258_291,
  modifiedTime: '2024-01-01T00:00:00.000Z',
  provider: 'googleDrive'
}

// A second install synced its own primary to the same account: another vault,
// another id, and older — so the probe lists it after `REMOTE`.
const OTHER: RemotePack = {
  id: 'drive-file-2',
  name: '11223344556677889900112233445566.rowel',
  vaultId: '11223344556677889900112233445566',
  size: 64_512,
  modifiedTime: '2023-06-02T00:00:00.000Z',
  provider: 'googleDrive'
}

beforeEach(() => {
  vi.clearAllMocks()
  vi.mocked(evaluate).mockImplementation(async password => scored(password))
})

// Welcome -> the merged password screen, filled in and submitted.
const choosePassword = async (password = STRONG, confirmation = password) => {
  await userEvent.click(screen.getByTestId('start-setup-button'))
  await userEvent.type(screen.getByTestId('setup-password-input'), password)
  await userEvent.type(screen.getByTestId('setup-confirm-password-input'), confirmation)
  await userEvent.click(screen.getByTestId('setup-continue-button'))
}

describe('welcome', () => {
  it('offers a fresh start and both ways back into existing data', () => {
    render(<Start />)

    expect(screen.getByText('Keep your secrets to yourself.')).toBeInTheDocument()
    expect(screen.getByTestId('start-setup-button')).toBeInTheDocument()
    expect(screen.getByTestId('start-drive-button')).toBeInTheDocument()
    expect(screen.getByTestId('start-dropbox-button')).toBeInTheDocument()
    expect(screen.getByTestId('start-restore-button')).toBeInTheDocument()
  })

  // One button per provider, each starting the consent with its own: the
  // backend has no other way to know whose sign-in page to open.
  it.each([
    ['start-drive-button', 'googleDrive'],
    ['start-dropbox-button', 'dropbox']
  ])('starts the connect %s names', async (testid, provider) => {
    render(<Start />)
    expect(screen.getByTestId(testid)).toHaveTextContent(
      provider === 'dropbox' ? 'Continue with Dropbox' : 'Continue with Google Drive'
    )

    await userEvent.click(screen.getByTestId(testid))

    expect(calls('setup_connect')).toEqual([{ provider }])
    expect(useApp.getState().setupAccount).toMatchObject({ status: 'pending', provider })
  })

  // The mascot belongs to the flow, not the screen: the same element stays
  // mounted from step to step and brightens as the steps go by.
  it('keeps one mascot through the steps and brightens it', async () => {
    render(<Start />)
    const mascot = screen.getByTestId('lock-mascot')
    expect(mascot).toHaveAttribute('data-joy', '0')

    await userEvent.click(screen.getByTestId('start-setup-button'))
    expect(screen.getByTestId('lock-mascot')).toBe(mascot)
    expect(mascot).toHaveAttribute('data-joy', '0.5')

    await userEvent.click(screen.getByTestId('go-back-button'))
    expect(screen.getByTestId('lock-mascot')).toBe(mascot)
    expect(mascot).toHaveAttribute('data-joy', '0')
  })

  // The footer names the vault about to open. There is none yet, so the first
  // run draws no footer on any of its screens — the lock screen keeps it.
  it('draws no footer strip during the first run', async () => {
    render(<Start />)
    expect(screen.queryByTestId('app-version')).not.toBeInTheDocument()

    await userEvent.click(screen.getByTestId('start-setup-button'))
    expect(screen.queryByTestId('app-version')).not.toBeInTheDocument()
    expect(screen.getByTestId('go-back-button')).toBeInTheDocument()
  })

  // A phone has no file system to hand a `.swftx` export from, so the option
  // that depends on one is not drawn there at all.
  it('hides the backup-file option on mobile', async () => {
    vi.resetModules()
    vi.doMock('@/lib/platform', () => ({ isIOS: true, isAndroid: false, isMobile: true }))
    try {
      const { default: Welcome } = await import('@/components/Start/Welcome')
      render(<Welcome onFresh={() => {}} onConnect={() => {}} onFile={() => {}} />)

      expect(screen.getByTestId('start-drive-button')).toBeInTheDocument()
      expect(screen.getByTestId('start-dropbox-button')).toBeInTheDocument()
      expect(screen.queryByTestId('start-restore-button')).not.toBeInTheDocument()
    } finally {
      vi.doUnmock('@/lib/platform')
      vi.resetModules()
    }
  })
})

describe('choosing a master password', () => {
  // The confirmation is a check on a password worth keeping, so it unfolds
  // only once there is one — and stays once it has.
  it('asks for the confirmation once the password is long enough', async () => {
    render(<Start />)
    await userEvent.click(screen.getByTestId('start-setup-button'))
    const input = screen.getByTestId('setup-password-input')

    await userEvent.type(input, 'a'.repeat(MIN_LENGTH - 1))
    expect(screen.queryByTestId('setup-confirm-password-input')).not.toBeInTheDocument()

    await userEvent.type(input, 'a')
    expect(screen.getByTestId('setup-confirm-password-input')).toBeInTheDocument()

    await userEvent.type(input, '{backspace}{backspace}')
    expect(screen.getByTestId('setup-confirm-password-input')).toBeInTheDocument()
  })

  // Too short to confirm: Continue says so on the strength line, once, and
  // the flow goes nowhere.
  it('blocks a weak password without reaching the next step', async () => {
    render(<Start />)
    await userEvent.click(screen.getByTestId('start-setup-button'))
    await userEvent.type(screen.getByTestId('setup-password-input'), 'secret')
    await userEvent.click(screen.getByTestId('setup-continue-button'))

    expect(await screen.findAllByText(/Use at least/)).toHaveLength(1)
    expect(screen.getByTestId('setup-password-input')).toBeInTheDocument()
    expect(screen.queryByTestId('setup-confirm-password-input')).not.toBeInTheDocument()
    expect(screen.queryByTestId('setup-skip-drive-button')).not.toBeInTheDocument()
  })

  it('refuses a confirmation that does not match', async () => {
    render(<Start />)
    await choosePassword(STRONG, 'something-else-entirely')

    expect(await screen.findByText('Passwords do not match')).toBeInTheDocument()
    expect(calls('setup_create')).toHaveLength(0)
  })

  // Both cards go inert while the check is out. Focus comes back to the one
  // the user was typing in, never to the other. (Submitted with Enter: a
  // click on Continue moves focus to the button on some platforms, and then
  // neither card has it to get back.)
  it('returns focus to the field that had it once the check lands', async () => {
    const check = deferred<Strength>()
    vi.mocked(evaluate).mockReturnValue(check.promise)
    render(<Start />)
    await userEvent.click(screen.getByTestId('start-setup-button'))
    await userEvent.type(screen.getByTestId('setup-password-input'), STRONG)
    const confirm = screen.getByTestId('setup-confirm-password-input')
    await userEvent.type(confirm, 'something-else-entirely{enter}')

    expect(confirm).toBeDisabled()
    await act(async () => check.resolve(scored(STRONG)))

    expect(await screen.findByText('Passwords do not match')).toBeInTheDocument()
    expect(confirm).toHaveFocus()
    expect(screen.getByTestId('setup-password-input')).not.toHaveFocus()
  })

  // One verdict at a time: a mismatch is about the pair, so editing the
  // password retires it, and a strength error never lands beside it.
  it('drops a stale mismatch once the password is edited', async () => {
    render(<Start />)
    await choosePassword(STRONG, 'something-else-entirely')
    await screen.findByText('Passwords do not match')

    await userEvent.clear(screen.getByTestId('setup-password-input'))
    await userEvent.type(screen.getByTestId('setup-password-input'), 'secret')
    await userEvent.click(screen.getByTestId('setup-continue-button'))

    expect(await screen.findAllByTestId('form-error')).toHaveLength(1)
    expect(screen.getByTestId('form-error')).toHaveTextContent(/Use at least/)
    expect(screen.queryByText('Passwords do not match')).not.toBeInTheDocument()
  })

  // Going back unmounts this screen. The check landing afterwards must not
  // carry the abandoned password on into the flow behind it.
  it('does not create the vault when the user goes back while the check is pending', async () => {
    const check = deferred<Strength>()
    vi.mocked(evaluate).mockReturnValue(check.promise)
    render(<Start />)
    await choosePassword()

    await userEvent.click(screen.getByTestId('go-back-button'))
    await act(async () => check.resolve(scored(STRONG)))

    expect(screen.getByTestId('start-setup-button')).toBeInTheDocument()
    expect(calls('setup_create')).toHaveLength(0)
  })

  // The screen holds still while the check is out: what it answers about has to
  // still be what is on screen — and what gets created — when it lands.
  it('refuses input while the check is pending', async () => {
    const check = deferred<Strength>()
    vi.mocked(evaluate).mockReturnValue(check.promise)
    render(<Start />)
    await choosePassword()

    expect(screen.getByTestId('setup-password-input')).toBeDisabled()
    expect(screen.getByTestId('setup-confirm-password-input')).toBeDisabled()

    await act(async () => check.resolve(scored(STRONG)))
    await userEvent.click(await screen.findByTestId('setup-skip-drive-button'))

    expect(calls('setup_create')).toContainEqual({ password: STRONG })
  })

  // A chunk that will not load leaves nothing to wait for: say so, and take
  // another press, rather than spinning on it forever.
  it('recovers when the strength engine fails to load', async () => {
    render(<Start />)
    await userEvent.click(screen.getByTestId('start-setup-button'))
    await userEvent.type(screen.getByTestId('setup-password-input'), STRONG)
    await userEvent.type(screen.getByTestId('setup-confirm-password-input'), STRONG)
    // Only the submit's own check fails; the meter has already been scored.
    await screen.findByText('Very strong')
    vi.mocked(evaluate).mockRejectedValue(new Error('failed to fetch dynamically imported module'))

    await userEvent.click(screen.getByTestId('setup-continue-button'))

    expect(await screen.findByText('Something went wrong')).toBeInTheDocument()
    expect(screen.getByTestId('setup-continue-button')).toBeEnabled()
  })
})

describe('the backup step', () => {
  it('creates local-only data when the backup is declined', async () => {
    render(<Start />)
    await choosePassword()

    await userEvent.click(await screen.findByTestId('setup-skip-drive-button'))

    expect(calls('setup_create')).toContainEqual({ password: STRONG })
    await waitFor(() => expect(useApp.getState().flow).toBe('main'))
  })

  it('creates with sync on once the probe finds the account bare', async () => {
    render(<Start />)
    await choosePassword()

    await userEvent.click(await screen.findByTestId('setup-drive-connect'))
    expect(calls('setup_connect')).toHaveLength(1)
    expect(screen.getByTestId('setup-drive-spinner')).toBeInTheDocument()

    await act(async () => setupAccountProbed([]))

    expect(calls('setup_create')).toContainEqual({ password: STRONG })
    await waitFor(() => expect(useApp.getState().flow).toBe('main'))
  })

  // The step offers both providers; the one pressed is the one probed, and the
  // wait says whose answer it is waiting on.
  it('connects whichever provider is pressed', async () => {
    render(<Start />)
    await choosePassword()

    await userEvent.click(await screen.findByTestId('setup-dropbox-connect'))

    expect(calls('setup_connect')).toEqual([{ provider: 'dropbox' }])
    expect(screen.getByText('Connecting to Dropbox…')).toBeInTheDocument()
  })

  // A double-click must not ask the backend to create twice: while the first
  // create is in flight both buttons are inert.
  it('creates once however many times the button is pressed', async () => {
    let finish: (result: { entries: never[]; syncConfigured: boolean }) => void = () => {}
    mockCommandOnce('setup_create', () => new Promise(resolve => (finish = resolve)))
    render(<Start />)
    await choosePassword()

    const skip = await screen.findByTestId('setup-skip-drive-button')
    await userEvent.click(skip)
    await userEvent.click(skip)
    await userEvent.click(screen.getByTestId('setup-drive-connect'))

    expect(calls('setup_create')).toHaveLength(1)
    expect(calls('setup_connect')).toHaveLength(0)

    await act(async () => finish({ entries: [], syncConfigured: false }))
    await waitFor(() => expect(useApp.getState().flow).toBe('main'))
  })

  // Leaving while consent is still out with the browser disowns it: a late
  // answer must not be waiting in the store the next time this step is shown.
  it('abandons a consent in flight when the user backs out', async () => {
    render(<Start />)
    await choosePassword()

    await userEvent.click(await screen.findByTestId('setup-drive-connect'))
    expect(screen.getByTestId('setup-drive-spinner')).toBeInTheDocument()

    await userEvent.click(screen.getByTestId('go-back-button'))

    expect(calls('setup_disconnect')).toHaveLength(1)
    expect(useApp.getState().setupAccount).toEqual({
      status: 'idle',
      provider: null,
      files: [],
      selectedId: null,
      error: null
    })
    expect(screen.getByTestId('setup-password-input')).toBeInTheDocument()
  })

  it('surfaces a failed connection and leaves the choice standing', async () => {
    render(<Start />)
    await choosePassword()

    await userEvent.click(await screen.findByTestId('setup-drive-connect'))
    await act(async () => setupAccountFailed('no oauth client configured'))

    expect(await screen.findByTestId('setup-drive-error')).toHaveTextContent(
      'no oauth client configured'
    )
    expect(screen.getByTestId('setup-drive-connect')).toBeInTheDocument()
    expect(calls('setup_create')).toHaveLength(0)
  })
})

describe('an account that already holds data', () => {
  const reachConflict = async () => {
    await choosePassword()
    await userEvent.click(await screen.findByTestId('setup-drive-connect'))
    await act(async () => setupAccountProbed([REMOTE]))
  }

  it('stops to ask rather than writing over the existing pack', async () => {
    render(<Start />)
    await reachConflict()

    expect(await screen.findByText('This Google Drive already has Rowel data')).toBeInTheDocument()
    // The hex file name means nothing to a user; the card is titled for what it
    // is, and the meta line is what tells two of them apart.
    expect(screen.getByTestId('setup-conflict-file')).toHaveTextContent('Rowel lockbox')
    expect(screen.getByTestId('setup-conflict-file')).not.toHaveTextContent(REMOTE.name)
    expect(calls('setup_create')).toHaveLength(0)
  })

  // Every device connected to an account syncs the vaults it holds. A second
  // vault beside the first is how data got forked across two packs, so the
  // screen offers exactly one way on: unlocking what is there.
  it('offers no way to start a second vault beside the existing one', async () => {
    render(<Start />)
    await reachConflict()

    expect(screen.queryByTestId('setup-start-fresh-button')).not.toBeInTheDocument()
    expect(screen.queryByText('Start a new vault')).not.toBeInTheDocument()
    expect(calls('setup_create')).toHaveLength(0)
  })

  it('hands over to the restore screen without asking Google again', async () => {
    render(<Start />)
    await reachConflict()

    await userEvent.click(await screen.findByTestId('setup-unlock-existing-button'))

    expect(await screen.findByTestId('drive-password-input')).toBeInTheDocument()
    // One connect for the whole flow: the found file is already in hand.
    expect(calls('setup_connect')).toHaveLength(1)
  })
})

describe('restoring from Google Drive', () => {
  const openDrive = async () => {
    await userEvent.click(screen.getByTestId('start-drive-button'))
  }

  it('waits on consent, then unlocks the pack the probe found', async () => {
    render(<Start />)
    await openDrive()

    expect(calls('setup_connect')).toHaveLength(1)
    expect(screen.getByTestId('drive-spinner')).toBeInTheDocument()

    await act(async () => setupAccountProbed([REMOTE]))
    // One vault: the card, not a choice — and titled for what it is rather
    // than by the opaque id its file is named after.
    expect(screen.getByTestId('drive-found-file')).toHaveTextContent('Rowel lockbox')
    expect(screen.queryByTestId(`drive-vault-${REMOTE.id}`)).not.toBeInTheDocument()

    await userEvent.type(screen.getByTestId('drive-password-input'), STRONG)
    await userEvent.click(screen.getByTestId('drive-unlock-button'))

    expect(calls('setup_restore_from_account')).toContainEqual({
      password: STRONG,
      fileId: REMOTE.id
    })
    await waitFor(() => expect(useApp.getState().flow).toBe('main'))
  })

  // Two installs syncing their own primary to one account leave two packs in
  // it. Restoring either is a choice only the user can make, so the screen
  // lists both and unlocks the one they point at.
  it('offers every vault in the account and restores the chosen one', async () => {
    render(<Start />)
    await openDrive()
    await act(async () => setupAccountProbed([REMOTE, OTHER]))

    expect(screen.queryByTestId('drive-found-file')).not.toBeInTheDocument()
    expect(screen.getByTestId(`drive-vault-${REMOTE.id}`)).toHaveAttribute('aria-checked', 'true')
    expect(screen.getByTestId(`drive-vault-${OTHER.id}`)).toHaveAttribute('aria-checked', 'false')

    await userEvent.click(screen.getByTestId(`drive-vault-${OTHER.id}`))
    await userEvent.type(screen.getByTestId('drive-password-input'), STRONG)
    await userEvent.click(screen.getByTestId('drive-unlock-button'))

    expect(calls('setup_restore_from_account')).toContainEqual({
      password: STRONG,
      fileId: OTHER.id
    })
    await waitFor(() => expect(useApp.getState().flow).toBe('main'))
  })

  // Once the unlock is running, the account it runs against is spoken for:
  // switching it, or leaving for a file, is not offered until it settles.
  it('withdraws account switching while the unlock is running', async () => {
    let finish: (result: { entries: never[]; syncConfigured: boolean }) => void = () => {}
    mockCommandOnce('setup_restore_from_account', () => new Promise(resolve => (finish = resolve)))
    render(<Start />)
    await openDrive()
    await act(async () => setupAccountProbed([REMOTE]))
    expect(screen.getByTestId('drive-switch-account')).toBeInTheDocument()

    await userEvent.type(screen.getByTestId('drive-password-input'), STRONG)
    await userEvent.click(screen.getByTestId('drive-unlock-button'))

    expect(screen.queryByTestId('drive-switch-account')).not.toBeInTheDocument()
    expect(screen.queryByTestId('drive-use-file')).not.toBeInTheDocument()

    await act(async () => finish({ entries: [], syncConfigured: true }))
    await waitFor(() => expect(useApp.getState().flow).toBe('main'))
  })

  it('names the password as the problem when the pack will not open', async () => {
    mockCommandOnce('setup_restore_from_account', () =>
      Promise.reject({ kind: 'invalidPassword', message: 'invalid master password' })
    )
    render(<Start />)
    await openDrive()
    await act(async () => setupAccountProbed([REMOTE]))

    await userEvent.type(screen.getByTestId('drive-password-input'), 'wrong-password')
    await userEvent.click(screen.getByTestId('drive-unlock-button'))

    expect(await screen.findByText(/isn't the password this was sealed with/)).toBeInTheDocument()
    expect(useApp.getState().flow).not.toBe('main')
  })

  it('sends a bare account back to the create flow, keeping the connection', async () => {
    render(<Start />)
    await openDrive()
    await act(async () => setupAccountProbed([]))

    expect(screen.getByText('Nothing here yet')).toBeInTheDocument()
    await userEvent.click(screen.getByTestId('drive-start-fresh-button'))

    // The sync step is skipped: Drive has already been agreed to.
    await userEvent.type(screen.getByTestId('setup-password-input'), STRONG)
    await userEvent.type(screen.getByTestId('setup-confirm-password-input'), STRONG)
    await userEvent.click(screen.getByTestId('setup-continue-button'))

    expect(calls('setup_create')).toContainEqual({ password: STRONG })
    await waitFor(() => expect(useApp.getState().flow).toBe('main'))
  })

  it('gives the pending tokens back when the user walks away', async () => {
    render(<Start />)
    await openDrive()
    await act(async () => setupAccountProbed([REMOTE]))

    await userEvent.click(screen.getByTestId('go-back-button'))

    expect(calls('setup_disconnect')).toHaveLength(1)
    expect(useApp.getState().setupAccount).toEqual({
      status: 'idle',
      provider: null,
      files: [],
      selectedId: null,
      error: null
    })
    expect(screen.getByTestId('start-setup-button')).toBeInTheDocument()
  })

  // Switching to a backup file is leaving the account behind too — the tokens
  // must not sit in the backend for a flow that will never use them.
  it('forgets the account when the user switches to a backup file', async () => {
    render(<Start />)
    await openDrive()
    await act(async () => setupAccountProbed([REMOTE]))

    await userEvent.click(screen.getByTestId('drive-use-file'))

    expect(calls('setup_disconnect')).toHaveLength(1)
    expect(screen.getByTestId('restore-dropzone')).toBeInTheDocument()
  })

  // Only the backend's own verdict blames the password; a failure before the
  // password was ever checked is shown as what it was.
  it('shows a non-password failure as itself', async () => {
    mockCommandOnce('setup_restore_from_account', () =>
      Promise.reject({ kind: 'io', message: 'sync file is truncated' })
    )
    render(<Start />)
    await openDrive()
    await act(async () => setupAccountProbed([REMOTE]))

    await userEvent.type(screen.getByTestId('drive-password-input'), STRONG)
    await userEvent.click(screen.getByTestId('drive-unlock-button'))

    expect(await screen.findByText('sync file is truncated')).toBeInTheDocument()
  })
})

// The restore screen is the same for every provider; only whose name it
// carries changes, and it knows that from the press, before the probe answers.
describe('the restore screen under each provider', () => {
  const DROPBOX_PACK: RemotePack = { ...REMOTE, id: 'id:dropbox-1', provider: 'dropbox' }

  it.each([
    ['start-drive-button', 'Google Drive'],
    ['start-dropbox-button', 'Dropbox']
  ])('names the provider %s connected while it waits', async (testid, name) => {
    render(<Start />)
    await userEvent.click(screen.getByTestId(testid))

    expect(screen.getByText(`Restore · ${name}`)).toBeInTheDocument()
    expect(screen.getByTestId('drive-spinner')).toHaveTextContent(`Waiting for ${name}…`)
  })

  it('says a bare Dropbox account is a Dropbox one', async () => {
    render(<Start />)
    await userEvent.click(screen.getByTestId('start-dropbox-button'))
    await act(async () => setupAccountProbed([]))

    expect(screen.getByText('Restore · Dropbox')).toBeInTheDocument()
    expect(
      screen.getByText('This Dropbox account has no Rowel data. Start fresh and it will sync here.')
    ).toBeInTheDocument()
  })

  it('blames Dropbox, and retries with Dropbox, when it does not answer', async () => {
    render(<Start />)
    await userEvent.click(screen.getByTestId('start-dropbox-button'))
    await act(async () => setupAccountFailed('access_denied'))

    expect(screen.getByText('Dropbox did not answer')).toBeInTheDocument()
    await userEvent.click(screen.getByTestId('drive-retry-button'))

    expect(calls('setup_connect')).toEqual([{ provider: 'dropbox' }, { provider: 'dropbox' }])
  })

  it('restores a Dropbox pack by its Dropbox id', async () => {
    render(<Start />)
    await userEvent.click(screen.getByTestId('start-dropbox-button'))
    await act(async () => setupAccountProbed([DROPBOX_PACK]))

    expect(screen.getByText('Restore · Dropbox')).toBeInTheDocument()
    await userEvent.type(screen.getByTestId('drive-password-input'), STRONG)
    await userEvent.click(screen.getByTestId('drive-unlock-button'))

    expect(calls('setup_restore_from_account')).toEqual([
      { password: STRONG, fileId: DROPBOX_PACK.id }
    ])
  })

  // Switch account stays with the provider the flow is on.
  it('switches account within the same provider', async () => {
    render(<Start />)
    await userEvent.click(screen.getByTestId('start-dropbox-button'))
    await act(async () => setupAccountProbed([DROPBOX_PACK]))

    await userEvent.click(screen.getByTestId('drive-switch-account'))

    await waitFor(() => expect(calls('setup_connect')).toHaveLength(2))
    expect(calls('setup_disconnect')).toHaveLength(1)
    expect(calls('setup_connect')[1]).toEqual({ provider: 'dropbox' })
  })
})

describe('restoring from a backup file', () => {
  it('picks a file, then unseals it with the master password', async () => {
    vi.mocked(open).mockResolvedValue('/tmp/Rowel backup 2026-09-15.rowel')
    render(<Start />)

    await userEvent.click(screen.getByTestId('start-restore-button'))
    await userEvent.click(screen.getByTestId('restore-dropzone'))

    expect(await screen.findByTestId('restore-found-file')).toHaveTextContent(
      'Rowel backup 2026-09-15.rowel'
    )

    await userEvent.type(screen.getByTestId('restore-password-input'), STRONG)
    await userEvent.click(screen.getByTestId('restore-confirm-button'))

    expect(calls('setup_restore_from_file')).toContainEqual(
      { path: '/tmp/Rowel backup 2026-09-15.rowel', password: STRONG }
    )
    await waitFor(() => expect(useApp.getState().flow).toBe('main'))
  })

  // The pack is what says whether the password was wrong; anything else it
  // reports — a truncated file, a vault from a newer build — is shown as itself.
  it('shows a non-password failure as itself', async () => {
    vi.mocked(open).mockResolvedValue('/tmp/vault.rowel')
    mockCommandOnce('setup_restore_from_file', () =>
      Promise.reject({ kind: 'io', message: 'sync file is truncated' })
    )
    render(<Start />)

    await userEvent.click(screen.getByTestId('start-restore-button'))
    await userEvent.click(screen.getByTestId('restore-dropzone'))
    await screen.findByTestId('restore-found-file')

    await userEvent.type(screen.getByTestId('restore-password-input'), STRONG)
    await userEvent.click(screen.getByTestId('restore-confirm-button'))

    expect(await screen.findByText('sync file is truncated')).toBeInTheDocument()
  })

  // Double-clicking a backup on a machine with no vault yet is asking to
  // restore it: the flow opens on that step with the file already picked, and
  // the store's copy is taken so nothing offers it a second time.
  it('opens on the restore step, file picked, when the OS opened a .rowel with the app', async () => {
    fileOpened('/tmp/Rowel backup 2026-09-15.rowel')
    render(<Start />)

    expect(await screen.findByTestId('restore-found-file')).toHaveTextContent(
      'Rowel backup 2026-09-15.rowel'
    )
    expect(screen.getByTestId('restore-password-input')).toBeInTheDocument()
    expect(useApp.getState().openedFile).toBeNull()

    await userEvent.type(screen.getByTestId('restore-password-input'), STRONG)
    await userEvent.click(screen.getByTestId('restore-confirm-button'))
    expect(calls('setup_restore_from_file')).toContainEqual({
      path: '/tmp/Rowel backup 2026-09-15.rowel',
      password: STRONG
    })
  })

  // The picker is already up when the OS opens a backup: the screen key does
  // not change, so the step has to adopt the file without being remounted.
  it('adopts a .rowel the OS opens while the picker is already on screen', async () => {
    render(<Start />)
    await userEvent.click(screen.getByTestId('start-restore-button'))
    expect(screen.getByTestId('restore-dropzone')).toBeInTheDocument()

    act(() => fileOpened('/tmp/late.rowel'))

    expect(await screen.findByTestId('restore-found-file')).toHaveTextContent('late.rowel')
    expect(useApp.getState().openedFile).toBeNull()
  })

  // A legacy `.swftx` cannot be restored — only imported into a vault — so the
  // first run leaves it parked for Settings › Import once there is one.
  it('leaves a .swftx the OS opened for the vault this flow ends in', () => {
    fileOpened('/tmp/old-vault.swftx')
    render(<Start />)

    expect(screen.getByTestId('start-restore-button')).toBeInTheDocument()
    expect(screen.queryByTestId('restore-found-file')).not.toBeInTheDocument()
    expect(useApp.getState().openedFile).toBe('/tmp/old-vault.swftx')
  })
})

describe('the biometric step', () => {
  const reachBiometric = async () => {
    biometricStatus(true)
    render(<Start />)
    await choosePassword()
    await userEvent.click(await screen.findByTestId('setup-skip-drive-button'))
    await screen.findByTestId('setup-enable-biometric-button')
  }

  it('is only offered where the device has a gate to offer', async () => {
    biometricStatus(false)
    render(<Start />)
    await choosePassword()
    await userEvent.click(await screen.findByTestId('setup-skip-drive-button'))

    await waitFor(() => expect(useApp.getState().flow).toBe('main'))
    expect(screen.queryByTestId('setup-enable-biometric-button')).not.toBeInTheDocument()
  })

  it('enrolls and then opens the app', async () => {
    await reachBiometric()

    expect(useApp.getState().flow).not.toBe('main')
    await userEvent.click(screen.getByTestId('setup-enable-biometric-button'))

    expect(calls('enable_biometric')).toHaveLength(1)
    await waitFor(() => expect(useApp.getState().flow).toBe('main'))
  })

  it('opens the app just the same when it is declined', async () => {
    await reachBiometric()

    await userEvent.click(screen.getByTestId('setup-skip-biometric-button'))

    expect(calls('enable_biometric')).toHaveLength(0)
    await waitFor(() => expect(useApp.getState().flow).toBe('main'))
  })
})
