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

const BYTES = new Uint8Array([0x25, 0x50, 0x44, 0x46])

const renderShow = () =>
  render(
    <FrameProvider value={Sheet}>
      <Show entry={loginMeta()} />
    </FrameProvider>
  )

const input = async () => screen.findByTestId<HTMLInputElement>('attach-file-input')

beforeEach(() => {
  mockCommand('reveal_entry', () => loginEntry())
})

describe('attaching a file on a phone', () => {
  it('opens the system sheet through a file input, not a sheet of its own', async () => {
    renderShow()
    const opened = vi.spyOn(await input(), 'click')

    await userEvent.click(await screen.findByTestId('attach-file-button'))

    expect(opened).toHaveBeenCalled()
    // Any file: the system's sheet is what offers the photo library, the
    // camera and Files, so nothing here narrows it.
    expect(await input()).not.toHaveAttribute('accept')
    expect(await input()).not.toHaveAttribute('capture')
    expect(calls('pick_file')).toEqual([])
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
  })

  it('attaches what the sheet hands back from its bytes, under its name', async () => {
    renderShow()

    await userEvent.upload(await input(), new File([BYTES], 'Résumé 2026.pdf'))

    await waitFor(() =>
      expect(calls('attachment_add_bytes')).toEqual([
        {
          body: BYTES,
          headers: { 'x-entry-id': 'l1', 'x-file-name': encodeURIComponent('Résumé 2026.pdf') }
        }
      ])
    )
    expect(calls('attachment_add')).toEqual([])
    // Listed like any other file, under the name it was given.
    expect(await screen.findByTestId('attachments-section')).toHaveTextContent('Résumé 2026.pdf')
  })

  it('refuses a file past the cap before reading it', async () => {
    renderShow()
    const huge = new File([BYTES], 'movie.mov')
    Object.defineProperty(huge, 'size', { value: MAX_ATTACHMENT_BYTES + 1 })
    const read = vi.spyOn(huge, 'arrayBuffer')

    await userEvent.upload(await input(), huge)

    expect(await screen.findByRole('alert')).toHaveTextContent('up to 10 MB')
    expect(read).not.toHaveBeenCalled()
    expect(calls('attachment_add_bytes')).toEqual([])
  })

  it('reports what the backend refused', async () => {
    mockCommand('attachment_add_bytes', () => {
      throw { kind: 'vaultFull', message: 'the vault is full' }
    })
    renderShow()

    await userEvent.upload(await input(), new File([BYTES], 'scan.pdf'))

    expect(await screen.findByRole('alert')).toHaveTextContent('attachments are at their limit')
  })
})
