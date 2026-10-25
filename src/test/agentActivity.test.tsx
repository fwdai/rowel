import { describe, it, expect } from 'vitest'
import { act, render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { EVENTS } from '@/api/events'
import type {
  AgentActivity,
  AgentEvent,
  AgentRule,
  AuditVerification,
  BrowserClient,
  BrowserStatus,
  WriterChain
} from '@/api/browser'
import Settings from '@/components/Main/Sidebar/Settings'
import { openSettings } from '@/store'
import { subscribeToEvents } from '@/store/events'
import { calls, mockCommand } from './ipc'
import { emitEvent } from './events'

const KEY = 'AAAAbbbbCCCCddddEEEEffffGGGGhhhhIIIIjjjjKKK='

const status = (agents: boolean): BrowserStatus => ({
  enabled: true,
  keepassxcCompat: true,
  agents: { enabled: agents, defaultStandingDays: 7 },
  browsers: [],
  clients: []
})

const activity = (overrides: Partial<AgentActivity> = {}): AgentActivity => ({
  events: [],
  staleRules: [],
  idleClients: [],
  ...overrides
})

const event = (overrides: Partial<AgentEvent> = {}): AgentEvent => ({
  id: 'e1',
  tsMs: Date.UTC(2030, 0, 8, 9, 14),
  writerId: 'this-writer',
  thisDevice: true,
  event: 'used',
  actorKind: 'agent',
  actorName: 'Claude Code',
  runId: 'run-1',
  grantId: 'g1',
  itemId: 'gh1',
  itemTitle: 'GitHub',
  target: 'https://github.com/login',
  mode: 'fill',
  approval: 'ask',
  reason: null,
  ...overrides
})

const rule = (overrides: Partial<AgentRule> = {}): AgentRule => ({
  id: 'r1',
  clientKey: KEY,
  clientName: 'Work Chrome',
  itemId: 'gh1',
  itemTitle: 'GitHub',
  origin: 'https://github.com/login',
  target: 'https://github.com/login',
  mode: 'fill',
  expiresAtMs: null,
  createdAtMs: Date.UTC(2029, 10, 2),
  ...overrides
})

const client = (overrides: Partial<BrowserClient> = {}): BrowserClient => ({
  name: 'Old Codex',
  key: KEY,
  kind: 'agent',
  platform: 'codex',
  createdAtMs: Date.UTC(2029, 5, 1),
  lastSeenAtMs: Date.UTC(2029, 6, 1),
  ...overrides
})

const writer = (overrides: Partial<WriterChain> = {}): WriterChain => ({
  writerId: 'this-writer',
  thisDevice: true,
  rows: 10,
  firstSeq: 1,
  lastSeq: 10,
  break: null,
  ...overrides
})

const verification = (writers: WriterChain[]): AuditVerification => ({
  ok: writers.every(chain => chain.break === null),
  writers
})

// The section is up once the hero has drawn; the panel reads on its own
// promise, so what follows waits for the read to have happened.
const openSection = async () => {
  openSettings('audit')
  render(<Settings />)
  await screen.findByTestId('settings-audit-run')
  await waitFor(() => expect(calls('audit_agent_activity')).not.toHaveLength(0))
}

describe('Settings › Vault audit › Agent activity', () => {
  it('is not there for someone with agents off and nothing in the log', async () => {
    mockCommand('browser_status', () => status(false))
    mockCommand('audit_agent_activity', () => activity())
    await openSection()
    expect(screen.queryByTestId('settings-agent-activity')).not.toBeInTheDocument()
  })

  it('is not there while the vault is locked, when the read is refused', async () => {
    mockCommand('browser_status', () => status(true))
    mockCommand('audit_agent_activity', () =>
      Promise.reject({ kind: 'locked', message: 'the vault is locked' })
    )
    await openSection()
    expect(screen.queryByTestId('settings-agent-activity')).not.toBeInTheDocument()
  })

  it('is there, empty, once agents are on', async () => {
    mockCommand('browser_status', () => status(true))
    mockCommand('audit_agent_activity', () => activity())
    await openSection()

    expect(await screen.findByTestId('settings-agent-activity-empty')).toHaveTextContent(
      'No agent activity in the last 7 days'
    )
    expect(screen.getByTestId('settings-audit-verify')).toBeInTheDocument()
    expect(screen.queryByTestId('settings-agent-stale-rule')).not.toBeInTheDocument()
    expect(screen.queryByTestId('settings-agent-idle-client')).not.toBeInTheDocument()
  })

  // What agents did stays readable after the switch goes off: the log is the
  // point of the panel, not the switch.
  it('is there with agents off while the log has something to show', async () => {
    mockCommand('browser_status', () => status(false))
    mockCommand('audit_agent_activity', () => activity({ events: [event()] }))
    await openSection()
    expect(await screen.findByTestId('settings-agent-event')).toHaveTextContent('GitHub')
  })

  it('shows each event as the item or the place, who, the mode, and how it went', async () => {
    mockCommand('browser_status', () => status(true))
    mockCommand('audit_agent_activity', () =>
      activity({
        events: [
          event(),
          event({
            id: 'e2',
            event: 'denied',
            reason: 'no_rule',
            itemId: null,
            itemTitle: null,
            target: 'https://example.com/signin'
          }),
          event({
            id: 'e3',
            event: 'exposed',
            reason: 'exposure',
            actorKind: 'sidecar',
            actorName: 'rowel run',
            itemTitle: 'CI',
            target: 'node deploy.js --prod',
            mode: 'env'
          })
        ]
      })
    )
    await openSection()

    const rows = await screen.findAllByTestId('settings-agent-event')
    expect(rows).toHaveLength(3)
    expect(rows[0]).toHaveTextContent('GitHub')
    expect(rows[0]).toHaveTextContent('Claude Code (Agent) · https://github.com/login')
    expect(within(rows[0]).getByTestId('settings-agent-event-outcome')).toHaveTextContent('Used')

    // No item was ever picked, so the place stands in for it.
    expect(within(rows[1]).getByText('https://example.com/signin')).toBeInTheDocument()
    expect(within(rows[1]).getByTestId('settings-agent-event-outcome')).toHaveTextContent(
      'Denied: no rule allowed it'
    )

    expect(rows[2]).toHaveTextContent('CI')
    expect(rows[2]).toHaveTextContent(
      "rowel run (Command line tool) · node deploy.js --prod · in a command's environment"
    )
    expect(within(rows[2]).getByTestId('settings-agent-event-outcome')).toHaveTextContent('Exposed')
  })

  it('shows the first 20 events and the rest on request', async () => {
    const events = Array.from({ length: 23 }, (_, i) => event({ id: `e${i}` }))
    mockCommand('browser_status', () => status(true))
    mockCommand('audit_agent_activity', () => activity({ events }))
    await openSection()

    expect(await screen.findAllByTestId('settings-agent-event')).toHaveLength(20)
    const more = screen.getByTestId('settings-agent-events-all')
    expect(more).toHaveTextContent('Show all 23')

    await userEvent.click(more)
    expect(screen.getAllByTestId('settings-agent-event')).toHaveLength(23)
    expect(screen.queryByTestId('settings-agent-events-all')).not.toBeInTheDocument()
  })

  it('lists stale rules and removes one, re-reading once Rust has answered', async () => {
    mockCommand('browser_status', () => status(true))
    mockCommand('audit_agent_activity', () => activity({ staleRules: [rule()] }))
    await openSection()

    const row = await screen.findByTestId('settings-agent-stale-rule')
    expect(row).toHaveTextContent('GitHub')
    expect(row).toHaveTextContent('https://github.com/login · Work Chrome · written 11/02/2029')

    mockCommand('audit_agent_activity', () => activity())
    await userEvent.click(within(row).getByTestId('settings-agent-stale-rule-remove'))
    expect(calls('agent_revoke_rule')).toEqual([{ id: 'r1' }])
    await waitFor(() =>
      expect(screen.queryByTestId('settings-agent-stale-rule')).not.toBeInTheDocument()
    )
    // Still here: agents are on, so the panel keeps its empty state.
    expect(screen.getByTestId('settings-agent-activity-empty')).toBeInTheDocument()
  })

  it('lists idle clients and forgets one', async () => {
    mockCommand('browser_status', () => status(true))
    mockCommand('audit_agent_activity', () =>
      activity({
        idleClients: [
          client(),
          client({
            key: 'other',
            name: 'Quiet Chrome',
            kind: 'extension',
            platform: undefined,
            lastSeenAtMs: undefined
          })
        ]
      })
    )
    await openSection()

    const rows = await screen.findAllByTestId('settings-agent-idle-client')
    expect(rows[0]).toHaveTextContent('Old Codex')
    expect(rows[0]).toHaveTextContent('Agent · Codex · last seen 07/01/2029')
    expect(rows[1]).toHaveTextContent('Browser extension · never seen, let in 06/01/2029')

    mockCommand('audit_agent_activity', () => activity({ idleClients: [client({ key: 'other' })] }))
    await userEvent.click(within(rows[0]).getByTestId('settings-agent-idle-client-forget'))
    expect(calls('browser_forget_client')).toEqual([{ key: KEY }])
    await waitFor(() => expect(screen.getAllByTestId('settings-agent-idle-client')).toHaveLength(1))
  })

  it('verifies the log and says so in one line', async () => {
    mockCommand('browser_status', () => status(true))
    mockCommand('audit_agent_activity', () => activity())
    mockCommand('audit_verify', () =>
      verification([writer(), writer({ writerId: 'abcdef1234567890', thisDevice: false, rows: 2 })])
    )
    await openSection()
    await screen.findByTestId('settings-agent-activity')

    await userEvent.click(screen.getByTestId('settings-audit-verify'))
    const result = await screen.findByTestId('settings-audit-verify-result')
    expect(calls('audit_verify')).toHaveLength(1)
    expect(result).toHaveTextContent('Log verified: 12 rows from 2 devices')
    expect(result).toHaveClass('text-good')
  })

  it('names the first break by device, row and what went wrong', async () => {
    mockCommand('browser_status', () => status(true))
    mockCommand('audit_agent_activity', () => activity())
    mockCommand('audit_verify', () =>
      verification([
        writer(),
        writer({
          writerId: 'abcdef1234567890',
          thisDevice: false,
          break: { seq: 7, reason: 'gap' }
        })
      ])
    )
    await openSection()
    await screen.findByTestId('settings-agent-activity')

    await userEvent.click(screen.getByTestId('settings-audit-verify'))
    const result = await screen.findByTestId('settings-audit-verify-result')
    expect(result).toHaveTextContent("Break in device abcdef12's log at row 7: a missing row")
    expect(result).toHaveClass('text-bad')

    mockCommand('audit_verify', () =>
      verification([writer({ break: { seq: 3, reason: 'hash' } })])
    )
    await userEvent.click(screen.getByTestId('settings-audit-verify'))
    await waitFor(() =>
      expect(screen.getByTestId('settings-audit-verify-result')).toHaveTextContent(
        "Break in this device's log at row 3: a row that was altered"
      )
    )
  })

  it('re-reads when Rust says a rule or grant came or went', async () => {
    mockCommand('browser_status', () => status(true))
    mockCommand('audit_agent_activity', () => activity())
    subscribeToEvents()
    await openSection()
    expect(await screen.findByTestId('settings-agent-activity-empty')).toBeInTheDocument()

    mockCommand('audit_agent_activity', () => activity({ events: [event()] }))
    act(() => emitEvent(EVENTS.agentsChanged, undefined))

    expect(await screen.findByTestId('settings-agent-event')).toHaveTextContent('GitHub')
  })
})
