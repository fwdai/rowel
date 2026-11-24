import { useState } from 'react'
import { describe, it, expect, beforeEach, vi } from 'vitest'
import { act, render, renderHook, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { FieldsProvider } from '@/components/elements/fields'
import type { DraftValue, EntryDraft } from '@/kinds/draft'
import Fields from '@/kinds/env/Fields'
import { useEnvIngest } from '@/kinds/env/useIngest'
import Main from '@/components/Main'
import { useUi, useVault, startEntry, openAddPicker } from '@/store'
import { withEntries, loginMeta, deferred } from './utils'
import { calls, mockCommand } from './ipc'

// The webview's drag-drop stream, replaced by a hand that can drop a file —
// setup.ts's mock is the inert one. A test waits for the listener before it
// drops; `unlisten` takes it back out.
type Handler = (event: { payload: { type: string; paths: string[] } }) => void
let handlers: Handler[] = []
vi.mock('@tauri-apps/api/webview', () => ({
  getCurrentWebview: () => ({
    onDragDropEvent: (handler: Handler) => {
      handlers.push(handler)
      return Promise.resolve(() => {
        handlers = handlers.filter(h => h !== handler)
      })
    }
  })
}))

const listening = () => waitFor(() => expect(handlers.length).toBeGreaterThan(0))

const drop = async (path: string) => {
  await listening()
  await act(async () => {
    for (const handler of [...handlers]) handler({ payload: { type: 'drop', paths: [path] } })
  })
}

const MINE = ['# Database', 'DATABASE_URL=postgres://mine', 'DB_POOL=10', ''].join('\n')
const THEIRS = ['DB_POOL=99', 'STRIPE_KEY=sk_live_1', ''].join('\n')
const FILE = { fileName: '.env.production', body: THEIRS }

const keyInputs = () =>
  Array.from(document.querySelectorAll<HTMLInputElement>('input[name^="env-key-"]')).map(
    el => el.value
  )

// Editing against a live draft, the way Show's editor holds one, with every
// write reported so the test can see the keys the zone set.
function Editor({
  body,
  title = '',
  onSet
}: {
  body: string
  title?: string
  onSet: (name: string, value: DraftValue) => void
}) {
  const [entry, setEntry] = useState<EntryDraft>({ type: 'env', title, body, fileName: '', note: '' })
  const set = (name: string, next: DraftValue) => {
    onSet(name, next)
    setEntry(prev => ({ ...prev, [name]: next }))
  }
  return (
    <FieldsProvider value={{ entry, set, attempted: false }}>
      <Fields />
      <button data-testid="edit-env-body" onClick={() => set('body', 'LOCAL=mine\n')} />
    </FieldsProvider>
  )
}

beforeEach(() => {
  vi.clearAllMocks()
  handlers = []
  mockCommand('read_env_file', () => FILE)
})

describe('the editor drop zone', () => {
  it('is only there while editing', () => {
    render(
      <FieldsProvider
        value={{ entry: { type: 'env', title: 'x', body: MINE, note: '' }, set: null, attempted: false }}
      >
        <Fields />
      </FieldsProvider>
    )
    expect(screen.queryByTestId('env-dropzone')).not.toBeInTheDocument()
  })

  it('fills an empty draft with the file, its name and a proposed title', async () => {
    const onSet = vi.fn()
    render(<Editor body="" onSet={onSet} />)
    expect(screen.getByTestId('env-dropzone')).toHaveTextContent('Drop a .env file here')

    await drop('/Users/me/code/api/.env.production')

    await waitFor(() => expect(onSet).toHaveBeenCalledWith('body', THEIRS))
    expect(onSet).toHaveBeenCalledWith('fileName', '.env.production')
    expect(onSet).toHaveBeenCalledWith('title', 'api · production')
    expect(calls('read_env_file')).toContainEqual({ path: '/Users/me/code/api/.env.production' })
    expect(keyInputs()).toEqual(['DB_POOL', 'STRIPE_KEY'])
  })

  it('leaves a title that was already typed', async () => {
    const onSet = vi.fn()
    render(<Editor body="" title="Mine" onSet={onSet} />)

    await drop('/Users/me/code/api/.env')

    await waitFor(() => expect(onSet).toHaveBeenCalledWith('body', THEIRS))
    expect(onSet).not.toHaveBeenCalledWith('title', expect.anything())
  })

  it('rechecks the live draft after a pending read instead of replacing newer typing', async () => {
    const reading = deferred<typeof FILE>()
    mockCommand('read_env_file', () => reading.promise)
    const onSet = vi.fn()
    render(<Editor body="" onSet={onSet} />)

    await drop('/Users/me/code/api/.env')
    await userEvent.click(screen.getByTestId('edit-env-body'))
    onSet.mockClear()

    await act(async () => reading.resolve(FILE))

    expect(await screen.findByTestId('env-drop-prompt')).toBeInTheDocument()
    expect(onSet).not.toHaveBeenCalled()
    expect(keyInputs()).toEqual(['LOCAL'])
  })

  it('asks before touching a draft that has variables, and replaces on request', async () => {
    const onSet = vi.fn()
    render(<Editor body={MINE} onSet={onSet} />)

    await drop('/Users/me/code/api/.env.production')

    const prompt = await screen.findByTestId('env-drop-prompt')
    expect(prompt).toHaveTextContent('.env.production')
    expect(onSet).not.toHaveBeenCalled()
    expect(keyInputs()).toEqual(['DATABASE_URL', 'DB_POOL'])

    await userEvent.click(screen.getByTestId('env-drop-replace'))

    expect(onSet).toHaveBeenCalledWith('body', THEIRS)
    expect(onSet).toHaveBeenCalledWith('fileName', '.env.production')
    expect(keyInputs()).toEqual(['DB_POOL', 'STRIPE_KEY'])
    expect(screen.queryByTestId('env-drop-prompt')).not.toBeInTheDocument()
  })

  it('merges only the new keys, leaving existing rows alone', async () => {
    const onSet = vi.fn()
    render(<Editor body={MINE} onSet={onSet} />)

    await drop('/Users/me/code/api/.env.production')
    await userEvent.click(await screen.findByTestId('env-drop-merge'))

    expect(onSet).toHaveBeenCalledWith('body', MINE + 'STRIPE_KEY=sk_live_1\n')
    expect(onSet).toHaveBeenCalledWith('fileName', '.env.production')
    expect(keyInputs()).toEqual(['DATABASE_URL', 'DB_POOL', 'STRIPE_KEY'])
    // The rotated value stays ours.
    expect(document.querySelector<HTMLTextAreaElement>('textarea[name="env-value-2"]')).toHaveValue(
      '10'
    )
  })

  it('can be told no, with the × or with Escape — which the editor never hears', async () => {
    const onSet = vi.fn()
    const heard = vi.fn()
    document.addEventListener('keydown', heard)
    render(<Editor body={MINE} onSet={onSet} />)

    await drop('/Users/me/code/api/.env.production')
    await userEvent.click(await screen.findByTestId('env-drop-cancel'))
    expect(screen.queryByTestId('env-drop-prompt')).not.toBeInTheDocument()

    await drop('/Users/me/code/api/.env.production')
    expect(screen.getByTestId('env-drop-replace')).toHaveFocus()
    await userEvent.keyboard('{Escape}')
    expect(screen.queryByTestId('env-drop-prompt')).not.toBeInTheDocument()

    expect(onSet).not.toHaveBeenCalled()
    expect(heard).not.toHaveBeenCalled()
    document.removeEventListener('keydown', heard)
  })

  it('shows why a file was refused', async () => {
    mockCommand('read_env_file', () =>
      Promise.reject({ kind: 'fileTooLarge', message: 'the file is too large' })
    )
    const onSet = vi.fn()
    render(<Editor body="" onSet={onSet} />)

    await drop('/Users/me/huge.env')

    expect(await screen.findByTestId('env-drop-error')).toHaveTextContent('This file is too large')
    expect(onSet).not.toHaveBeenCalled()
  })

  it('declines image and unrelated-text drops so their owning surfaces can handle them', async () => {
    mockCommand('read_env_file', () => ({ fileName: 'notes.txt', body: 'hello\nworld\n' }))
    const onSet = vi.fn()
    render(<Editor body={MINE} onSet={onSet} />)

    await drop('/Users/me/notes.txt')
    await waitFor(() => expect(calls('read_env_file')).toContainEqual({ path: '/Users/me/notes.txt' }))
    await drop('/Users/me/card.png')

    expect(calls('read_env_file')).not.toContainEqual({ path: '/Users/me/card.png' })
    expect(screen.queryByTestId('env-drop-prompt')).not.toBeInTheDocument()
    expect(screen.queryByTestId('env-drop-error')).not.toBeInTheDocument()
    expect(onSet).not.toHaveBeenCalled()
  })
})

describe('interactive env ingestion', () => {
  it('surfaces picker failures through the shared request lifecycle', async () => {
    mockCommand('pick_file', () =>
      Promise.reject({ kind: 'fileTooLarge', message: 'the file is too large' })
    )
    const consume = vi.fn()
    const { result } = renderHook(() => useEnvIngest(consume))

    await act(() => result.current.pick())

    expect(result.current.error).toContain('This file is too large')
    expect(consume).not.toHaveBeenCalled()
  })

  it('reads the file the backend dialog gave it, unfiltered', async () => {
    mockCommand('pick_file', () => '/Users/me/code/api/.env.production')
    mockCommand('read_env_file', () => FILE)
    const consume = vi.fn()
    const { result } = renderHook(() => useEnvIngest(consume))

    await act(() => result.current.pick())

    expect(calls('pick_file')).toContainEqual({ kind: 'env', label: null })
    expect(calls('read_env_file')).toContainEqual({ path: '/Users/me/code/api/.env.production' })
    expect(consume).toHaveBeenCalledTimes(1)
  })

  it('lets only the newest read publish a result', async () => {
    const first = deferred<typeof FILE>()
    const second = deferred<typeof FILE>()
    mockCommand('read_env_file', ({ path }) =>
      (path as string).endsWith('first.env') ? first.promise : second.promise
    )
    const consume = vi.fn()
    const { result } = renderHook(() => useEnvIngest(consume))

    let firstRun: Promise<void>
    let secondRun: Promise<void>
    act(() => {
      firstRun = result.current.drop('/tmp/first.env')
      secondRun = result.current.drop('/tmp/second.env')
    })
    await act(async () => {
      second.resolve({ fileName: 'second.env', body: 'SECOND=2\n' })
      await secondRun
      first.resolve({ fileName: 'first.env', body: 'FIRST=1\n' })
      await firstRun
    })

    expect(consume).toHaveBeenCalledTimes(1)
    expect(consume.mock.calls[0][0]).toMatchObject({ fileName: 'second.env', body: 'SECOND=2\n' })
  })
})

describe('a .env dropped on the idle window', () => {
  const seed = () => withEntries([loginMeta({ id: 'l1', title: 'Google' })])

  it('opens a new env draft with the file already in it', async () => {
    seed()
    render(<Main />)

    await drop('/Users/me/code/api/.env.production')

    await waitFor(() => expect(useVault.getState().creating).toBe('env'))
    await waitFor(() => expect(keyInputs()).toEqual(['DB_POOL', 'STRIPE_KEY']))
    // Seeded through the same prefill a scan uses, and consumed the same way.
    expect(useVault.getState().prefill).toBeNull()
    expect(document.querySelector('input[name="title"]')).toHaveValue('api · production')
  })

  it('takes a file that only reads like one', async () => {
    mockCommand('read_env_file', () => ({ fileName: 'keys.txt', body: 'A=1\nB=2\n' }))
    seed()
    render(<Main />)

    await drop('/Users/me/keys.txt')

    await waitFor(() => expect(useVault.getState().creating).toBe('env'))
  })

  it('ignores anything that is neither named nor written like one', async () => {
    mockCommand('read_env_file', () => ({ fileName: 'notes.txt', body: 'hello\nworld\n' }))
    seed()
    render(<Main />)

    await drop('/Users/me/notes.txt')
    await drop('/Users/me/card.png')

    expect(useVault.getState().creating).toBeNull()
    // The scanner's, so not even read.
    expect(calls('read_env_file')).not.toContainEqual({ path: '/Users/me/card.png' })
  })

  it('leaves an open editor alone', async () => {
    seed()
    startEntry('login')
    render(<Main />)

    await drop('/Users/me/code/api/.env')

    expect(useVault.getState().creating).toBe('login')
    expect(calls('read_env_file')).toHaveLength(0)
  })

  it('does not replace an editor opened while the file is still being read', async () => {
    const reading = deferred<typeof FILE>()
    mockCommand('read_env_file', () => reading.promise)
    seed()
    render(<Main />)

    await drop('/Users/me/code/api/.env')
    act(() => startEntry('login'))
    await act(async () => reading.resolve(FILE))

    expect(useVault.getState().creating).toBe('login')
  })

  it('answers the Add picker and closes it', async () => {
    seed()
    render(<Main />)
    act(() => openAddPicker())
    expect(screen.getByTestId('add-secret-modal')).toBeInTheDocument()

    await drop('/Users/me/code/api/.env')

    await waitFor(() => expect(useVault.getState().creating).toBe('env'))
    expect(useUi.getState().addPicker).toBe(false)
  })
})
