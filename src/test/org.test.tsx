import { describe, it, expect, beforeEach, vi } from 'vitest'
import { act, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import Settings from '@/components/Main/Sidebar/Settings'
import BrowserConsent from '@/components/Main/BrowserConsent'
import ProviderButtons from '@/components/elements/ProviderButtons'
import { EVENTS } from '@/api/events'
import type { AgentGrantAsk } from '@/api/browser'
import type { OrgConnection, OrgDevice, OrgLockbox } from '@/api/org'
import { providerName } from '@/api/provider'
import { subscribeToEvents } from '@/store/events'
import { calls, clearCalls, mockCommand, mockCommandOnce } from './ipc'
import { emitEvent } from './events'
import { resetStores, seedApp } from './utils'

beforeEach(() => {
  vi.clearAllMocks()
  resetStores()
})

const SERVER = 'https://rowel.acme.example'
const THIS_KEY = '1a92 f238 52dc 908d'
const PHONE_KEY = '7c01 aa10 0b2e 44f9'

const CONNECTION: OrgConnection = {
  url: SERVER,
  tenantName: 'Acme',
  memberEmail: 'ada@acme.example',
  memberName: 'Ada',
  deviceId: 'd-this',
  deviceState: 'approved'
}

const DEVICES: OrgDevice[] = [
  { id: 'd-this', name: 'This MacBook', state: 'pending', fingerprint: THIS_KEY, thisDevice: true, createdAtMs: 1 },
  { id: 'd-phone', name: "Ada's iPhone", state: 'pending', fingerprint: PHONE_KEY, thisDevice: false, createdAtMs: 2 }
]

const LOCKBOXES: OrgLockbox[] = [
  { id: 'lb-eng', name: 'Engineering', workspaceId: null },
  { id: 'lb-ops', name: 'Ops', workspaceId: 'w-ops' }
]

// Settings › Lockboxes, with the event stream wired, since a connect's
// outcome only ever arrives as an event.
const openLockboxes = async () => {
  subscribeToEvents()
  const { container } = render(<Settings />)
  await userEvent.click(container.querySelector('.settings-button')!)
  await userEvent.click(screen.getByTestId('settings-nav-workspaces'))
}

const connect = async () => {
  await userEvent.type(screen.getByTestId('org-connect-url'), SERVER)
  await userEvent.click(screen.getByTestId('org-connect'))
}

describe('Settings › Lockboxes › Rowel Server', () => {
  it("shows this device's enrollment key before any server is connected", async () => {
    await openLockboxes()
    expect(await screen.findByTestId('org-fingerprint')).toHaveTextContent(THIS_KEY)
    expect(screen.queryByTestId(/^org-connection-/)).not.toBeInTheDocument()
  })

  // The command opens the browser and resolves; the sign-in finishes out
  // there. The row waits with a Cancel, and a `done` is a new connection, so
  // the list is read again and the card appears.
  it('waits on the browser, then lists the connection once the sign-in is done', async () => {
    await openLockboxes()
    await connect()

    expect(calls('org_connect')).toEqual([{ url: SERVER }])
    expect(screen.getByTestId('org-connect-waiting')).toHaveTextContent(
      'Finish signing in in your browser…'
    )
    expect(screen.getByTestId('org-connect-cancel')).toBeInTheDocument()
    expect(screen.queryByTestId('org-connect')).not.toBeInTheDocument()

    mockCommand('org_connections', () => [CONNECTION])
    act(() => emitEvent(EVENTS.orgConnect, { url: SERVER, attempt: 1, status: 'done' }))

    const card = await screen.findByTestId('org-connection-rowel.acme.example')
    expect(card).toHaveTextContent('Acme')
    expect(card).toHaveTextContent('ada@acme.example')
    expect(screen.getByTestId('org-device-line')).toHaveTextContent('This device is enrolled')
    expect(screen.queryByTestId('org-connect-waiting')).not.toBeInTheDocument()
  })

  it('shows the failure the event carries, and offers Connect again', async () => {
    await openLockboxes()
    await connect()

    act(() =>
      emitEvent(EVENTS.orgConnect, {
        url: SERVER,
        attempt: 1,
        status: 'error',
        error: 'the server refused the sign-in'
      })
    )

    expect(screen.getByTestId('org-connect-error')).toHaveTextContent(
      'the server refused the sign-in'
    )
    expect(screen.queryByTestId('org-connect-waiting')).not.toBeInTheDocument()
    expect(screen.getByTestId('org-connect')).toBeInTheDocument()
  })

  // A cancel ends the attempt here and now. Its last word — the failure Rust
  // may already have had on its way — is nobody's, so it is not shown.
  it('cancels a sign-in still out with the browser, and ignores what it says afterwards', async () => {
    await openLockboxes()
    await connect()
    await userEvent.click(screen.getByTestId('org-connect-cancel'))

    expect(calls('org_cancel_connect')).toHaveLength(1)
    expect(screen.queryByTestId('org-connect-waiting')).not.toBeInTheDocument()

    act(() =>
      emitEvent(EVENTS.orgConnect, { url: SERVER, attempt: 1, status: 'error', error: 'cancelled' })
    )

    expect(screen.queryByTestId('org-connect-error')).not.toBeInTheDocument()
    expect(screen.queryByTestId('org-connect-waiting')).not.toBeInTheDocument()
    expect(screen.getByTestId('org-connect')).toBeInTheDocument()
  })

  // A newer press replaces the attempt: the older one's error does not
  // interrupt the wait, and only the newer one's `done` lists the connection.
  it('reports only the live attempt once a newer connect began', async () => {
    mockCommandOnce('org_connect', () => 1)
    mockCommandOnce('org_connect', () => 2)
    await openLockboxes()
    await connect()
    await userEvent.click(screen.getByTestId('org-connect-cancel'))
    await userEvent.click(screen.getByTestId('org-connect'))

    expect(calls('org_connect')).toEqual([{ url: SERVER }, { url: SERVER }])
    expect(screen.getByTestId('org-connect-waiting')).toBeInTheDocument()

    act(() =>
      emitEvent(EVENTS.orgConnect, { url: SERVER, attempt: 1, status: 'error', error: 'too late' })
    )
    expect(screen.queryByTestId('org-connect-error')).not.toBeInTheDocument()
    expect(screen.getByTestId('org-connect-waiting')).toBeInTheDocument()

    mockCommand('org_connections', () => [CONNECTION])
    act(() => emitEvent(EVENTS.orgConnect, { url: SERVER, attempt: 2, status: 'done' }))

    expect(await screen.findByTestId('org-connection-rowel.acme.example')).toHaveTextContent('Acme')
    expect(screen.queryByTestId('org-connect-waiting')).not.toBeInTheDocument()
  })

  // The sign-in comes back through the browser, which the phone cannot listen
  // for yet: the probe says so, and the whole group stays off the screen.
  it('draws nothing of it when the build cannot connect to a server', async () => {
    seedApp({ orgConnect: false })
    await openLockboxes()

    expect(screen.getByText('Cloud sync')).toBeInTheDocument()
    expect(screen.queryByTestId('org-fingerprint-row')).not.toBeInTheDocument()
    expect(screen.queryByTestId('org-connect-row')).not.toBeInTheDocument()
    expect(calls('org_connections')).toEqual([])
    expect(calls('org_enrollment_fingerprint')).toEqual([])
  })

  // A device still waiting says who it waits on and shows the key they will
  // compare; the other pending device gets an Approve, this one does not, and
  // the confirm quotes that device's key before anything is sent.
  it('shows the key while approval is pending, and approves another device by it', async () => {
    mockCommand('org_connections', () => [{ ...CONNECTION, deviceState: 'pending' }])
    mockCommand('org_devices', () => DEVICES)
    await openLockboxes()

    const line = await screen.findByTestId('org-device-line')
    expect(line).toHaveTextContent('Waiting for approval from another of your devices or an admin')
    expect(line).toHaveTextContent(THIS_KEY)

    expect(await screen.findByTestId('org-device-d-phone')).toHaveTextContent(PHONE_KEY)
    expect(screen.queryByTestId('org-approve-d-this')).not.toBeInTheDocument()

    await userEvent.click(screen.getByTestId('org-approve-d-phone'))
    expect(screen.getByTestId('org-approve-sheet')).toHaveTextContent(PHONE_KEY)
    expect(calls('org_approve_device')).toEqual([])

    await userEvent.click(screen.getByTestId('org-approve-confirm'))
    expect(calls('org_approve_device')).toEqual([{ url: SERVER, deviceId: 'd-phone' }])
  })

  // Adding one is a new workspace, which lives on the launch probe: the app is
  // re-probed. One already here says so and offers its key fetched again.
  it('adds an org lockbox and re-probes the app, and refreshes the key of one already here', async () => {
    mockCommand('org_connections', () => [CONNECTION])
    mockCommand('org_lockboxes', () => LOCKBOXES)
    await openLockboxes()

    expect(await screen.findByTestId('org-lockbox-lb-ops')).toHaveTextContent('On this device')
    expect(screen.getByText(/so the server can open it/)).toBeInTheDocument()

    clearCalls('app_status')
    await userEvent.click(screen.getByTestId('org-add-lb-eng'))
    expect(calls('org_add_lockbox')).toEqual([{ url: SERVER, lockboxId: 'lb-eng' }])
    await waitFor(() => expect(calls('app_status')).toHaveLength(1))

    await userEvent.click(screen.getByTestId('org-refresh-key-lb-ops'))
    expect(calls('org_refresh_lockbox_key')).toEqual([{ workspaceId: 'w-ops' }])
  })

  it('disconnects only after a confirm that says the lockboxes here stay', async () => {
    mockCommand('org_connections', () => [CONNECTION])
    await openLockboxes()

    await userEvent.click(await screen.findByTestId('org-disconnect'))
    expect(screen.getByTestId('org-disconnect-sheet')).toHaveTextContent(
      'The org lockboxes already on this device stay, but stop syncing.'
    )
    expect(calls('org_disconnect')).toEqual([])

    mockCommand('org_connections', () => [])
    await userEvent.click(screen.getByTestId('org-disconnect-confirm'))
    expect(calls('org_disconnect')).toEqual([{ url: SERVER }])
    await waitFor(() =>
      expect(screen.queryByTestId('org-connection-rowel.acme.example')).not.toBeInTheDocument()
    )
  })
})

const ASK: AgentGrantAsk = {
  id: 'ask-s',
  clientName: 'Claude Code',
  clientKind: 'agent',
  targets: [{ kind: 'origin', origin: 'https://github.com' }],
  mode: 'fill',
  endsSession: true,
  stepUp: false,
  candidates: [{ id: 'gh1', title: 'GitHub', username: 'octocat' }],
  defaultStandingDays: 7,
  server: 'Acme'
}

describe('the grant dialog for an ask a Rowel Server routed here', () => {
  // The agent is named as it is for a local ask; the server is named under
  // it. Nothing is filled on this device, so no sign-out is warned of.
  it('says which server asked, and never warns of a sign-out', async () => {
    subscribeToEvents()
    render(<BrowserConsent />)
    act(() => emitEvent(EVENTS.agentGrant, ASK))

    await screen.findByTestId('agent-grant-dialog')
    expect(screen.getByTestId('agent-grant-title')).toHaveTextContent(
      'Claude Code wants to log into https://github.com'
    )
    expect(screen.getByTestId('agent-grant-server')).toHaveTextContent('Asked through Acme')
    expect(screen.queryByTestId('agent-grant-ends-session')).not.toBeInTheDocument()
  })
})

describe('the Rowel Server provider', () => {
  it('has a product name but is never offered by the account pickers', () => {
    expect(providerName('rowelServer')).toBe('Rowel Server')

    seedApp({ providers: ['googleDrive', 'dropbox', 'iCloud'] })
    render(<ProviderButtons testid="pick" onPick={() => {}} />)
    expect(screen.getAllByRole('button').map(button => button.getAttribute('data-testid'))).toEqual(
      ['pick-drive-connect', 'pick-dropbox-connect', 'pick-icloud-connect']
    )
  })
})
