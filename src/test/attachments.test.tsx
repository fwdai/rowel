import { describe, it, expect, beforeEach, vi } from 'vitest'
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
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

  // The row carries no controls on either shell: the file opens, and saving
  // and removing are the preview's.
  it('saves from the open file, by id', async () => {
    mockCommand('attachment_list', () => [file()])
    render(<Show entry={loginMeta()} />)

    const row = await screen.findByTestId('attachment-row')
    expect(row.querySelectorAll('button')).toHaveLength(1)
    await userEvent.click(screen.getByTestId('attachment-open-button'))
    await userEvent.click(screen.getByTestId('preview-save-button'))

    expect(calls('attachment_save')).toEqual([{ id: 'a1' }])
  })

  it('asks before removing, naming the file, then removes and closes', async () => {
    seedAttachments([file()])
    render(<Show entry={loginMeta()} />)

    await userEvent.click(await screen.findByTestId('attachment-open-button'))
    await userEvent.click(screen.getByTestId('preview-remove-button'))
    expect(calls('attachment_delete')).toEqual([])
    const sheet = screen.getByTestId('attachment-remove-sheet')
    expect(sheet).toHaveTextContent('Remove “contract.pdf”?')
    const confirm = screen.getByTestId('attachment-remove-confirm')
    expect(confirm).toHaveTextContent('Remove attachment')

    await userEvent.click(confirm)

    expect(calls('attachment_delete')).toEqual([{ id: 'a1' }])
    expect(screen.queryByTestId('attachment-preview')).not.toBeInTheDocument()
    await waitFor(() =>
      expect(screen.queryByTestId('attachments-section')).not.toBeInTheDocument()
    )
    expect(screen.getByTestId('attach-file-button')).toBeInTheDocument()
  })

  it('backs out of removing without removing, and the file stays open', async () => {
    seedAttachments([file()])
    render(<Show entry={loginMeta()} />)

    await userEvent.click(await screen.findByTestId('attachment-open-button'))
    await userEvent.click(screen.getByTestId('preview-remove-button'))
    await userEvent.click(screen.getByTestId('confirm-cancel'))

    await waitFor(() =>
      expect(screen.queryByTestId('attachment-remove-sheet')).not.toBeInTheDocument()
    )
    expect(calls('attachment_delete')).toEqual([])
    expect(screen.getByTestId('attachment-preview')).toBeInTheDocument()
  })
})

const bytesOf = (text: string) => new TextEncoder().encode(text).buffer

