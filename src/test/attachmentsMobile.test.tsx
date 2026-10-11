import { describe, it, expect, beforeEach, vi } from 'vitest'
import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import Show from '@/components/Main/Body/Aside/Show'
import { FrameProvider } from '@/components/elements/Frame'
import Sheet from '@/components/elements/Sheet'
import { MAX_ATTACHMENT_BYTES } from '@/api/attachments'
import { loginEntry, loginMeta } from './utils'
import { calls, mockCommand } from './ipc'

// The platform is baked in at build time, so a phone build is a module-level
// fact rather than a prop: this file is the attachments suite as that build
// sees it. The phone shell frames its dialogs with `Sheet`, so the tree is
// wrapped the way `Main` wraps it there.
vi.mock('@/lib/platform', () => ({
  isIOS: true,
  isAndroid: false,
  isMobile: true,
  isPC: false,
  chord: (key: string) => `⌘${key}`
}))

const BYTES = new Uint8Array([0xff, 0xd8, 0xff, 0xe0])

const renderShow = () =>
  render(
    <FrameProvider value={Sheet}>
      <Show entry={loginMeta()} />
    </FrameProvider>
  )

const openSheet = async () => {
  renderShow()
  await userEvent.click(await screen.findByTestId('attach-file-button'))
  return screen.findByTestId('attach-source-sheet')
}

beforeEach(() => {
  mockCommand('reveal_entry', () => loginEntry())
})

describe('attaching a file on a phone', () => {
  it('asks where the file comes from before opening any picker', async () => {
    const sheet = await openSheet()

    expect(sheet).toHaveAttribute('data-frame', 'bottom-sheet')
    expect(sheet).toHaveTextContent('Take Photo')
    expect(sheet).toHaveTextContent('Photo Library')
    expect(sheet).toHaveTextContent('Choose File…')
    expect(screen.getByTestId('attach-source-capture')).toHaveAttribute('capture', 'environment')
    expect(calls('pick_file')).toEqual([])
  })

  it('attaches a photo the camera took from its bytes, under its name', async () => {
    await openSheet()

    await userEvent.upload(
      screen.getByTestId('attach-source-capture'),
      new File([BYTES], 'IMG_0042.jpg', { type: 'image/jpeg' })
    )

    await waitFor(() =>
      expect(calls('attachment_add_bytes')).toEqual([
        { body: BYTES, headers: { 'x-entry-id': 'l1', 'x-file-name': 'IMG_0042.jpg' } }
      ])
    )
    expect(calls('attachment_add')).toEqual([])
    expect(screen.queryByTestId('attach-source-sheet')).not.toBeInTheDocument()
    expect(await screen.findByTestId('attachments-section')).toHaveTextContent('IMG_0042.jpg')
  })

  it('opens the photo library as an attachment pick under the image filter', async () => {
    mockCommand('pick_file', () => '/tmp/IMG_0042.heic')
    await openSheet()

    await userEvent.click(screen.getByTestId('attach-source-library'))

    expect(calls('pick_file')).toEqual([{ kind: 'photo', label: 'Images' }])
    await waitFor(() =>
      expect(calls('attachment_add')).toContainEqual({ entryId: 'l1', path: '/tmp/IMG_0042.heic' })
    )
    await waitFor(() =>
      expect(screen.queryByTestId('attach-source-sheet')).not.toBeInTheDocument()
    )
  })

  it('opens the file browser as the plain attachment pick', async () => {
    mockCommand('pick_file', () => '/tmp/contract.pdf')
    await openSheet()

    await userEvent.click(screen.getByTestId('attach-source-browse'))

    expect(calls('pick_file')).toEqual([{ kind: 'attachment', label: null }])
    await waitFor(() =>
      expect(calls('attachment_add')).toContainEqual({ entryId: 'l1', path: '/tmp/contract.pdf' })
    )
    await waitFor(() =>
      expect(screen.queryByTestId('attach-source-sheet')).not.toBeInTheDocument()
    )
  })

  it('leaves the question open when the picker is cancelled', async () => {
    mockCommand('pick_file', () => null)
    await openSheet()

    await userEvent.click(screen.getByTestId('attach-source-library'))

    await waitFor(() => expect(calls('pick_file')).toHaveLength(1))
    expect(screen.getByTestId('attach-source-sheet')).toBeInTheDocument()
    expect(calls('attachment_add')).toEqual([])
  })

  it('backs out of the sheet without opening a picker', async () => {
    await openSheet()

    await userEvent.click(await screen.findByTestId('modal-close'))

    await waitFor(() =>
      expect(screen.queryByTestId('attach-source-sheet')).not.toBeInTheDocument()
    )
    expect(calls('pick_file')).toEqual([])
    expect(calls('attachment_add')).toEqual([])
  })

  it('refuses a photo past the cap before reading it', async () => {
    await openSheet()
    const huge = new File([BYTES], 'IMG_0043.heic', { type: 'image/heic' })
    Object.defineProperty(huge, 'size', { value: MAX_ATTACHMENT_BYTES + 1 })
    const read = vi.spyOn(huge, 'arrayBuffer')

    await userEvent.upload(screen.getByTestId('attach-source-capture'), huge)

    expect(await screen.findByRole('alert')).toHaveTextContent('up to 10 MB')
    expect(read).not.toHaveBeenCalled()
    expect(calls('attachment_add_bytes')).toEqual([])
  })

  it('reports what the backend refused', async () => {
    mockCommand('attachment_add_bytes', () => {
      throw { kind: 'vaultFull', message: 'the vault is full' }
    })
    await openSheet()

    await userEvent.upload(
      screen.getByTestId('attach-source-capture'),
      new File([BYTES], 'IMG_0044.jpg', { type: 'image/jpeg' })
    )

    expect(await screen.findByRole('alert')).toHaveTextContent('attachments are at their limit')
  })
})
