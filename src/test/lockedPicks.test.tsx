import { describe, it, expect, vi } from 'vitest'
import { act, render, renderHook, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import type { PickedFile } from '@/api/tools'
import { clearSession, useUi, useVault } from '@/store'
import { pickAndScan } from '@/components/Main/Scan/pick'
import { runScan } from '@/components/Main/Scan/run'
import { useAttachments } from '@/components/Main/Body/Aside/Show/Attachments/useAttachments'
import { useImport } from '@/components/Main/Sidebar/Settings/Sections/Import/useImport'
import ImportDropZone from '@/components/Main/Sidebar/Settings/Sections/Import/DropZone'
import RestoreBackup from '@/components/Main/Sidebar/Settings/Sections/Workspaces/RestoreBackup'
import { useEnvIngest } from '@/kinds/env/useIngest'
import { pickedPath } from '@/lib/picked'
import { withEntries, loginMeta, deferred } from './utils'
import { calls, mockCommand } from './ipc'

// Every flow here awaits something with nothing of the session in hand — a
// picker, a file being read — and then publishes, a result or an error. A lock
// while it waits ends the session that asked (`clearSession`); whatever is
// open when the wait ends never asked, and hears nothing of it.

const CARD = { number: '4242424242424242', month: '04', year: '27', name: 'ADA LOVELACE' }

const lock = () => act(() => clearSession())

// A picker held open until the test answers it.
const heldPicker = () => {
  const answer = deferred<PickedFile | null>()
  mockCommand('pick_file', () => answer.promise)
  return answer
}

describe('a scan picked while the vault locks', () => {
  it('neither scans nor reports a picker answered after the lock', async () => {
    withEntries([loginMeta()])
    const picker = heldPicker()
    mockCommand('scan_image', () => ({ kind: 'card', fields: CARD }))

    const handed = pickAndScan()
    lock()
    picker.resolve(pickedPath('/Users/me/card.png'))

    expect(await handed).toBe(false)
    expect(calls('scan_image')).toHaveLength(0)
    expect(useUi.getState()).toMatchObject({ scanBusy: false, scanError: null })
  })

  it('reports nothing of a picker that failed after the lock', async () => {
    withEntries([loginMeta()])
    const picker = heldPicker()

    const handed = pickAndScan()
    lock()
    picker.reject({ kind: 'unsupported', message: 'the picked file has no local path' })

    expect(await handed).toBe(false)
    expect(useUi.getState().scanError).toBeNull()
  })

  it('lands nothing of a scan the lock overtook', async () => {
    withEntries([loginMeta()])
    const scan = deferred<unknown>()
    mockCommand('scan_image', () => scan.promise)

    const running = runScan('/Users/me/card.png')
    lock()
    scan.resolve({ kind: 'card', fields: CARD })
    await running

    expect(useVault.getState().creating).toBeNull()
    expect(useUi.getState()).toMatchObject({ scanBusy: false, scanError: null })
  })
})

describe('an attachment picked while the vault locks', () => {
  it('attaches nothing a picker answered after the lock', async () => {
    withEntries([loginMeta()])
    const picker = heldPicker()
    const { result } = renderHook(() => useAttachments('l1'))

    let picking: Promise<boolean> = Promise.resolve(false)
    act(() => {
      picking = result.current.pick()
    })
    lock()
    await act(async () => {
      picker.resolve(pickedPath('/Users/me/contract.pdf'))
      expect(await picking).toBe(false)
    })

    expect(calls('attachment_add')).toEqual([])
    expect(result.current.error).toBeNull()
  })

  it('shows nothing of a picker that failed after the lock', async () => {
    withEntries([loginMeta()])
    const picker = heldPicker()
    const { result } = renderHook(() => useAttachments('l1'))

    let picking: Promise<boolean> = Promise.resolve(false)
    act(() => {
      picking = result.current.pick()
    })
    lock()
    await act(async () => {
      picker.reject({ kind: 'unsupported', message: 'the picked file has no local path' })
      await picking
    })

    expect(result.current.error).toBeNull()
  })

  it('sends nothing of a photo whose read the lock overtook', async () => {
    withEntries([loginMeta()])
    const read = deferred<ArrayBuffer>()
    const photo = new File([new Uint8Array([0xff, 0xd8])], 'IMG_0042.jpg', { type: 'image/jpeg' })
    Object.defineProperty(photo, 'arrayBuffer', { value: () => read.promise })
    const { result } = renderHook(() => useAttachments('l1'))

    let adding: Promise<void> = Promise.resolve()
    act(() => {
      adding = result.current.addFile(photo)
    })
    lock()
    await act(async () => {
      read.resolve(new Uint8Array([0xff, 0xd8]).buffer)
      await adding
    })

    expect(calls('attachment_add_bytes')).toEqual([])
    expect(result.current.error).toBeNull()
  })
})

describe('an import picked while the vault locks', () => {
  it('starts nothing from a tile whose picker answered after the lock', async () => {
    withEntries([loginMeta()])
    const picker = heldPicker()
    const { result } = renderHook(() => useImport())

    let choosing: Promise<void> = Promise.resolve()
    act(() => {
      choosing = result.current.chooseFile('bitwarden')
    })
    lock()
    await act(async () => {
      picker.resolve(pickedPath('/Users/me/export.json'))
      await choosing
    })

    expect(result.current.picked).toBeNull()
    expect(calls('import_entries')).toEqual([])
  })

  it('starts nothing from the backup card whose picker answered after the lock', async () => {
    withEntries([loginMeta()])
    const picker = heldPicker()
    const { result } = renderHook(() => useImport())

    let choosing: Promise<void> = Promise.resolve()
    act(() => {
      choosing = result.current.chooseBackup()
    })
    lock()
    await act(async () => {
      picker.resolve(pickedPath('/Users/me/old.swftx'))
      await choosing
    })

    expect(result.current.picked).toBeNull()
  })

  it('hands nothing on from the lead whose picker answered after the lock', async () => {
    withEntries([loginMeta()])
    const picker = heldPicker()
    const onDrop = vi.fn()
    render(<ImportDropZone onDrop={onDrop} />)

    await userEvent.click(screen.getByTestId('import-choose-file'))
    lock()
    await act(async () => picker.resolve(pickedPath('/Users/me/export.csv')))

    expect(onDrop).not.toHaveBeenCalled()
  })
})

describe('a lockbox backup picked while the vault locks', () => {
  it('shows no file a picker answered after the lock', async () => {
    withEntries([loginMeta()])
    const picker = heldPicker()
    render(<RestoreBackup chosen={null} />)

    await userEvent.click(screen.getByTestId('workspace-backup-pick'))
    lock()
    await act(async () => picker.resolve(pickedPath('/Users/me/Rowel backup.rowel')))

    expect(screen.queryByTestId('workspace-backup-file')).not.toBeInTheDocument()
  })
})

describe('an env file picked while the vault locks', () => {
  it('reads nothing a picker answered after the lock', async () => {
    withEntries([loginMeta()])
    const picker = heldPicker()
    const consume = vi.fn()
    const { result } = renderHook(() => useEnvIngest(consume))

    let picking: Promise<void> = Promise.resolve()
    act(() => {
      picking = result.current.pick()
    })
    lock()
    await act(async () => {
      picker.resolve(pickedPath('/Users/me/code/api/.env'))
      await picking
    })

    expect(calls('read_env_file')).toEqual([])
    expect(consume).not.toHaveBeenCalled()
  })

  it('shows nothing of a picker that failed after the lock', async () => {
    withEntries([loginMeta()])
    const picker = heldPicker()
    const { result } = renderHook(() => useEnvIngest(vi.fn()))

    let picking: Promise<void> = Promise.resolve()
    act(() => {
      picking = result.current.pick()
    })
    lock()
    await act(async () => {
      picker.reject({ kind: 'fileTooLarge', message: 'the file is too large' })
      await picking
    })

    expect(result.current.error).toBeNull()
  })
})