describe('opening a file', () => {
  // A row draws the backend's small thumbnail, never the file: the file is read
  // only when it is opened. (A list of pictures used to be the pictures, whole.)
  it('draws an image from its thumbnail and reads the file only when opened', async () => {
    seedAttachments([file({ name: 'IMG_0971.png', mime: 'image/png', size: 686_080 })])
    mockCommand('attachment_thumbnail', () => bytesOf('small png'))
    mockCommand('attachment_read', () => bytesOf('the whole png'))
    render(<Show entry={loginMeta()} />)

    const thumb = await screen.findByTestId('attachment-thumb')
    expect(thumb).toHaveAttribute('src', expect.stringMatching(/^blob:/))
    expect(calls('attachment_thumbnail')).toEqual([{ id: 'a1' }])
    expect(calls('attachment_read')).toEqual([])
    expect(screen.getByTestId('attachment-row')).toHaveTextContent('670 KB · PNG')

    await userEvent.click(screen.getByTestId('attachment-open-button'))

    const preview = screen.getByTestId('attachment-preview')
    expect(preview).toHaveTextContent('IMG_0971.png')
    const image = await screen.findByTestId('preview-image')
    expect(image).toHaveAttribute('src', expect.stringMatching(/^blob:/))
    expect(image.getAttribute('src')).not.toBe(thumb.getAttribute('src'))
    expect(calls('attachment_read')).toEqual([{ id: 'a1' }])

    await userEvent.click(screen.getByTestId('attachment-preview-close'))
    expect(screen.queryByTestId('attachment-preview')).not.toBeInTheDocument()
  })

  // A picture the backend cannot make a thumbnail of (HEIC) keeps the kind's
  // glyph in its row, and so does one the engine will not draw.
  it('shows the kind’s glyph when there is no thumbnail to draw', async () => {
    seedAttachments([file({ name: 'IMG_0971.heic', mime: 'image/heic' })])
    mockCommand('attachment_thumbnail', () =>
      Promise.reject({ kind: 'unsupported', message: 'no codec' })
    )
    render(<Show entry={loginMeta()} />)

    await screen.findByTestId('attachment-row')
    await waitFor(() => expect(calls('attachment_thumbnail')).toHaveLength(1))
    expect(screen.queryByTestId('attachment-thumb')).not.toBeInTheDocument()

    mockCommand('attachment_thumbnail', () => bytesOf('png'))
    seedAttachments([file({ id: 'a2', name: 'odd.png', mime: 'image/png' })])
    act(() => setEntries([loginMeta()]))
    const thumb = await screen.findByTestId('attachment-thumb')
    fireEvent.error(thumb)
    await waitFor(() => expect(screen.queryByTestId('attachment-thumb')).not.toBeInTheDocument())
  })

  // A file attached before its format was in the backend's table has no type;
  // its name still says what it is, and the bytes are held under that type so
  // the document viewer knows what it is looking at.
  it('treats a typeless file as what its name says it is', async () => {
    seedAttachments([
      file({ id: 'a1', name: 'scan.tif', mime: undefined }),
      file({ id: 'a2', name: 'old.pdf', mime: undefined })
    ])
    mockCommand('attachment_thumbnail', () => bytesOf('png'))
    mockCommand('attachment_read', () => bytesOf('%PDF-1.4'))
    const made = vi.spyOn(URL, 'createObjectURL')
    render(<Show entry={loginMeta()} />)

    const [tif, pdf] = await screen.findAllByTestId('attachment-row')
    await waitFor(() => expect(tif.querySelector('[data-testid="attachment-thumb"]')).toBeTruthy())
    expect(calls('attachment_thumbnail')).toEqual([{ id: 'a1' }])

    await userEvent.click(pdf.querySelector('[data-testid="attachment-open-button"]')!)
    expect(await screen.findByTestId('preview-document')).toBeInTheDocument()
    const held = made.mock.calls[made.mock.calls.length - 1][0] as Blob
    expect(held.type).toBe('application/pdf')
    made.mockRestore()
  })

  // What a TIFF comes to on an engine that cannot decode one: the backend's PNG
  // of it, shown in its place. A PNG that will not decode either is given up on.
  it('falls back to the backend’s PNG when the engine cannot decode a picture', async () => {
    seedAttachments([file({ name: 'scan.tiff', mime: 'image/tiff' })])
    mockCommand('attachment_read', () => bytesOf('tiff'))
    mockCommand('attachment_render', () => bytesOf('png'))
    render(<Show entry={loginMeta()} />)

    await userEvent.click(await screen.findByTestId('attachment-open-button'))
    const image = await screen.findByTestId('preview-image')
    const original = image.getAttribute('src')
    fireEvent.error(image)

    await waitFor(() => expect(calls('attachment_render')).toEqual([{ id: 'a1' }]))
    const rendered = await screen.findByTestId('preview-image')
    await waitFor(() => expect(rendered.getAttribute('src')).not.toBe(original))

    fireEvent.error(screen.getByTestId('preview-image'))
    expect(await screen.findByTestId('preview-unavailable')).toBeInTheDocument()
    expect(calls('attachment_render')).toHaveLength(1)
  })

  it('reads a text file only when opened, and shows it as text', async () => {
    seedAttachments([file({ name: 'notes.txt', mime: 'text/plain', size: 11 })])
    mockCommand('attachment_read', () => bytesOf('hello world'))
    render(<Show entry={loginMeta()} />)

    await screen.findByTestId('attachment-row')
    expect(calls('attachment_read')).toEqual([])

    await userEvent.click(screen.getByTestId('attachment-open-button'))

    expect(await screen.findByTestId('preview-text')).toHaveTextContent('hello world')
    expect(calls('attachment_read')).toEqual([{ id: 'a1' }])
  })

  it('says when a file cannot be shown, and still saves it', async () => {
    seedAttachments([file({ name: 'keys.zip', mime: 'application/zip' })])
    render(<Show entry={loginMeta()} />)

    await userEvent.click(await screen.findByTestId('attachment-open-button'))

    expect(screen.getByTestId('preview-unavailable')).toBeInTheDocument()
    expect(calls('attachment_read')).toEqual([])
    await userEvent.click(screen.getByTestId('preview-save-button'))
    expect(calls('attachment_save')).toEqual([{ id: 'a1' }])
  })

  it('says when the file would not open', async () => {
    seedAttachments([file()])
    mockCommand('attachment_read', () => Promise.reject({ kind: 'other', message: 'nope' }))
    render(<Show entry={loginMeta()} />)

    await userEvent.click(await screen.findByTestId('attachment-open-button'))

    expect(await screen.findByRole('alert')).toHaveTextContent('Could not open this file.')
  })

  it('closes on a press on the ground beside the file, not on the file', async () => {
    seedAttachments([file({ name: 'IMG_0971.png', mime: 'image/png' })])
    mockCommand('attachment_read', () => bytesOf('png'))
    render(<Show entry={loginMeta()} />)

    await userEvent.click(await screen.findByTestId('attachment-open-button'))
    const image = await screen.findByTestId('preview-image')
    await userEvent.click(image)
    expect(screen.getByTestId('attachment-preview')).toBeInTheDocument()

    await userEvent.click(image.parentElement!)
    expect(screen.queryByTestId('attachment-preview')).not.toBeInTheDocument()
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

      await userEvent.click(screen.getByTestId('attachment-open-button'))
      await userEvent.click(screen.getByTestId('preview-remove-button'))
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
