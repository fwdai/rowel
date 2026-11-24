import { describe, it, expect, vi } from 'vitest'
import { act, render, renderHook, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import type { PickedFile } from '@/api/tools'
import { useImport } from '@/components/Main/Sidebar/Settings/Sections/Import/useImport'
import ImportDropZone from '@/components/Main/Sidebar/Settings/Sections/Import/DropZone'
import RestoreBackup from '@/components/Main/Sidebar/Settings/Sections/Workspaces/RestoreBackup'
import { useAttachments } from '@/components/Main/Body/Aside/Show/Attachments/useAttachments'
import { pickAndScan } from '@/components/Main/Scan/pick'
import { Start } from '@/components/Start'
import { useEnvIngest } from '@/kinds/env/useIngest'
import { withEntries, loginMeta } from './utils'
import { calls, mockCommand } from './ipc'

// On Android a picked file's address is a document URI, which ends in an id:
// nothing about the file can be read off it. Everything the webview tells
// about a pick is told from the name the pick came with, and the address is
// only ever handed back to be read.

const document = (id: string, name: string): PickedFile => ({
  path: `content://com.android.providers.downloads.documents/document/${id}`,
  name
})

const ROWEL = document('42', 'Rowel backup 2026-09-15.rowel')
const SWFTX = document('43', 'old-vault.swftx')

describe('a picked document in Settings › Import', () => {
  // The reported failure: a `.rowel` taken for a legacy vault by its address.
  it('takes a .rowel from the backup card as one of ours, by its name', async () => {
    mockCommand('pick_file', () => ROWEL)
    const { result } = renderHook(() => useImport())

    await act(() => result.current.chooseBackup())

    expect(result.current.picked).toEqual({ kind: 'rowel', ...ROWEL })
    expect(calls('import_swftx')).toEqual([])
  })

  it('takes a .swftx from the backup card as a legacy vault, read at its address', async () => {
    mockCommand('pick_file', () => SWFTX)
    mockCommand('import_swftx', () => ({ count: 1, entries: [] }))
    const { result } = renderHook(() => useImport())

    await act(() => result.current.chooseBackup())
    expect(result.current.picked).toEqual({ kind: 'swftx', ...SWFTX })
    await act(async () => result.current.runBackup('old-password'))

    expect(calls('import_swftx')).toEqual([{ path: SWFTX.path, password: 'old-password' }])
  })

  // The lead's own picker offers exports, but a backup picked there is still
  // told apart by its name rather than sent to the export parser.
  it('tells a backup picked from the lead by its name', async () => {
    mockCommand('pick_file', () => ROWEL)
    const { result } = renderHook(() => useImport())
    render(<ImportDropZone onDrop={result.current.dropped} />)

    await userEvent.click(screen.getByTestId('import-choose-file'))

    await waitFor(() => expect(result.current.picked).toEqual({ kind: 'rowel', ...ROWEL }))
    expect(calls('import_entries')).toEqual([])
  })

  it('previews an export read at its address', async () => {
    const csv = document('44', 'export.csv')
    mockCommand('pick_file', () => csv)
    const { result } = renderHook(() => useImport())

    await act(() => result.current.chooseFile('auto'))

    expect(calls('import_entries')).toContainEqual({ path: csv.path, format: 'auto', dryRun: true })
  })
})

describe('a picked document restored as a lockbox', () => {
  it('shows the name and restores from the address', async () => {
    withEntries([loginMeta()])
    mockCommand('pick_file', () => ROWEL)
    render(<RestoreBackup chosen={null} />)

    await userEvent.click(screen.getByTestId('workspace-backup-pick'))

    expect(await screen.findByTestId('workspace-backup-file')).toHaveTextContent(ROWEL.name)
    expect(screen.getByTestId('workspace-backup-file')).not.toHaveTextContent('content://')
    await userEvent.type(screen.getByTestId('workspace-backup-password'), 'backup-pass')
    await act(async () => {
      await userEvent.click(screen.getByTestId('workspace-backup-submit'))
    })

    expect(calls('workspace_restore_from_file')).toEqual([
      { name: '', password: 'backup-pass', path: ROWEL.path }
    ])
  })
})

describe('a picked document restored on a fresh install', () => {
  it('takes a legacy vault by its name and restores from the address', async () => {
    mockCommand('pick_file', () => SWFTX)
    render(<Start />)

    await userEvent.click(screen.getByTestId('start-restore-button'))
    await userEvent.click(screen.getByTestId('restore-dropzone'))

    expect(await screen.findByTestId('restore-found-file')).toHaveTextContent('old-vault.swftx')
    await userEvent.type(screen.getByTestId('restore-password-input'), 'old-password')
    await userEvent.click(screen.getByTestId('restore-confirm-button'))

    expect(calls('setup_restore_from_legacy')).toEqual([
      { path: SWFTX.path, password: 'old-password' }
    ])
    expect(calls('setup_restore_from_file')).toEqual([])
  })
})

describe('a picked document read as an env file', () => {
  it('reads the address and proposes a title from the name alone', async () => {
    const env = document('45', '.env.production')
    mockCommand('pick_file', () => env)
    mockCommand('read_env_file', () => ({ fileName: env.name, body: 'A=1\n' }))
    const consume = vi.fn()
    const { result } = renderHook(() => useEnvIngest(consume))

    await act(() => result.current.pick())

    expect(calls('read_env_file')).toEqual([{ path: env.path }])
    expect(consume).toHaveBeenCalledWith(expect.objectContaining({ title: 'production' }))
  })
})

describe('a picked document attached or scanned', () => {
  it('attaches from the address', async () => {
    withEntries([loginMeta()])
    const pdf = document('46', 'contract.pdf')
    mockCommand('pick_file', () => pdf)
    const { result } = renderHook(() => useAttachments('l1'))

    await act(async () => {
      expect(await result.current.pick()).toBe(true)
    })

    expect(calls('attachment_add')).toEqual([{ entryId: 'l1', path: pdf.path }])
  })

  it('scans from the address', async () => {
    withEntries([loginMeta()])
    const photo = document('47', 'IMG_0042.jpg')
    mockCommand('pick_file', () => photo)

    expect(await pickAndScan()).toBe(true)

    await waitFor(() => expect(calls('scan_image')).toEqual([{ path: photo.path }]))
  })
})
