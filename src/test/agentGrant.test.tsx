import { describe, it, expect, vi } from 'vitest'
import { act, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { EVENTS } from '@/api/events'
import { ASSOCIATE_TIMEOUT_MS, type AgentGrantAsk } from '@/api/browser'
import BrowserConsent from '@/components/Main/BrowserConsent'
import { useUi } from '@/store'
import { clearSession } from '@/store/app'
import { subscribeToEvents } from '@/store/events'
import { calls } from './ipc'
import { emitEvent } from './events'

const ASK: AgentGrantAsk = {
  id: 'ask-1',
  clientName: 'Claude in Chrome',
  clientKind: 'extension',
  targets: [{ kind: 'origin', origin: 'https://github.com/login' }],
  mode: 'fill',
  endsSession: false,
  stepUp: false,
  candidates: [
    { id: 'gh1', title: 'GitHub', username: 'octocat' },
    { id: 'gh2', title: 'GitHub (work)', username: 'octo@work.example' }
  ],
  defaultStandingDays: 7
}

const DEPLOY = 'node deploy.js --prod'

/** An agent's ask for its Stripe key in requests to the API. */
const HEADER_ASK: AgentGrantAsk = {
  ...ASK,
  id: 'ask-h',
  clientName: 'Claude Code',
  clientKind: 'agent',
  targets: [{ kind: 'http', host: 'api.stripe.com', path: '/v1/*', methods: ['GET', 'POST'] }],
  mode: 'header',
  candidates: [{ id: 'stripe', title: 'Stripe', username: null }]
}

/** A sidecar's ask for an env file's variable in one command. */
const ENV_ASK: AgentGrantAsk = {
  ...ASK,
  id: 'ask-e',
  clientName: 'rowel run',
  clientKind: 'sidecar',
  targets: [{ kind: 'command', line: DEPLOY }],
  mode: 'env',
  candidates: [{ id: 'ci', title: 'CI', username: null }]
}

describe('the agent grant dialog', () => {
  const ask = async (payload: AgentGrantAsk = ASK) => {
    subscribeToEvents()
    render(<BrowserConsent />)
    act(() => emitEvent(EVENTS.agentGrant, payload))
    return screen.findByTestId('agent-grant-dialog')
  }

  // The ask was refused behind the dialog's back (the Agents switch went
  // off): the dialog closes, and a later ask's id does not close it.
  it('closes when Rust says its ask is over, and only for its own ask', async () => {
    await ask()
    act(() => emitEvent(EVENTS.agentGrantClosed, { id: 'another-ask' }))
    expect(screen.getByTestId('agent-grant-dialog')).toBeInTheDocument()

    act(() => emitEvent(EVENTS.agentGrantClosed, { id: 'ask-1' }))
    expect(screen.queryByTestId('agent-grant-dialog')).not.toBeInTheDocument()
    expect(useUi.getState().consentAsk).toBeNull()
    expect(calls('agent_grant_respond')).toEqual([])
  })

  // An agent's ask, as against the extension's own: it asks to be logged in
  // rather than to use a login, says why in its own words, and the dialog
  // warns of the sign-out that follows when it is done.
  it("names an agent's ask as a login, quotes its reason and warns of the sign-out", async () => {
    await ask({
      ...ASK,
      id: 'ask-7',
      clientName: 'Claude Code',
      clientKind: 'agent',
      targets: [{ kind: 'origin', origin: 'https://github.com' }],
      reason: 'deploying the <docs> site',
      endsSession: true
    })
    expect(screen.getByTestId('agent-grant-title')).toHaveTextContent(
      'Claude Code wants to log into https://github.com'
    )
    expect(screen.getByTestId('agent-grant-reason')).toHaveTextContent(
      'Its reason: “deploying the <docs> site”'
    )
    expect(screen.getByTestId('agent-grant-ends-session')).toHaveTextContent(
      'When this grant ends, Rowel will sign the browser that filled it out of https://github.com and the rest of its site.'
    )

    await userEvent.click(screen.getByTestId('agent-grant-once'))
    expect(calls('agent_grant_respond')).toEqual([
      { id: 'ask-7', answer: { itemId: 'gh1', standingDays: null } }
    ])
  })

  // A header ask names the API as the agent wrote it, methods first, and
  // says the proxy does the swapping; nothing is filled, so no sign-out.
  it('names a header ask by the request it is for, and allows it once', async () => {
    await ask(HEADER_ASK)
    expect(screen.getByTestId('agent-grant-title')).toHaveTextContent(
      'Claude Code wants to send a secret in requests to GET, POST api.stripe.com/v1/*'
    )
    expect(screen.getByText(/The proxy swaps the value into the request itself/)).toBeInTheDocument()
    expect(screen.queryByTestId('agent-grant-command')).not.toBeInTheDocument()
    expect(screen.queryByTestId('agent-grant-ends-session')).not.toBeInTheDocument()
    expect(screen.getByTestId('agent-grant-item-stripe')).toHaveTextContent('Stripe')
    expect(screen.getByTestId('agent-grant-standing-note')).toHaveTextContent(
      'this client sends this item in requests to that place without asking'
    )

    await userEvent.click(screen.getByTestId('agent-grant-once'))
    expect(calls('agent_grant_respond')).toEqual([
      { id: 'ask-h', answer: { itemId: 'stripe', standingDays: null } }
    ])
  })

  // An env ask shows the one command line verbatim, in a block of its own,
  // so the person reads what the value goes to; the three answers are the
  // same as for any grant.
  it('shows an env ask the command line it is for, and takes every answer', async () => {
    await ask(ENV_ASK)
    expect(screen.getByTestId('agent-grant-title')).toHaveTextContent(
      'rowel run wants to give a secret to a command'
    )
    expect(screen.getByTestId('agent-grant-command')).toHaveTextContent(DEPLOY)
    expect(screen.getByText(/environment of that one command only/)).toBeInTheDocument()
    expect(screen.queryByTestId('agent-grant-ends-session')).not.toBeInTheDocument()
    expect(screen.getByTestId('agent-grant-standing-note')).toHaveTextContent(
      'this client gives this item to exactly that command line without asking'
    )

    await userEvent.click(screen.getByTestId('agent-grant-once'))

    act(() => emitEvent(EVENTS.agentGrant, { ...ENV_ASK, id: 'ask-e2' }))
    const days = await screen.findByTestId('agent-grant-days')
    await userEvent.clear(days)
    await userEvent.type(days, '400')
    await userEvent.click(screen.getByTestId('agent-grant-standing'))

    act(() => emitEvent(EVENTS.agentGrant, { ...ENV_ASK, id: 'ask-e3' }))
    await userEvent.click(await screen.findByTestId('agent-grant-deny'))

    expect(calls('agent_grant_respond')).toEqual([
      { id: 'ask-e', answer: { itemId: 'ci', standingDays: null } },
      { id: 'ask-e2', answer: { itemId: 'ci', standingDays: 90 } },
      { id: 'ask-e3', answer: null }
    ])
  })

  it('names who asks and where, lists the candidates with the first picked, and allows once', async () => {
    await ask()
    expect(screen.getByTestId('agent-grant-title')).toHaveTextContent(
      'Claude in Chrome wants to use a login on https://github.com/login'
    )
    expect(screen.queryByTestId('agent-grant-reason')).not.toBeInTheDocument()
    expect(screen.queryByTestId('agent-grant-ends-session')).not.toBeInTheDocument()
    expect(screen.getByTestId('agent-grant-item-gh1')).toHaveAttribute('aria-checked', 'true')
    expect(screen.getByTestId('agent-grant-item-gh1')).toHaveTextContent('GitHub')
    expect(screen.getByTestId('agent-grant-item-gh1')).toHaveTextContent('octocat')
    expect(screen.getByTestId('agent-grant-item-gh2')).toHaveAttribute('aria-checked', 'false')
    expect(screen.queryByTestId('agent-grant-step-up')).not.toBeInTheDocument()
    expect(screen.getByTestId('agent-grant-standing-note')).toHaveTextContent(
      'this client fills this login on this site without asking'
    )

    await userEvent.click(screen.getByTestId('agent-grant-once'))

    expect(calls('agent_grant_respond')).toEqual([
      { id: 'ask-1', answer: { itemId: 'gh1', standingDays: null } }
    ])
    expect(screen.queryByTestId('agent-grant-dialog')).not.toBeInTheDocument()
    expect(useUi.getState().consentAsk).toBeNull()
  })

  it('sends the picked item for N days, with the days clamped to what a rule may have', async () => {
    await ask()
    expect(screen.getByTestId('agent-grant-days')).toHaveValue(7)
    expect(screen.getByTestId('agent-grant-standing')).toHaveTextContent('Allow for 7 days')

    await userEvent.click(screen.getByTestId('agent-grant-item-gh2'))
    const days = screen.getByTestId('agent-grant-days')
    await userEvent.clear(days)
    await userEvent.type(days, '500')
    await userEvent.tab()
    expect(days).toHaveValue(90)
    expect(screen.getByTestId('agent-grant-standing')).toHaveTextContent('Allow for 90 days')

    await userEvent.click(screen.getByTestId('agent-grant-standing'))

    expect(calls('agent_grant_respond')).toEqual([
      { id: 'ask-1', answer: { itemId: 'gh2', standingDays: 90 } }
    ])
  })

  it('never writes a rule for less than a day', async () => {
    await ask()
    const days = screen.getByTestId('agent-grant-days')
    await userEvent.clear(days)
    await userEvent.type(days, '0')
    await userEvent.tab()
    expect(days).toHaveValue(1)
    expect(screen.getByTestId('agent-grant-standing')).toHaveTextContent('Allow for 1 day')

    await userEvent.click(screen.getByTestId('agent-grant-standing'))
    expect(calls('agent_grant_respond')).toEqual([
      { id: 'ask-1', answer: { itemId: 'gh1', standingDays: 1 } }
    ])
  })

  it('sends null on Deny', async () => {
    await ask()
    await userEvent.click(screen.getByTestId('agent-grant-deny'))

    expect(calls('agent_grant_respond')).toEqual([{ id: 'ask-1', answer: null }])
    expect(screen.queryByTestId('agent-grant-dialog')).not.toBeInTheDocument()
  })

  it('names a lone candidate instead of offering a pick', async () => {
    await ask({ ...ASK, id: 'ask-2', candidates: [ASK.candidates[0]] })
    const only = screen.getByTestId('agent-grant-item-gh1')
    expect(only).not.toHaveAttribute('role', 'radio')
    expect(only).toHaveTextContent('GitHub')

    await userEvent.click(screen.getByTestId('agent-grant-once'))
    expect(calls('agent_grant_respond')).toEqual([
      { id: 'ask-2', answer: { itemId: 'gh1', standingDays: null } }
    ])
  })

  it('holds a step-up ask until the master password is typed, and sends it along', async () => {
    await ask({ ...ASK, id: 'ask-3', stepUp: true, verify: 'password' })
    expect(screen.getByTestId('agent-grant-step-up-note')).toHaveTextContent('master password')
    expect(screen.getByTestId('agent-grant-once')).toBeDisabled()
    expect(screen.getByTestId('agent-grant-standing')).toBeDisabled()
    expect(screen.getByTestId('agent-grant-deny')).toBeEnabled()

    await userEvent.type(screen.getByTestId('agent-grant-password'), 'hunter2')
    await userEvent.click(screen.getByTestId('agent-grant-once'))

    expect(calls('agent_grant_respond')).toEqual([
      { id: 'ask-3', answer: { itemId: 'gh1', standingDays: null }, password: 'hunter2' }
    ])
  })

  it('announces the biometric step and sends no password for it', async () => {
    await ask({ ...ASK, id: 'ask-4', stepUp: true, verify: 'biometric' })
    expect(screen.getByTestId('agent-grant-step-up-note')).toHaveTextContent('Touch ID')
    expect(screen.queryByTestId('agent-grant-password')).not.toBeInTheDocument()

    await userEvent.click(screen.getByTestId('agent-grant-once'))
    expect(calls('agent_grant_respond')).toEqual([
      { id: 'ask-4', answer: { itemId: 'gh1', standingDays: null } }
    ])
  })

  it('replaces an ask still on screen with a newer one, of any kind', async () => {
    await ask()
    act(() => emitEvent(EVENTS.agentGrant, { ...ASK, id: 'ask-5', clientName: 'Comet' }))
    expect(screen.getByTestId('agent-grant-title')).toHaveTextContent('Comet wants to use')

    await userEvent.click(screen.getByTestId('agent-grant-once'))
    expect(calls('agent_grant_respond')).toEqual([
      { id: 'ask-5', answer: { itemId: 'gh1', standingDays: null } }
    ])

    // An extension asking to connect takes the place of a grant dialog left
    // up, rather than opening beside it.
    act(() => emitEvent(EVENTS.agentGrant, { ...ASK, id: 'ask-6' }))
    expect(screen.getByTestId('agent-grant-dialog')).toBeInTheDocument()
    act(() =>
      emitEvent(EVENTS.browserAssociate, {
        id: 'ask-9',
        key: 'a'.repeat(43) + '=',
        kind: 'extension'
      })
    )
    expect(screen.queryByTestId('agent-grant-dialog')).not.toBeInTheDocument()
    expect(screen.getByTestId('browser-associate-modal')).toBeInTheDocument()
    expect(useUi.getState().consentAsk?.kind).toBe('associate')
  })

  it('leaves on its own once Rust has given up on the ask', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true })
    try {
      await ask()
      act(() => {
        vi.advanceTimersByTime(ASSOCIATE_TIMEOUT_MS)
      })
      expect(screen.queryByTestId('agent-grant-dialog')).not.toBeInTheDocument()
      expect(calls('agent_grant_respond')).toEqual([])
    } finally {
      vi.useRealTimers()
    }
  })

  it('is gone once the vault locks', async () => {
    await ask()
    act(() => clearSession())

    expect(screen.queryByTestId('agent-grant-dialog')).not.toBeInTheDocument()
    expect(calls('agent_grant_respond')).toEqual([])
  })
})
