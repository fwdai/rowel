import { describe, it, expect, beforeEach, vi } from 'vitest'
import { act, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import Show from '@/components/Main/Body/Aside/Show'
import Main from '@/components/Main'
import { initialApp, setCurrentEntry, setEntries, setSyncStatus } from '@/store'
import type { Attachment } from '@/api/types'
import { loginEntry, loginMeta, withEntries } from './utils'
import { calls, mockCommand, seedAttachments } from './ipc'

// The drag-drop stream, with a hand that can drop a file (see envIngest.test).
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

const drop = async (path: string) => {
  await waitFor(() => expect(handlers.length).toBeGreaterThan(0))
  await act(async () => {
    for (const handler of [...handlers]) handler({ payload: { type: 'drop', paths: [path] } })
  })
}

const file = (overrides: Partial<Attachment> = {}): Attachment => ({
  id: 'a1',
  entryId: 'l1',
  name: 'contract.pdf',
  mime: 'application/pdf',
  size: 1536,
  createdAt: '2024-01-01T00:00:00.000Z',
  ...overrides
})

const MB = 1024 * 1024

beforeEach(() => {
  handlers = []
  mockCommand('reveal_entry', () => loginEntry())
})

describe('attachments on an entry', () => {
  it('shows only a quiet attach action on an entry with none', async () => {
    render(<Show entry={loginMeta()} />)

    expect(await screen.findByTestId('attach-file-button')).toHaveTextContent('Attach file')
    await waitFor(() => expect(calls('attachment_list')).toContainEqual({ entryId: 'l1' }))
    expect(screen.queryByTestId('attachments-section')).not.toBeInTheDocument()
    expect(screen.queryByText('Attachments')).not.toBeInTheDocument()
  })

  it('offers nothing on an env file entry', async () => {
    mockCommand('reveal_entry', () => ({ id: 'l1', type: 'env', title: 'api', body: 'A=1\n' }))
    render(<Show entry={loginMeta({ type: 'env' })} />)

    await screen.findByTestId('entry-footer')
    await waitFor(() => expect(screen.queryByTestId('attachments')).not.toBeInTheDocument())
    expect(calls('attachment_list')).toEqual([])
  })

  it('lists a picked file once it is attached', async () => {
    mockCommand('pick_file', () => '/Users/me/contract.pdf')
    render(<Show entry={loginMeta()} />)

    await userEvent.click(await screen.findByTestId('attach-file-button'))

    expect(calls('pick_file')).toContainEqual({ kind: 'attachment', label: null })
    expect(calls('attachment_add')).toContainEqual({
      entryId: 'l1',
      path: '/Users/me/contract.pdf'
    })
    const section = await screen.findByTestId('attachments-section')
    expect(section).toHaveTextContent('Attachments')
    expect(section).toHaveTextContent('contract.pdf')
    expect(section).toHaveTextContent('2 KB')
  })

  it('attaches nothing when the dialog is dismissed', async () => {
    render(<Show entry={loginMeta()} />)

    await userEvent.click(await screen.findByTestId('attach-file-button'))

    expect(calls('attachment_add')).toEqual([])
    expect(screen.queryByTestId('attachments-section')).not.toBeInTheDocument()
  })

  it('says how large a file may be when one is refused', async () => {
    mockCommand('pick_file', () => '/Users/me/huge.iso')
    mockCommand('attachment_add', () =>
      Promise.reject({ kind: 'fileTooLarge', message: 'the file is too large' })
    )
    render(<Show entry={loginMeta()} />)

    await userEvent.click(await screen.findByTestId('attach-file-button'))

    expect(await screen.findByRole('alert')).toHaveTextContent(
      'Attachments can be up to 10 MB each'
    )
    expect(screen.queryByTestId('attachments-section')).not.toBeInTheDocument()
  })

  it('saves through the backend, by id', async () => {
    mockCommand('attachment_list', () => [file()])
    render(<Show entry={loginMeta()} />)

    await userEvent.click(await screen.findByTestId('attachment-save-button'))

    expect(calls('attachment_save')).toEqual([{ id: 'a1' }])
  })

  it('asks before removing, then removes', async () => {
    seedAttachments([file()])
    render(<Show entry={loginMeta()} />)

    await userEvent.click(await screen.findByTestId('attachment-remove-button'))
    expect(calls('attachment_delete')).toEqual([])
    const confirm = screen.getByTestId('attachment-remove-confirm')
    expect(confirm).toHaveTextContent('Remove?')

    await userEvent.click(confirm)

    expect(calls('attachment_delete')).toEqual([{ id: 'a1' }])
    await waitFor(() =>
      expect(screen.queryByTestId('attachments-section')).not.toBeInTheDocument()
    )
    expect(screen.getByTestId('attach-file-button')).toBeInTheDocument()
  })
})

describe('the files of an open entry', () => {
  // Another device attached a file, then removed it. Each merge replaces the
  // entry list while the same entry stays open, and the files follow.
  it('are reread when a sync merge lands', async () => {
    render(<Show entry={loginMeta()} />)
    await waitFor(() => expect(calls('attachment_list')).toHaveLength(1))

    mockCommand('attachment_list', () => [file()])
    act(() => setEntries([loginMeta()]))
    expect(await screen.findByTestId('attachments-section')).toHaveTextContent('contract.pdf')

    mockCommand('attachment_list', () => [])
    act(() => setEntries([loginMeta()]))
    await waitFor(() =>
      expect(screen.queryByTestId('attachments-section')).not.toBeInTheDocument()
    )
  })

  it('are published once one is attached or removed', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true })
    try {
      setSyncStatus({ ...initialApp.sync, configured: true })
      mockCommand('pick_file', () => '/Users/me/contract.pdf')
      render(<Show entry={loginMeta()} />)

      await userEvent.click(await screen.findByTestId('attach-file-button'))
      await screen.findByTestId('attachments-section')
      await vi.advanceTimersByTimeAsync(2_000)
      expect(calls('sync_now')).toHaveLength(1)

      await userEvent.click(screen.getByTestId('attachment-remove-button'))
      await userEvent.click(screen.getByTestId('attachment-remove-confirm'))
      await waitFor(() => expect(calls('attachment_delete')).toHaveLength(1))
      await vi.advanceTimersByTimeAsync(2_000)
      expect(calls('sync_now')).toHaveLength(2)
    } finally {
      vi.useRealTimers()
    }
  })
})

