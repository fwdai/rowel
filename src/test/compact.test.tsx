import { describe, it, expect, beforeEach, vi } from 'vitest'
import { act, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import Main from '@/components/Main'
import AuthShell from '@/components/elements/AuthShell'
import Frame, { FrameProvider } from '@/components/elements/Frame'
import Sheet from '@/components/elements/Sheet'
import type { Entry } from '@/api/types'
import { type Audit } from '@/api/tools'
import {
  useUi,
  useVault,
  selectCurrent,
  editEntry,
  lockSettings,
  openPalette,
  openSettings,
  openAddPicker,
  openGenerator,
  setCurrentEntry,
  setEntries,
  setView,
  startEntry
} from '@/store'
import { withEntries, loginEntry, loginMeta } from './utils'
import { setLayout } from './layout'
import { calls, mockCommand } from './ipc'

const seed = () =>
  withEntries([loginMeta({ id: 'l1', title: 'Google' }), loginMeta({ id: 'l2', title: 'Airbnb' })])

const audit: Audit = {
  l1: { score: 0, isWeak: true, isRepeating: false, breached: false },
  l2: { score: 2, isWeak: false, isRepeating: true, breached: false }
}

// The form's inputs carry names rather than testids, as the wide suite's do.
const titleInput = () => document.querySelector<HTMLInputElement>('input[name="title"]')!
const field = (name: string) => document.querySelector<HTMLInputElement>(`input[name="${name}"]`)!

// Edit is under the read screen's one menu, not a button of its own.
const openEdit = async () => {
  await userEvent.click(screen.getByTestId('more-actions-button'))
  await userEvent.click(screen.getByTestId('edit-entry-button'))
}

beforeEach(() => {
  vi.clearAllMocks()
  mockCommand('generate_password', () => 'Generated123!')
  setLayout('compact')
})

describe('compact shell', () => {
  it('replaces the rail with a tab bar, puts add in its centre and tags in the list header', () => {
    seed()
    render(<Main />)

    expect(screen.getByTestId('compact-shell')).toBeInTheDocument()
    expect(screen.getByTestId('tab-bar')).toBeInTheDocument()
    expect(screen.queryByTestId('view-items')).not.toBeInTheDocument()
    // The verb rests in the bar's notch, in the thumb's spot — beside the bar
    // in the tree, since the bar is masked around it. The header keeps only
    // the controls that narrow the list under it.
    expect(screen.getByTestId('add-entry-button')).toBeInTheDocument()
    expect(screen.getByTestId('tab-bar')).not.toContainElement(
      screen.getByTestId('add-entry-button')
    )
    expect(screen.getByTestId('tags-button')).toBeInTheDocument()
  })

  it('pushes the detail screen on select and comes back from it', async () => {
    mockCommand('reveal_entry', () => loginEntry({ id: 'l1', title: 'Google' }))
    seed()
    render(<Main />)

    expect(screen.getAllByTestId('entry-item')).toHaveLength(2)
    expect(screen.queryByTestId('compact-back')).not.toBeInTheDocument()

    await userEvent.click(screen.getByText('Google'))
    expect(screen.getByRole('heading', { name: 'Google' })).toBeInTheDocument()
    // The way back is named after where it goes, iOS-style — the list root's
    // own title, from the same hook the list root draws it with.
    expect(screen.getByTestId('compact-back')).toHaveTextContent('All Items')
    // One screen at a time: the list and the tab bar are gone while it is up.
    expect(screen.queryByTestId('entry-item')).not.toBeInTheDocument()
    expect(screen.queryByTestId('tab-bar')).not.toBeInTheDocument()
    // The corner holds one control: the star sits by the entry's name, and
    // Edit waits under the menu.
    expect(screen.getByTestId('favorite-toggle')).toBeInTheDocument()
    expect(screen.queryByTestId('edit-entry-button')).not.toBeInTheDocument()
    await userEvent.click(screen.getByTestId('more-actions-button'))
    expect(screen.getByTestId('edit-entry-button')).toBeInTheDocument()
    await userEvent.keyboard('{Escape}')

    await userEvent.click(screen.getByTestId('compact-back'))
    expect(useVault.getState().currentId).toBeNull()
    expect(screen.getAllByTestId('entry-item')).toHaveLength(2)
  })

  it('lands a new entry on the form screen, titled by its kind', async () => {
    seed()
    render(<Main />)

    await userEvent.click(screen.getByTestId('add-entry-button'))
    await userEvent.click(screen.getByTestId('add-kind-login'))

    expect(useVault.getState().creating).toBe('login')
    // Cancel · what this screen is for · Save, the iOS way round. The kind
    // names the screen because there is no entry to name it yet.
    expect(screen.getByTestId('cancel-entry-button')).toHaveTextContent('Cancel')
    expect(screen.getByRole('heading', { name: 'Add a login' })).toBeInTheDocument()
    expect(screen.getByTestId('save-entry-button')).toHaveTextContent('Save')
    // Cancel/Discard is the only exit, so a draft cannot be dropped by a stray
    // tap on a back button.
    expect(screen.queryByTestId('compact-back')).not.toBeInTheDocument()
    // The desktop's accent frame is the pane saying which mode it is in; here
    // the nav row says it.
    expect(screen.queryByTestId('entry-sheet')).not.toBeInTheDocument()
  })

  it('refuses to save an untitled draft and stays on the form', async () => {
    seed()
    render(<Main />)

    act(() => startEntry('login'))
    await userEvent.click(screen.getByTestId('save-entry-button'))

    // The title's own message plus the two rows login also requires.
    expect(screen.getAllByText('Required')).toHaveLength(3)
    expect(calls('save_entry')).toHaveLength(0)
    expect(useVault.getState().creating).toBe('login')
  })

  it('guards a dirty draft behind two presses of Cancel', async () => {
    seed()
    render(<Main />)

    act(() => startEntry('login'))
    await userEvent.type(titleInput(), 'Netflix')

    const cancel = screen.getByTestId('cancel-entry-button')
    await userEvent.click(cancel)
    // Armed, and saying so where the way out is.
    expect(cancel).toHaveTextContent('Discard changes?')
    expect(useVault.getState().creating).toBe('login')

    await userEvent.click(cancel)
    expect(useVault.getState().creating).toBeNull()
    expect(screen.getAllByTestId('entry-item')).toHaveLength(2)
  })

  it('opens an existing entry seeded from its reveal, and saves it', async () => {
    mockCommand('reveal_entry', () =>
      loginEntry({ id: 'l1', title: 'Google', username: 'me@example.com' })
    )
    seed()
    render(<Main />)

    await userEvent.click(screen.getByText('Google'))
    await openEdit()

    expect(useVault.getState().editing).toBe(true)
    // The entry names its own form, and the draft holds the decrypted values:
    // the screen is held back until the reveal lands, so it never seeds empty.
    expect(screen.getByRole('heading', { name: 'Google' })).toBeInTheDocument()
    expect(field('username')).toHaveValue('me@example.com')
    expect(screen.queryByTestId('compact-back')).not.toBeInTheDocument()

    await userEvent.click(screen.getByTestId('save-entry-button'))
    expect(calls('save_entry')).toContainEqual(
      { entry: expect.objectContaining({ id: 'l1', title: 'Google', username: 'me@example.com' }) }
    )
  })

  // One screen, one reveal: the read face and the editor are two faces of
  // `Compact/Entry`, so stepping between them decrypts nothing again — which is
  // what used to blank the screen on the way into edit.
  it('shares one reveal between reading and editing an entry', async () => {
    mockCommand('reveal_entry', () =>
      loginEntry({ id: 'l1', title: 'Google', username: 'me@example.com' })
    )
    seed()
    render(<Main />)

    await userEvent.click(screen.getByText('Google'))
    expect(calls('reveal_entry')).toHaveLength(1)

    await openEdit()
    expect(calls('reveal_entry')).toHaveLength(1)
    // Seeded from the reveal the read screen already had, with no held frame
    // in between.
    expect(field('username')).toHaveValue('me@example.com')
  })

  it('leaves a way out of a form whose secrets never land', async () => {
    // A reveal that never settles: the editor cannot be seeded, so the screen
    // holds its frame — and has to stay leavable while it does.
    mockCommand('reveal_entry', () => new Promise<Entry>(() => {}))
    seed()
    render(<Main />)

    await userEvent.click(screen.getByText('Google'))
    await openEdit()

    expect(screen.getByRole('heading', { name: 'Google' })).toBeInTheDocument()
    await userEvent.click(screen.getByTestId('cancel-entry-button'))
    expect(useVault.getState().currentId).toBeNull()
    expect(screen.getAllByTestId('entry-item')).toHaveLength(2)
  })

  it('holds the primary action at the bottom until the secrets are in', async () => {
    let land: (entry: Entry) => void = () => {}
    mockCommand('reveal_entry', () => new Promise<Entry>(resolve => {
        land = resolve
      }))
    seed()
    render(<Main />)

    await userEvent.click(screen.getByText('Google'))
    const action = screen.getByTestId('primary-action-button')
    // Nothing decrypted yet: a button that copies '' is worse than one that is
    // plainly not ready.
    expect(action).toBeDisabled()

    await act(async () => land(loginEntry({ id: 'l1', password: 'hunter2' })))
    expect(action).toBeEnabled()
    expect(action).toHaveTextContent('Copy password')

    await userEvent.click(action)
    expect(calls('copy_to_clipboard')).toContainEqual(
      { value: 'hunter2', clearAfterMs: expect.any(Number) }
    )
  })

  it('copies a field value when the value itself is tapped', async () => {
    mockCommand('reveal_entry', () => loginEntry({ id: 'l1', username: 'copyme' }))
    seed()
    render(<Main />)

    await userEvent.click(screen.getByText('Google'))
    await userEvent.click(await screen.findByTestId('entry-value-username'))

    expect(calls('copy_to_clipboard')).toContainEqual(
      { value: 'copyme', clearAfterMs: expect.any(Number) }
    )
  })

  it('switches view from the tab bar', async () => {
    seed()
    render(<Main />)

    await userEvent.click(screen.getByTestId('tab-favorites'))
    expect(useUi.getState().view).toBe('favorites')
    expect(screen.getByTestId('list-title')).toHaveTextContent('Favorites')

    await userEvent.click(screen.getByTestId('tab-items'))
    expect(useUi.getState().view).toBe('items')
  })

  it('slides one lens to the selected tab and parks it for a view with no tab', async () => {
    seed()
    render(<Main />)
    const lens = screen.getByTestId('tab-lens')

    // Five slots, the third empty under the Add disc: the lens is moved by
    // whole slots, so favorites is one over and settings four.
    expect(lens.style.transform).toBe('translateX(0%)')
    await userEvent.click(screen.getByTestId('tab-favorites'))
    expect(lens.style.transform).toBe('translateX(100%)')
    await userEvent.click(screen.getByTestId('tab-settings'))
    expect(lens.style.transform).toBe('translateX(400%)')

    // The archive is a list with no tab: the lens fades where it stands —
    // still under Settings — instead of sliding to a tab that is not selected
    // while it goes.
    await userEvent.click(screen.getByTestId('settings-archive'))
    expect(lens.style.transform).toBe('translateX(400%)')
    expect(lens.className).toContain('opacity-0')
    expect(screen.getByTestId('tab-items')).toHaveAttribute('aria-pressed', 'false')

    // Picking a tab again lights it from where the lens was parked.
    await userEvent.click(screen.getByTestId('tab-items'))
    expect(lens.style.transform).toBe('translateX(0%)')
    expect(lens.className).not.toContain('opacity-0')
  })

  it('carries four tabs and keeps the archive in settings', async () => {
    seed()
    render(<Main />)

    expect(screen.getByTestId('tab-bar').querySelectorAll('button')).toHaveLength(4)
    expect(screen.queryByTestId('tab-archive')).not.toBeInTheDocument()

    await userEvent.click(screen.getByTestId('tab-settings'))
    await userEvent.click(screen.getByTestId('settings-archive'))

    expect(useUi.getState().view).toBe('archive')
    expect(useUi.getState().settings).toBe(false)
    expect(screen.getByTestId('list-title')).toHaveTextContent('Archive')
  })

  it('leaves the settings root when a list tab is tapped', async () => {
    seed()
    render(<Main />)

    act(() => openSettings())
    expect(screen.queryByTestId('entry-item')).not.toBeInTheDocument()

    await userEvent.click(screen.getByTestId('tab-items'))
    expect(useUi.getState().settings).toBe(false)
    expect(screen.getAllByTestId('entry-item')).toHaveLength(2)
  })

  it('opens the generator and settings from the tab bar', async () => {
    seed()
    render(<Main />)

    await userEvent.click(screen.getByTestId('tab-generator'))
    // A root screen rather than the sheet it used to be: no dialog, and the bar
    // it was opened from is still up.
    expect(screen.getByTestId('generator-screen')).toBeInTheDocument()
    expect(screen.queryByTestId('generator-dialog')).not.toBeInTheDocument()
    expect(screen.getByTestId('tab-generator')).toHaveAttribute('aria-pressed', 'true')
    expect(screen.getByTestId('tab-bar')).toBeInTheDocument()

    await userEvent.click(screen.getByTestId('tab-settings'))
    expect(useUi.getState().settings).toBe(true)
    // The two non-list roots are one slot: taking it closes the other.
    expect(useUi.getState().generator.open).toBe(false)
    expect(screen.queryByTestId('generator-screen')).not.toBeInTheDocument()
    // A root screen, with the bar still up: the tab bar is the way out of it.
    expect(screen.getByTestId('settings-nav-security')).toBeInTheDocument()
    expect(screen.getByTestId('tab-bar')).toBeInTheDocument()
  })

  it('generates and copies from the generator root, and leaves it by tab', async () => {
    seed()
    render(<Main />)

    await userEvent.click(screen.getByTestId('tab-generator'))
    expect(await screen.findByText('Generated123!')).toBeInTheDocument()

    // The bottom button is what the desktop card calls its confirm, in the
    // thumb's half of the screen.
    const use = screen.getByTestId('generator-use-button')
    expect(use).toHaveTextContent('Use & copy')
    await userEvent.click(use)
    expect(calls('copy_to_clipboard')).toContainEqual(
      { value: 'Generated123!', clearAfterMs: expect.any(Number) }
    )
    // Confirming is not leaving: a tab root is left through the tab bar.
    expect(screen.getByTestId('generator-screen')).toBeInTheDocument()

    await userEvent.click(screen.getByTestId('tab-items'))
    expect(useUi.getState().generator.open).toBe(false)
    expect(screen.getAllByTestId('entry-item')).toHaveLength(2)
  })

  // A root opened over a selection has to be the screen, or ⌘G with a row on
  // the detail sets `generator.open` and nothing visible happens.
  it('puts a root opened over a selected entry in front of it', async () => {
    mockCommand('reveal_entry', () => loginEntry({ id: 'l1', title: 'Google' }))
    seed()
    render(<Main />)

    await userEvent.click(screen.getByText('Google'))
    expect(screen.getByTestId('compact-back')).toBeInTheDocument()

    act(() => openGenerator())
    expect(screen.getByTestId('generator-screen')).toBeInTheDocument()
    expect(screen.queryByTestId('compact-back')).not.toBeInTheDocument()
    // A root, so the bar it is left by is back up.
    expect(screen.getByTestId('tab-bar')).toBeInTheDocument()

    act(() => openSettings())
    expect(screen.getByTestId('settings-nav-security')).toBeInTheDocument()
  })

  // The one thing that still outranks a root: a draft has unsaved work on it.
  it('keeps the form in front of a root opened under it', async () => {
    seed()
    render(<Main />)

    act(() => startEntry('login'))
    act(() => openSettings())
    expect(screen.getByTestId('save-entry-button')).toBeInTheDocument()
    expect(screen.queryByTestId('settings-nav-security')).not.toBeInTheDocument()
  })

  it('shows the audit score under the groups on the health view', () => {
    withEntries([loginMeta({ id: 'l1' }), loginMeta({ id: 'l2', title: 'Airbnb' })], audit)
    render(<Main />)

    act(() => setView('health'))
    // The list root is the only pane, so it carries what the wide shell puts
    // in its detail pane: the score panel under the groups, not just the groups.
    expect(screen.getByText('Password Audit')).toBeInTheDocument()
    expect(screen.getByText('Overall Score')).toBeInTheDocument()
    expect(screen.getByTestId('audit-stat-weak')).toHaveTextContent('1')
  })

  it('pushes a settings section and comes back to the root', async () => {
    seed()
    render(<Main />)

    act(() => openSettings())
    // Every section is a row, in the desktop nav's order.
    // A narrow desktop window, so Browser extension is among them.
    expect(screen.getAllByTestId(/^settings-nav-/)).toHaveLength(7)

    await userEvent.click(screen.getByTestId('settings-nav-security'))
    expect(screen.getByRole('heading', { name: 'Security' })).toBeInTheDocument()
    // One level deep, not a modal: the tab bar is still there.
    expect(screen.getByTestId('tab-bar')).toBeInTheDocument()
    expect(screen.queryByTestId('settings-nav-security')).not.toBeInTheDocument()

    await userEvent.click(screen.getByTestId('settings-back'))
    expect(screen.getByTestId('settings-nav-security')).toBeInTheDocument()
  })

  // A section body that sends the user elsewhere (the export pane's "save a
  // backup instead") has to move this shell's pushed pane, not the modal's
  // remembered section, which nothing on a phone is showing.
  it('follows the export pane’s backup link to the Sync pane', async () => {
    seed()
    render(<Main />)

    act(() => openSettings())
    await userEvent.click(screen.getByTestId('settings-nav-import'))
    await userEvent.click(screen.getByTestId('settings-io-export'))
    await userEvent.click(screen.getByTestId('settings-export-backup-link'))
    expect(screen.getByRole('heading', { name: 'Sync & backup' })).toBeInTheDocument()
  })

  // The pane's section is local state, so the store's guard on the wide
  // modal's close and nav does not reach it by itself — the Back has to honour
  // the same lock, or a restore from Drive could be walked away from here
  // (and would finish behind the user's back) while the desktop refuses it.
  it('holds a locked settings section against Back and the tab bar', async () => {
    seed()
    render(<Main />)

    act(() => openSettings())
    await userEvent.click(screen.getByTestId('settings-nav-workspaces'))
    expect(screen.getByRole('heading', { name: 'Workspaces' })).toBeInTheDocument()

    act(() => lockSettings(true))
    expect(screen.getByTestId('settings-back')).toBeDisabled()
    await userEvent.click(screen.getByTestId('settings-back'))
    expect(screen.getByRole('heading', { name: 'Workspaces' })).toBeInTheDocument()
    // Leaving Settings from the tab bar goes through the guarded `closeSettings`.
    await userEvent.click(screen.getByTestId('tab-items'))
    expect(useUi.getState().settings).toBe(true)
    expect(screen.getByRole('heading', { name: 'Workspaces' })).toBeInTheDocument()

    act(() => lockSettings(false))
    await userEvent.click(screen.getByTestId('settings-back'))
    expect(screen.getByTestId('settings-nav-workspaces')).toBeInTheDocument()
  })

  it('pushes a sub-page over its section, named after it, and comes back', async () => {
    seed()
    render(<Main />)

    act(() => openSettings())
    await userEvent.click(screen.getByTestId('settings-nav-workspaces'))
    await userEvent.click(screen.getByTestId('workspace-new-row'))

    expect(screen.getByRole('heading', { name: 'New workspace' })).toBeInTheDocument()
    expect(screen.getByTestId('settings-subpage-back')).toHaveTextContent('Workspaces')
    expect(screen.queryByTestId('settings-back')).not.toBeInTheDocument()

    await userEvent.click(screen.getByTestId('settings-subpage-back'))
    expect(screen.getByRole('heading', { name: 'Workspaces' })).toBeInTheDocument()
    expect(screen.getByTestId('settings-back')).toBeInTheDocument()
  })

  // The chip's default is the wide modal's section state, which a pushed pane
  // does not read — on this shell it navigates the way the rows do.
  it('opens the Sync pane from the sync chip on the settings root', async () => {
    seed()
    render(<Main />)

    act(() => openSettings())
    await userEvent.click(screen.getByTestId('sync-indicator'))

    expect(screen.getByTestId('settings-back')).toBeInTheDocument()
    expect(screen.getByRole('heading', { name: 'Sync & backup' })).toBeInTheDocument()
  })

  it('locks the vault from the settings root', async () => {
    seed()
    render(<Main />)

    act(() => openSettings())
    await userEvent.click(screen.getByTestId('lock-vault-button'))

    expect(calls('lock')).toHaveLength(1)
  })

  it('shows the empty-vault hero on the one pane it has', () => {
    withEntries([])
    render(<Main />)

    expect(screen.getAllByText('Your vault is empty')).toHaveLength(1)
  })

  it('leaves the command palette out', () => {
    seed()
    render(<Main />)

    act(() => openPalette())
    expect(screen.queryByPlaceholderText('Run a command')).not.toBeInTheDocument()
  })
})

describe('overlay frames', () => {
  it('makes settings a screen on compact and the modal on wide', () => {
    seed()
    const { unmount } = render(<Main />)
    act(() => openSettings())
    // A tab root rather than an overlay: no sheet over the list, and the
    // sections it navigates are in the shell itself.
    expect(screen.queryByTestId('settings-modal')).not.toBeInTheDocument()
    expect(screen.getByTestId('compact-shell')).toContainElement(
      screen.getByTestId('settings-nav-security')
    )
    unmount()

    setLayout('wide')
    seed()
    render(<Main />)
    act(() => openSettings())
    expect(screen.getByTestId('settings-modal')).not.toHaveAttribute('data-frame')
  })

  // `fit="content"`: the picker is short, so the phone answers it from the
  // bottom edge instead of giving it a page.
  it('gives the add picker a bottom sheet on compact and the card on wide', () => {
    seed()
    const { unmount } = render(<Main />)
    act(() => openAddPicker())
    expect(screen.getByTestId('add-secret-modal')).toHaveAttribute('data-frame', 'bottom-sheet')
    unmount()

    setLayout('wide')
    seed()
    render(<Main />)
    act(() => openAddPicker())
    expect(screen.getByTestId('add-secret-modal')).not.toHaveAttribute('data-frame')
  })

  it('keeps the page sheet for the generator a password row opens', () => {
    seed()
    render(<Main />)
    // What the login form's generate action does: a full dialog with a callback
    // to fill, which needs the whole screen rather than a card off the edge.
    act(() => openGenerator(() => {}))
    const sheet = screen.getByTestId('generator-dialog')
    expect(sheet).toHaveAttribute('data-frame', 'sheet')
    // Outside the shell, like every fixed overlay: the shell carries the
    // visual-viewport translate, so a `fixed` child would be laid out against
    // it and take the keyboard offset a second time.
    expect(screen.getByTestId('compact-shell')).not.toContainElement(sheet)
  })

  it('makes the standalone generator a screen on compact and the card on wide', () => {
    seed()
    const { unmount } = render(<Main />)
    act(() => openGenerator())
    // Nothing is waiting for the value, so there is nothing to overlay: it is a
    // tab root of its own.
    expect(screen.queryByTestId('generator-dialog')).not.toBeInTheDocument()
    expect(screen.getByTestId('compact-shell')).toContainElement(
      screen.getByTestId('generator-screen')
    )
    unmount()

    setLayout('wide')
    seed()
    render(<Main />)
    act(() => openGenerator())
    const card = screen.getByTestId('generator-dialog')
    expect(card).not.toHaveAttribute('data-frame')
    // The e2e suite and the dialogs' own topmost check (`useDialogFocus`,
    // `Generator/useDialogKeys`) both read the card off the DOM.
    expect(card).toHaveAttribute('role', 'dialog')
    expect(card).toHaveAttribute('aria-labelledby', 'generator-title')
  })
})

// A sync merge replaces the list wholesale, and the selection is re-resolved
// against it. Losing the selection has to end the edit of it too: the compact
// form is a whole screen, and one with no subject is a blank one with no exit.
describe('entries slice', () => {
  it('ends an edit when the row being edited falls out of the list', () => {
    withEntries([loginMeta({ id: 'l1' }), loginMeta({ id: 'l2', title: 'Airbnb' })])
    setCurrentEntry('l1')
    editEntry()
    expect(useVault.getState().editing).toBe(true)

    setEntries([loginMeta({ id: 'l2', title: 'Airbnb' })])
    expect(useVault.getState().currentId).toBeNull()
    expect(useVault.getState().editing).toBe(false)
  })

  it('keeps the edit when the row survives the merge', () => {
    withEntries([loginMeta({ id: 'l1' })])
    setCurrentEntry('l1')
    editEntry()

    setEntries([loginMeta({ id: 'l1', title: 'Google Mail' })])
    expect(selectCurrent(useVault.getState())?.title).toBe('Google Mail')
    expect(useVault.getState().editing).toBe(true)
  })
})

// The frame a dialog gets is context, not a layout question it asks itself.
describe('Frame', () => {
  const dialog = (
    <Frame onClose={() => {}} testid="framed">
      <button type="button">inside</button>
    </Frame>
  )

  it('is the card by default and whatever the shell provides otherwise', () => {
    const { unmount } = render(dialog)
    expect(screen.getByTestId('framed')).not.toHaveAttribute('data-frame')
    unmount()

    render(<FrameProvider value={Sheet}>{dialog}</FrameProvider>)
    expect(screen.getByTestId('framed')).toHaveAttribute('data-frame', 'sheet')
  })
})

describe('AuthShell on compact', () => {
  it('renders its column and the back affordance', () => {
    const onBack = vi.fn()
    render(
      <AuthShell onBack={onBack}>
        <div>Unlock</div>
      </AuthShell>
    )

    expect(screen.getByText('Unlock')).toBeInTheDocument()
    expect(screen.getByTestId('go-back-button')).toBeInTheDocument()
  })
})
