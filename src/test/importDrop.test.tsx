import { describe, it, expect, beforeEach, vi } from 'vitest'
import { act, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import Settings from '@/components/Main/Sidebar/Settings'
import { calls, mockCommand } from './ipc'

// The webview's drag-drop stream, replaced by a hand that can drop a file —
// setup.ts's mock is the inert one (see envIngest.test.tsx).
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

beforeEach(() => vi.clearAllMocks())

const drop = async (path: string) => {
  await waitFor(() => expect(handlers.length).toBeGreaterThan(0))
  await act(async () => {
    for (const handler of [...handlers]) handler({ payload: { type: 'drop', paths: [path] } })
  })
}

const openImport = async () => {
  const { container } = render(<Settings />)
  await userEvent.click(container.querySelector('.settings-button')!)
  await userEvent.click(screen.getByTestId('settings-nav-import'))
}

// A drop on the import pane has no tile behind it. An export is sniffed by
// the backend; one of our own backups is taken as the backup card takes it,
// rather than sent to the sniffer to fail as "not an export".
describe('a file dropped on Settings › Import', () => {
  it('asks a .swftx for its password instead of sniffing it', async () => {
    mockCommand('import_swftx', () => ({ count: 2, entries: [] }))
    await openImport()

    await drop('/tmp/old-vault.swftx')

    expect(await screen.findByText('old-vault.swftx')).toBeInTheDocument()
    expect(calls('import_entries')).toEqual([])

    await userEvent.type(document.querySelector('input[name="import_password"]')!, 'source-pw')
    await userEvent.click(screen.getByTestId('import-run-backup'))

    expect(calls('import_swftx')).toEqual([{ path: '/tmp/old-vault.swftx', password: 'source-pw' }])
  })

  it('explains a .rowel rather than importing it', async () => {
    await openImport()

    await drop('/tmp/vault.rowel')

    expect(await screen.findByTestId('import-rowel-notice')).toBeInTheDocument()
    expect(calls('import_entries')).toEqual([])
  })

  it('still sniffs an export', async () => {
    await openImport()

    await drop('/tmp/export.csv')

    await waitFor(() =>
      expect(calls('import_entries')).toContainEqual({
        path: '/tmp/export.csv',
        format: 'auto',
        dryRun: true
      })
    )
  })
})
