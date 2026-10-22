import { describe, it, expect, beforeEach, vi } from 'vitest'
import { act, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import Show from '@/components/Main/Body/Aside/Show'
import Aside from '@/components/Main/Body/Aside'
import Generator from '@/components/Main/Generator'
import AddSecret from '@/components/Main/AddSecret'
import { openAddPicker, newEntry, setCurrentEntry, useVault } from '@/store'
import type { Entry, LoginEntry } from '@/api/types'
import { withEntries, loginEntry, loginMeta } from './utils'
import { calls, mockCommand } from './ipc'
import { toEntryMeta } from './meta'

beforeEach(() => {
  vi.clearAllMocks()
  mockCommand('save_entry', ({ entry }) => toEntryMeta(entry as Entry))
})

const titleInput = () => document.querySelector<HTMLInputElement>('input[name="title"]')!
const field = (name: string) => document.querySelector<HTMLInputElement>(`input[name="${name}"]`)!

describe('Editing in the pane', () => {
  it('renders login fields for a new entry', () => {
    render(<Show type="login" editing />)
    // en-US maps "Website" -> "URL"
    expect(screen.getByText('URL')).toBeInTheDocument()
    expect(screen.getByText('Username')).toBeInTheDocument()
  })

  it('says which kind is being edited, and names the empty title after it', () => {
    render(<Show type="card" editing />)
    expect(screen.getByText('Editing')).toBeInTheDocument()
    expect(titleInput()).toHaveAttribute('placeholder', 'Untitled credit card')
  })

  it('saves a valid new login', async () => {
    render(<Show type="login" editing />)
    await userEvent.type(titleInput(), 'GitHub')
    await userEvent.type(field('username'), 'octocat')
    await userEvent.type(field('password'), 'pw')
    await userEvent.click(screen.getByText('Save'))

    expect(calls('save_entry')).toHaveLength(1)
    await waitFor(() => expect(useVault.getState().items[0].title).toBe('GitHub'))
  })

  // A new entry has no id until the backend answers; every save call before
  // that minted its own, so two quick presses wrote two rows.
  it('writes a new entry once however many times Save is pressed in flight', async () => {
    let finish: (value: unknown) => void = () => {}
    mockCommand('save_entry', ({ entry }) => new Promise(done => (finish = () => done(toEntryMeta(entry as Entry)))))
    render(<Show type="login" editing />)
    await userEvent.type(titleInput(), 'GitHub')
    await userEvent.type(field('username'), 'octocat')
    await userEvent.type(field('password'), 'pw')

    await userEvent.click(screen.getByTestId('save-entry-button'))
    await userEvent.click(screen.getByTestId('save-entry-button'))
    await userEvent.keyboard('{Meta>}{Enter}{/Meta}')

    expect(calls('save_entry')).toHaveLength(1)
    expect(screen.getByTestId('save-entry-button')).toBeDisabled()

    await act(async () => finish(undefined))
    await waitFor(() => expect(useVault.getState().items).toHaveLength(1))
  })

  it('blocks an invalid save and says which rows are missing', async () => {
    render(<Show type="login" editing />)
    await userEvent.click(screen.getByText('Save'))

    expect(calls('save_entry')).toHaveLength(0)
    // Title, username and password: the three fields login's isValid requires.
    expect(screen.getAllByText('Required')).toHaveLength(3)
  })

  it('closes straight away when nothing was typed', async () => {
    render(<Show type="login" editing />)
    newEntry('login')
    await userEvent.click(screen.getByTestId('cancel-entry-button'))
    expect(screen.queryByText('Discard changes?')).not.toBeInTheDocument()
    expect(useVault.getState().creating).toBeNull()
  })

  it('guards unsaved changes with an inline confirm', async () => {
    render(<Show type="login" editing />)
    newEntry('login')
    await userEvent.type(titleInput(), 'GitHub')

    // First press only arms the confirm; the editor stays open.
    await userEvent.click(screen.getByTestId('cancel-entry-button'))
    expect(screen.getByText('Discard changes?')).toBeInTheDocument()
    expect(useVault.getState().creating).toBe('login')

    // Second press discards.
    await userEvent.click(screen.getByTestId('cancel-entry-button'))
    expect(useVault.getState().creating).toBeNull()
  })

  it('runs the same discard guard on Escape', async () => {
    render(<Show type="login" editing />)
    newEntry('login')
    await userEvent.type(titleInput(), 'GitHub')

    await userEvent.keyboard('{Escape}')
    expect(screen.getByText('Discard changes?')).toBeInTheDocument()
    expect(useVault.getState().creating).toBe('login')

    await userEvent.keyboard('{Escape}')
    expect(useVault.getState().creating).toBeNull()
  })

  it('stands down on both keys while a dialog owns the keyboard', async () => {
    render(
      <>
        <Show type="login" editing />
        <AddSecret />
      </>
    )
    newEntry('login')
    await userEvent.type(titleInput(), 'GitHub')
    await userEvent.type(field('username'), 'octocat')
    await userEvent.type(field('password'), 'pw')

    // The editor's own listener is on `document`, so it sees these keys on the
    // way down to the dialog's handler. Dismissing a dialog must not end the
    // edit session, and saving behind one must not happen at all.
    act(() => openAddPicker())
    expect(screen.getByRole('dialog')).toBeInTheDocument()

    await userEvent.keyboard('{Escape}')
    expect(screen.queryByText('Discard changes?')).not.toBeInTheDocument()
    expect(useVault.getState().creating).toBe('login')

    act(() => openAddPicker())
    await userEvent.keyboard('{Meta>}{Enter}{/Meta}')
    expect(calls('save_entry')).toHaveLength(0)
  })

  it('saves on ⌘⏎ from anywhere in the pane', async () => {
    render(<Show type="login" editing />)
    await userEvent.type(titleInput(), 'GitHub')
    await userEvent.type(field('username'), 'octocat')
    await userEvent.type(field('password'), 'pw')
    await userEvent.keyboard('{Meta>}{Enter}{/Meta}')

    expect(calls('save_entry')).toHaveLength(1)
  })

  it('generates a password through the generator dialog', async () => {
    mockCommand('generate_password', () => 'Generated123!')
    render(
      <>
        <Show type="login" editing />
        <Generator />
      </>
    )
    await userEvent.click(screen.getByTestId('generate-password-link'))
    expect(await screen.findByTestId('generator-dialog')).toBeInTheDocument()
    await screen.findByText('Generated123!')

    await userEvent.click(screen.getByTestId('generator-use-button'))
    await waitFor(() => expect(field('password').value).toBe('Generated123!'))
    expect(screen.queryByTestId('generator-dialog')).not.toBeInTheDocument()
  })

  // One press of Generate settles three rows: an SSH key is a keypair, not a
  // string, so the generator hands the whole thing back to the draft.
  it('generates a whole SSH keypair into the draft', async () => {
    render(
      <>
        <Show type="ssh" editing />
        <Generator />
      </>
    )
    await userEvent.type(titleInput(), 'Deploy key')
    await userEvent.click(screen.getByTestId('generate-ssh-key-link'))

    expect(await screen.findByTestId('generator-ssh-public')).toBeInTheDocument()
    await userEvent.click(screen.getByTestId('generator-use-button'))

    await waitFor(() =>
      expect(field('publicKey').value).toBe('ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI')
    )
    expect(screen.getByTestId('entry-value-fingerprint')).toHaveTextContent(
      'SHA256:GeneratedFingerprint'
    )

    await userEvent.click(screen.getByText('Save'))
    expect(calls('save_entry')).toContainEqual({ entry: expect.objectContaining({
        type: 'ssh',
        title: 'Deploy key',
        privateKey: expect.stringContaining('OPENSSH PRIVATE KEY'),
        fingerprint: 'SHA256:GeneratedFingerprint'
      }) })
  })

  it('dismisses the generator on Escape without ending the edit session', async () => {
    // The reported reproducer: add a login, open the generator off the password
    // row, press Escape. The editor's Esc listener is on `document`, so it sees
    // the key first — and a draft with nothing typed yet closes with no confirm,
    // silently taking the session down with the dialog.
    mockCommand('generate_password', () => 'Generated123!')
    render(
      <>
        <Show type="login" editing />
        <Generator />
      </>
    )
    newEntry('login')

    await userEvent.click(screen.getByTestId('generate-password-link'))
    expect(await screen.findByTestId('generator-dialog')).toBeInTheDocument()

    await userEvent.keyboard('{Escape}')

    // Only the generator went.
    expect(screen.queryByTestId('generator-dialog')).not.toBeInTheDocument()
    expect(useVault.getState().creating).toBe('login')
    expect(screen.getByTestId('entry-sheet')).toBeInTheDocument()

    // And the editor still owns Escape now that the dialog is gone.
    await userEvent.keyboard('{Escape}')
    expect(useVault.getState().creating).toBeNull()
  })

  it('keeps in-progress edits when the entry refreshes mid-edit', async () => {
    // The revealed title differs from the metadata title so this wait proves
    // the decrypted values were actually adopted, not just the initial meta.
    mockCommand('reveal_entry', () => loginEntry({ title: 'Google (decrypted)' }))
    const { rerender } = render(<Show entry={loginMeta()} editing />)
    await waitFor(() => expect(titleInput().value).toBe('Google (decrypted)'))

    await userEvent.clear(titleInput())
    await userEvent.type(titleInput(), 'Renamed by me')

    // A sync merge landing mid-edit bumps updatedAt and re-runs the decrypt.
    // The draft adopts the reveal once, at open — a refetch must not clobber
    // what the user has typed (their save wins by last-writer-wins anyway).
    mockCommand('reveal_entry', () => loginEntry({ title: 'Merged elsewhere' }))
    rerender(<Show entry={loginMeta({ updatedAt: '2024-06-01T00:00:00.000Z' })} editing />)
    await waitFor(() => expect(calls('reveal_entry')).toHaveLength(2))

    expect(titleInput().value).toBe('Renamed by me')
  })

  it('starts a new draft empty after reading an entry, and saves it as a new one', async () => {
    // The reproducer behind every multi-entry e2e failure: read an entry, then
    // press Add. Both modes are a `Show` in the same slot, so React kept one
    // instance, and `useRevealed` only drops the old reveal in an effect — one
    // render too late. The fresh draft adopted the read entry, id included, and
    // "Add" quietly saved over it instead of creating a row.
    mockCommand('reveal_entry', () => loginEntry({ title: 'Google' }))
    withEntries([loginMeta()])
    setCurrentEntry('l1')
    render(<Aside />)

    // The read view is up and the reveal has landed.
    await screen.findByTestId('more-actions-button')
    await waitFor(() => expect(calls('reveal_entry')).toContainEqual({ id: 'l1' }))

    act(() => newEntry('login'))
    expect(titleInput().value).toBe('')

    await userEvent.type(titleInput(), 'GitHub')
    await userEvent.type(field('username'), 'octocat')
    await userEvent.type(field('password'), 'pw')
    await userEvent.click(screen.getByTestId('save-entry-button'))

    // The store mints the id for a draft that has none, so what proves the
    // draft was new is that the id is not the entry we were just reading.
    expect(calls('save_entry')).toHaveLength(1)
    const saved = calls('save_entry')[0].entry as Entry
    expect(saved.title).toBe('GitHub')
    expect(saved.id).not.toBe('l1')

    // And the list grew instead of the read entry being overwritten.
    await waitFor(() => expect(useVault.getState().items).toHaveLength(2))
    expect(useVault.getState().items[0].title).toBe('Google')
  })
})

describe('Show', () => {
  it('renders entry details', async () => {
    mockCommand('reveal_entry', () => loginEntry({ title: 'Google' }))
    render(<Show entry={loginMeta({ title: 'Google' })} />)
    expect(screen.getByRole('heading', { name: 'Google' })).toBeInTheDocument()
    expect(await screen.findByText('me@example.com')).toBeInTheDocument()
  })

  it('renders and copies a TOTP code', async () => {
    mockCommand('generate_otp', () => ({ code: '123456', time: 25 }))
    mockCommand('reveal_entry', () => loginEntry({ otp: 'BASE32SECRET' }) as LoginEntry)
    render(<Show entry={loginMeta()} />)

    expect(await screen.findByText('123 456')).toBeInTheDocument()
    expect(calls('generate_otp')).toContainEqual({ secret: 'BASE32SECRET' })
  })

  it('copies a field value', async () => {
    mockCommand('reveal_entry', () => loginEntry({ username: 'copyme' }))
    render(<Show entry={loginMeta()} />)
    await screen.findByText('copyme')
    // The value is the row's copy control; there is no button beside it.
    await userEvent.click(screen.getByTestId('entry-value-username'))
    expect(calls('copy_to_clipboard')).toContainEqual(
      { value: 'copyme', clearAfterMs: expect.any(Number) }
    )
  })

  it('copies the password from the header without revealing it', async () => {
    mockCommand('reveal_entry', () => loginEntry({ password: 'hunter2' }))
    render(<Show entry={loginMeta()} />)

    const action = await screen.findByTestId('primary-action-button')
    await waitFor(() => expect(action).toBeEnabled())
    expect(action).toHaveTextContent('Copy password')
    await userEvent.click(action)

    expect(calls('copy_to_clipboard')).toContainEqual(
      { value: 'hunter2', clearAfterMs: expect.any(Number) }
    )
    // The password row is still masked: nothing toggled its reveal.
    expect(screen.getByTitle('Reveal')).toBeInTheDocument()
  })

  it('re-decrypts after an in-place save, so copy never serves the old secret', async () => {
    mockCommand('reveal_entry', () => loginEntry({ password: 'old-secret' }))
    const { rerender } = render(<Show entry={loginMeta()} />)
    const action = await screen.findByTestId('primary-action-button')
    await waitFor(() => expect(action).toBeEnabled())

    // A save keeps the id but stamps updatedAt. The pane stays mounted across
    // saves, so a decrypt keyed on the id alone kept serving — and copying —
    // the pre-edit password. The regression: rotate, then copy.
    mockCommand('reveal_entry', () => loginEntry({ password: 'new-secret' }))
    rerender(<Show entry={loginMeta({ updatedAt: '2024-06-01T00:00:00.000Z' })} />)
    await waitFor(() => expect(calls('reveal_entry')).toHaveLength(2))

    const refreshed = screen.getByTestId('primary-action-button')
    await waitFor(() => expect(refreshed).toBeEnabled())
    await userEvent.click(refreshed)

    expect(calls('copy_to_clipboard')).toContainEqual(
      { value: 'new-secret', clearAfterMs: expect.any(Number) }
    )
  })

  it('runs the primary action on a bare Enter', async () => {
    mockCommand('reveal_entry', () => loginEntry({ password: 'hunter2' }))
    render(<Show entry={loginMeta()} />)

    await waitFor(() => expect(screen.getByTestId('primary-action-button')).toBeEnabled())
    await userEvent.keyboard('{Enter}')

    expect(calls('copy_to_clipboard')).toContainEqual(
      { value: 'hunter2', clearAfterMs: expect.any(Number) }
    )
  })

  it('announces the more menu trigger as a disclosure', async () => {
    mockCommand('reveal_entry', () => loginEntry())
    render(<Show entry={loginMeta()} />)

    const trigger = screen.getByTestId('more-actions-button')
    expect(trigger).toHaveAttribute('aria-haspopup', 'menu')
    expect(trigger).toHaveAttribute('aria-expanded', 'false')

    await userEvent.click(trigger)
    expect(trigger).toHaveAttribute('aria-expanded', 'true')
  })

  it('leaves Enter to whichever control holds focus', async () => {
    mockCommand('reveal_entry', () => loginEntry({ password: 'hunter2' }))
    render(<Show entry={loginMeta()} />)

    await waitFor(() => expect(screen.getByTestId('primary-action-button')).toBeEnabled())

    // Enter on a focused button activates that button and nothing else — it
    // used to open the more menu *and* copy the password.
    screen.getByTestId('more-actions-button').focus()
    await userEvent.keyboard('{Enter}')

    expect(screen.getByRole('menu')).toBeInTheDocument()
    expect(calls('copy_to_clipboard')).toHaveLength(0)
  })

  it('archives from the more menu behind a two-press inline confirm', async () => {
    mockCommand('reveal_entry', () => loginEntry())
    render(<Show entry={loginMeta()} />)

    await userEvent.click(screen.getByTestId('more-actions-button'))

    // First press only arms the row.
    await userEvent.click(screen.getByText('Archive'))
    expect(calls('delete_entry')).toHaveLength(0)

    // Second press archives.
    await userEvent.click(screen.getByText('Archive entry?'))
    await waitFor(() => expect(calls('delete_entry')).toContainEqual({ id: 'l1' }))
  })
})
