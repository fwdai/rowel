import { describe, it, expect, beforeEach, vi } from 'vitest'
import { act, render, screen, waitFor } from '@testing-library/react'
import Main from '@/components/Main'
import { useUi } from '@/store'
import { workspaceAdded } from '@/store/events'
import { calls } from './ipc'
import { resetStores, withEntries } from './utils'

// A vault from the account arriving as a workspace on its own: the password
// that opened this vault opened it too (`commands::autojoin`). Nothing on
// screen switches, so the user is told in passing and the workspace list is
// re-probed.
describe('a vault added from the account', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    resetStores()
  })

  it('says so for a moment and re-probes the workspace list', async () => {
    withEntries([])
    render(<Main />)
    const probes = calls('app_status').length

    await act(async () => workspaceAdded('Vault 9f3c1a'))

    expect(screen.getByTestId('notice-toast')).toHaveTextContent(
      'Added “Vault 9f3c1a” from your Google Drive account'
    )
    await waitFor(() => expect(calls('app_status').length).toBeGreaterThan(probes))
    // The notice is not state the shell keeps: it clears on its own.
    expect(useUi.getState().notice).not.toBeNull()
  })

  it('lets a later notice take over rather than queueing behind the first', async () => {
    withEntries([])
    render(<Main />)

    await act(async () => workspaceAdded('Vault aaaaaa'))
    await act(async () => workspaceAdded('Vault bbbbbb'))

    expect(screen.getAllByTestId('notice-toast')).toHaveLength(1)
    expect(screen.getByTestId('notice-toast')).toHaveTextContent('Vault bbbbbb')
  })
})
