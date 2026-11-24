import { createRef } from 'react'
import { describe, it, expect, vi } from 'vitest'
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import FileInput from '@/components/elements/FileInput'
import { FrameProvider } from '@/components/elements/Frame'
import Sheet from '@/components/elements/Sheet'
import Show from '@/components/Main/Body/Aside/Show'
import { MAX_ATTACHMENT_BYTES } from '@/api/attachments'
import { clearSession } from '@/store'
import { deferred, loginEntry, loginMeta, withEntries } from './utils'
import { calls, mockCommand } from './ipc'

// The Android build, where the camera is the camera app writing into a file
// wry made for it and never removes: the capture input says when it is done
// with that file, and only then (`capture_settled`, swept by `capture.rs`).
vi.mock('@/lib/platform', () => ({
  isIOS: false,
  isAndroid: true,
  isMobile: true,
  isPC: false,
  chord: (key: string) => `⌘${key}`
}))

const BYTES = new Uint8Array([0xff, 0xd8, 0xff, 0xe0])
const photo = (name = 'JPEG_20261124_104700_42.jpg') =>
  new File([BYTES], name, { type: 'image/jpeg' })

const input = (onFile: (file: File) => Promise<void> | void, capture = true) => {
  render(
    <FileInput
      testid="capture"
      accept="image/*"
      capture={capture}
      onFile={onFile}
      ref={createRef()}
    />
  )
  return screen.getByTestId('capture')
}

describe('a camera capture on Android', () => {
  // A capture still being read is the camera's file in use: the sweep waits
  // for whatever was handed the photo to be done with it.
  it('is let go once the photo it was handed is done with, not before', async () => {
    const reading = deferred<void>()
    const onFile = vi.fn(() => reading.promise)

    await userEvent.upload(input(onFile), photo())

    expect(onFile).toHaveBeenCalledTimes(1)
    expect(calls('capture_settled')).toEqual([])
    await act(async () => reading.resolve())
    await waitFor(() => expect(calls('capture_settled')).toHaveLength(1))
  })

  // The camera backed out of: wry's empty file is given up on.
  it('is let go when the camera is cancelled', () => {
    const onFile = vi.fn()

    fireEvent(input(onFile), new Event('cancel'))

    expect(onFile).not.toHaveBeenCalled()
    expect(calls('capture_settled')).toHaveLength(1)
  })

  // A file input that is no camera has no file of wry's behind it.
  it('asks nothing of a plain file input', async () => {
    const plain = input(() => {}, false)
    await userEvent.upload(plain, photo('contract.pdf'))
    fireEvent(plain, new Event('cancel'))

    expect(calls('capture_settled')).toEqual([])
  })
})

describe('an attachment photo on Android', () => {
  const openSheet = async () => {
    withEntries([loginMeta()])
    mockCommand('reveal_entry', () => loginEntry())
    render(
      <FrameProvider value={Sheet}>
        <Show entry={loginMeta()} />
      </FrameProvider>
    )
    await userEvent.click(await screen.findByTestId('attach-file-button'))
    await screen.findByTestId('attach-source-sheet')
  }

  // Refused by size in the webview, so its bytes never reach Rust: the camera's
  // file goes all the same.
  it('lets the capture go when the photo is refused for its size', async () => {
    await openSheet()
    const huge = photo()
    Object.defineProperty(huge, 'size', { value: MAX_ATTACHMENT_BYTES + 1 })

    await userEvent.upload(screen.getByTestId('attach-source-capture'), huge)

    expect(await screen.findByRole('alert')).toHaveTextContent('up to 10 MB')
    expect(calls('attachment_add_bytes')).toEqual([])
    await waitFor(() => expect(calls('capture_settled')).toHaveLength(1))
  })

  // Dropped across a lock, its bytes never reach Rust either.
  it('lets the capture go when a lock drops the photo', async () => {
    await openSheet()
    const read = deferred<ArrayBuffer>()
    const slow = photo()
    Object.defineProperty(slow, 'arrayBuffer', { value: () => read.promise })

    await userEvent.upload(screen.getByTestId('attach-source-capture'), slow)
    act(() => clearSession())
    await act(async () => read.resolve(BYTES.buffer))

    expect(calls('attachment_add_bytes')).toEqual([])
    await waitFor(() => expect(calls('capture_settled')).toHaveLength(1))
  })

  it('lets the capture go once the photo is attached', async () => {
    await openSheet()

    await userEvent.upload(screen.getByTestId('attach-source-capture'), photo())

    await waitFor(() => expect(calls('attachment_add_bytes')).toHaveLength(1))
    await waitFor(() => expect(calls('capture_settled')).toHaveLength(1))
  })
})
