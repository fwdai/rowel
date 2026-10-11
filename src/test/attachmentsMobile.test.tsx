import { describe, it, expect, beforeEach, vi } from 'vitest'
import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import Show from '@/components/Main/Body/Aside/Show'
import { FrameProvider } from '@/components/elements/Frame'
import Sheet from '@/components/elements/Sheet'
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

const renderShow = () =>
  render(
    <FrameProvider value={Sheet}>
      <Show entry={loginMeta()} />
    </FrameProvider>
  )

beforeEach(() => {
  mockCommand('reveal_entry', () => loginEntry())
})

describe('attaching a file on a phone', () => {
  it('asks where the file comes from before opening any picker', async () => {
    renderShow()

    await userEvent.click(await screen.findByTestId('attach-file-button'))

    const sheet = await screen.findByTestId('attach-source-sheet')
    expect(sheet).toHaveAttribute('data-frame', 'bottom-sheet')
    expect(sheet).toHaveTextContent('Photo Library')
    expect(sheet).toHaveTextContent('Choose File…')
    expect(calls('pick_file')).toEqual([])
  })

  it('opens the photo library as an attachment pick under the image filter', async () => {
    mockCommand('pick_file', () => '/tmp/IMG_0042.heic')
    renderShow()

    await userEvent.click(await screen.findByTestId('attach-file-button'))
    await userEvent.click(await screen.findByTestId('attach-photo-button'))

    expect(calls('pick_file')).toEqual([{ kind: 'photo', label: 'Images' }])
    await waitFor(() =>
      expect(calls('attachment_add')).toContainEqual({ entryId: 'l1', path: '/tmp/IMG_0042.heic' })
    )
    expect(screen.queryByTestId('attach-source-sheet')).not.toBeInTheDocument()
  })

  it('opens the file browser as the plain attachment pick', async () => {
    mockCommand('pick_file', () => '/tmp/contract.pdf')
    renderShow()

    await userEvent.click(await screen.findByTestId('attach-file-button'))
    await userEvent.click(await screen.findByTestId('attach-from-files-button'))

    expect(calls('pick_file')).toEqual([{ kind: 'attachment', label: null }])
    await waitFor(() =>
      expect(calls('attachment_add')).toContainEqual({ entryId: 'l1', path: '/tmp/contract.pdf' })
    )
    expect(screen.queryByTestId('attach-source-sheet')).not.toBeInTheDocument()
  })

  it('backs out of the sheet without opening a picker', async () => {
    renderShow()

    await userEvent.click(await screen.findByTestId('attach-file-button'))
    await userEvent.click(await screen.findByTestId('modal-close'))

    await waitFor(() =>
      expect(screen.queryByTestId('attach-source-sheet')).not.toBeInTheDocument()
    )
    expect(calls('pick_file')).toEqual([])
    expect(calls('attachment_add')).toEqual([])
  })
})