describe("the vault's attachment budget", () => {
  it('goes unmentioned while there is room', async () => {
    mockCommand('attachment_list', () => [file()])
    mockCommand('attachment_usage', () => ({ used: 100 * MB, limit: 128 * MB }))
    render(<Show entry={loginMeta()} />)

    await screen.findByTestId('attachments-section')
    await waitFor(() => expect(calls('attachment_usage')).not.toHaveLength(0))
    expect(screen.queryByTestId('attachments-usage')).not.toBeInTheDocument()
  })

  it('is mentioned under the files once it is nearly spent', async () => {
    mockCommand('attachment_list', () => [file()])
    mockCommand('attachment_usage', () => ({ used: 120 * MB, limit: 128 * MB }))
    render(<Show entry={loginMeta()} />)

    expect(await screen.findByTestId('attachments-usage')).toHaveTextContent(
      '120 MB of 128 MB used by attachments in this vault'
    )
  })

  it('says so when a file would take the vault past it', async () => {
    mockCommand('pick_file', () => '/Users/me/scan.pdf')
    mockCommand('attachment_add', () =>
      Promise.reject({ kind: 'vaultFull', message: "this vault's attachments are at their limit" })
    )
    render(<Show entry={loginMeta()} />)

    await userEvent.click(await screen.findByTestId('attach-file-button'))

    expect(await screen.findByRole('alert')).toHaveTextContent(
      "This vault's attachments are at their limit (128 MB)"
    )
  })
})

describe('a file dropped on an open entry', () => {
  const open = () => {
    withEntries([loginMeta()])
    setCurrentEntry('l1')
    render(<Main />)
  }

  it('is attached to it, and never read as an env file', async () => {
    open()
    await screen.findByTestId('attach-file-button')

    await drop('/Users/me/contract.pdf')

    await waitFor(() =>
      expect(calls('attachment_add')).toContainEqual({
        entryId: 'l1',
        path: '/Users/me/contract.pdf'
      })
    )
    expect(calls('read_env_file')).toEqual([])
    expect(await screen.findByTestId('attachments-section')).toHaveTextContent('contract.pdf')
  })

  it('leaves a .env to the env flow', async () => {
    open()
    await screen.findByTestId('attach-file-button')

    await drop('/Users/me/code/api/.env')

    await waitFor(() => expect(calls('read_env_file')).toHaveLength(1))
    expect(calls('attachment_add')).toEqual([])
  })
})
