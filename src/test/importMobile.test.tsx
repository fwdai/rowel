import { describe, it, expect, vi } from 'vitest'
import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { open as openDialog } from '@tauri-apps/plugin-dialog'
import DropZone from '@/components/Main/Sidebar/Settings/Sections/Import/DropZone'

// The phone build: nothing drags a file onto it, so the import pane's lead is
// the picker itself rather than a drop target with a picker button inside.
vi.mock('@/lib/platform', () => ({
  isIOS: true,
  isAndroid: false,
  isMobile: true,
  isPC: false,
  chord: (key: string) => `⌘${key}`
}))

describe('the import lead, on a phone', () => {
  it('is one card that picks a file, and never says to drop one', async () => {
    const onDrop = vi.fn()
    vi.mocked(openDialog).mockResolvedValue('/tmp/export.csv')
    render(<DropZone onDrop={onDrop} />)

    expect(screen.queryByText('Drop an export file')).not.toBeInTheDocument()
    expect(screen.queryByTestId('import-choose-file')).not.toBeInTheDocument()
    const card = screen.getByTestId('import-dropzone')
    expect(card).toHaveTextContent('Choose an export file')

    await userEvent.click(card)
    await waitFor(() => expect(onDrop).toHaveBeenCalledWith('/tmp/export.csv'))
  })
})
