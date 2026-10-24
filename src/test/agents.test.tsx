import { describe, it, expect } from 'vitest'
import { act, render, screen, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { EVENTS } from '@/api/events'
import type { AgentGrant, AgentRule, BrowserStatus } from '@/api/browser'
import Settings from '@/components/Main/Sidebar/Settings'
import { openSettings } from '@/store'
import { subscribeToEvents } from '@/store/events'
import { appStatusDefault, calls, mockCommand } from './ipc'
import { emitEvent } from './events'
import { seedApp } from './utils'

const status = (overrides: Partial<BrowserStatus> = {}): BrowserStatus => ({
  enabled: true,
  keepassxcCompat: true,
  agents: { enabled: false, defaultStandingDays: 7 },
  browsers: [],
  clients: [{ name: 'Work Chrome', key: 'AAAAbbbbCCCCddddEEEEffffGGGGhhhhIIIIjjjjKKK=' }],
  ...overrides
})

const agentsOn = () => status({ agents: { enabled: true, defaultStandingDays: 7 } })

const rule = (overrides: Partial<AgentRule> = {}): AgentRule => ({
  id: 'r1',
  clientKey: 'AAAAbbbbCCCCddddEEEEffffGGGGhhhhIIIIjjjjKKK=',
  clientName: 'Work Chrome',
  itemId: 'gh1',
  itemTitle: 'GitHub',
  origin: 'https://github.com/login',
  expiresAtMs: Date.UTC(2030, 0, 15),
  createdAtMs: Date.UTC(2030, 0, 8),
  ...overrides
})

const grant = (overrides: Partial<AgentGrant> = {}): AgentGrant => ({
  id: 'g1',
  clientKey: 'AAAAbbbbCCCCddddEEEEffffGGGGhhhhIIIIjjjjKKK=',
  clientName: 'Work Chrome',
  itemId: 'gh1',
  itemTitle: 'GitHub',
  origin: 'https://github.com/login',
  state: 'active',
  expiresAtMs: Date.now() + 12 * 60_000,
  uses: 1,
  ...overrides
})

const openSection = async () => {
  openSettings('browser')
  render(<Settings />)
  return screen.findByTestId('settings-agents-toggle')
}

describe('Settings › Browser & agents › Agents', () => {
  it('is titled for both, and off by default with the lists out of sight', async () => {
    mockCommand('browser_status', () => status())
    const toggle = await openSection()

    expect(screen.getByRole('heading', { name: 'Browser & agents' })).toBeInTheDocument()
    expect(toggle).toHaveAttribute('aria-checked', 'false')
    expect(
      screen.getByText(
        'Let AI agents use logins through the browser extension'
      )
    ).toBeInTheDocument()
    expect(screen.getByTestId('settings-agents-days')).toBeDisabled()
    expect(screen.queryByTestId('settings-agents-rules-empty')).not.toBeInTheDocument()
    expect(screen.queryByTestId('settings-agents-grants-empty')).not.toBeInTheDocument()
  })

  it('turns agents on, and then shows the empty lists', async () => {
    mockCommand('browser_status', () => status())
    mockCommand('browser_set_agents_enabled', ({ enabled }) =>
      status({ agents: { enabled: enabled as boolean, defaultStandingDays: 7 } })
    )
    await openSection()

    await userEvent.click(screen.getByTestId('settings-agents-toggle'))

    expect(calls('browser_set_agents_enabled')).toEqual([{ enabled: true }])
    expect(screen.getByTestId('settings-agents-toggle')).toHaveAttribute('aria-checked', 'true')
    expect(await screen.findByTestId('settings-agents-rules-empty')).toBeInTheDocument()
    expect(await screen.findByTestId('settings-agents-grants-empty')).toBeInTheDocument()
    expect(screen.getByTestId('settings-agents-days')).toBeEnabled()
  })

  it('commits a new default length when the field is left, clamped to 1–90', async () => {
    mockCommand('browser_status', agentsOn)
    mockCommand('browser_set_agents_standing_days', ({ days }) =>
      status({ agents: { enabled: true, defaultStandingDays: days as number } })
    )
    await openSection()
    const days = screen.getByTestId('settings-agents-days')
    expect(days).toHaveValue(7)

    await userEvent.clear(days)
    await userEvent.type(days, '120')
    await userEvent.tab()

    expect(calls('browser_set_agents_standing_days')).toEqual([{ days: 90 }])
    expect(screen.getByTestId('settings-agents-days')).toHaveValue(90)
  })

  it('lists rules and active grants, and removes or ends them', async () => {
    mockCommand('browser_status', agentsOn)
    mockCommand('agent_rules', () => [rule(), rule({ id: 'r2', expiresAtMs: null, origin: null })])
    mockCommand('agent_grants', () => [grant(), grant({ id: 'g2', state: 'expired' })])
    await openSection()

    const rules = await screen.findAllByTestId('settings-agents-rule')
    expect(rules).toHaveLength(2)
    expect(rules[0]).toHaveTextContent('GitHub')
    expect(rules[0]).toHaveTextContent('https://github.com/login · Work Chrome · until 01/15/2030')
    expect(rules[1]).toHaveTextContent('Work Chrome · until removed')

    const grants = screen.getAllByTestId('settings-agents-grant')
    expect(grants).toHaveLength(2)
    expect(grants[0]).toHaveTextContent('Work Chrome · https://github.com/login · Expires in 12 minutes')
    expect(within(grants[0]).getByTestId('settings-agents-grant-end')).toBeInTheDocument()
    expect(grants[1]).toHaveTextContent('Ended')
    expect(within(grants[1]).queryByTestId('settings-agents-grant-end')).not.toBeInTheDocument()

    mockCommand('agent_rules', () => [rule({ id: 'r2', expiresAtMs: null })])
    await userEvent.click(within(rules[0]).getByTestId('settings-agents-rule-remove'))
    expect(calls('agent_revoke_rule')).toEqual([{ id: 'r1' }])
    expect(await screen.findAllByTestId('settings-agents-rule')).toHaveLength(1)

    mockCommand('agent_grants', () => [grant({ state: 'revoked' })])
    await userEvent.click(within(grants[0]).getByTestId('settings-agents-grant-end'))
    expect(calls('agent_end_grant')).toEqual([{ id: 'g1' }])
    expect((await screen.findAllByTestId('settings-agents-grant'))[0]).toHaveTextContent('Ended')
  })

  it('re-reads both lists when Rust says they changed', async () => {
    mockCommand('browser_status', agentsOn)
    subscribeToEvents()
    await openSection()
    expect(await screen.findByTestId('settings-agents-rules-empty')).toBeInTheDocument()

    mockCommand('agent_rules', () => [rule()])
    mockCommand('agent_grants', () => [grant()])
    act(() => emitEvent(EVENTS.agentsChanged, undefined))

    expect(await screen.findByTestId('settings-agents-rule')).toHaveTextContent('GitHub')
    expect(await screen.findByTestId('settings-agents-grant')).toHaveTextContent('Work Chrome')
  })
})

describe('Settings › Lockboxes › edit › agents', () => {
  const edit = async (id: string) => {
    const workspaces = {
      workspaces: [
        { id: 'default', name: null },
        { id: 'w2', name: 'Work', vaultId: 'cafe', synced: true, itemCount: 12 }
      ],
      activeWorkspace: 'default'
    }
    seedApp(workspaces)
    mockCommand('app_status', () => ({ ...appStatusDefault(), ...workspaces }))
    openSettings('workspaces')
    render(<Settings />)
    await userEvent.click(await screen.findByTestId(`workspace-menu-${id}`))
    await userEvent.click(screen.getByTestId(`workspace-edit-${id}`))
    return screen.findByTestId('workspace-edit-agents')
  }

  it('offers the agent-access switch, off, with its note, and saves it', async () => {
    const toggle = await edit('w2')
    expect(toggle).toHaveAttribute('aria-checked', 'false')
    expect(screen.getByText('Agents can use items in this lockbox')).toBeInTheDocument()
    expect(
      screen.getByText(
        'While Rowel is unlocked, an agent with a standing rule uses items here without asking.'
      )
    ).toBeInTheDocument()
    expect(screen.getByTestId('workspace-edit-save')).toBeDisabled()

    await userEvent.click(toggle)
    expect(screen.getByTestId('workspace-edit-save')).toBeEnabled()
    await userEvent.click(screen.getByTestId('workspace-edit-save'))

    expect(calls('workspace_set_agents')).toEqual([{ id: 'w2', enabled: true }])
    expect(calls('workspace_rename')).toEqual([])
    expect(calls('workspace_set_color')).toEqual([])
  })
})
