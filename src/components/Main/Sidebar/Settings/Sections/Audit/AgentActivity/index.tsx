import { useCallback, useEffect, useState } from 'react'
import { auditAgentActivity, browserStatus, type AgentActivity as Activity } from '@/api/browser'
import { useLatestRequest } from '@/hooks/useLatestRequest'
import { useUi } from '@/store'
import Events from './Events'
import StaleRules from './StaleRules'
import IdleClients from './IdleClients'
import Verify from './Verify'

interface Panel {
  activity: Activity
  /** The Agents switch in Browser & agents. */
  agents: boolean
}

const hasSomething = ({ events, staleRules, idleClients }: Activity) =>
  events.length + staleRules.length + idleClients.length > 0

// The agent half of Vault Health. Shown to someone who has agents on, or who
// has anything in the log from when they did; a person who never uses agents
// sees nothing about them here. Both reads reject while the vault is locked,
// which is the same as having nothing to show. Re-read whenever Rust says a
// rule or grant came or went (`agents:changed`), and after a Remove or Forget
// below, so a row goes when Rust says it has gone.
export default function AgentActivity() {
  const seq = useUi(state => state.agentsSeq)
  const [panel, setPanel] = useState<Panel | null>(null)
  const [reread, setReread] = useState(0)
  const begin = useLatestRequest()
  const refresh = useCallback(() => setReread(n => n + 1), [])

  useEffect(() => {
    const current = begin()
    Promise.all([browserStatus(), auditAgentActivity()])
      .then(([status, activity]) => {
        if (current()) setPanel({ activity, agents: status.agents.enabled })
      })
      .catch(() => current() && setPanel(null))
  }, [begin, seq, reread])

  if (!panel || !(panel.agents || hasSomething(panel.activity))) return null
  const { events, staleRules, idleClients } = panel.activity

  return (
    <div data-testid="settings-agent-activity">
      <Events events={events} />
      {staleRules.length > 0 && <StaleRules rules={staleRules} onChange={refresh} />}
      {idleClients.length > 0 && <IdleClients clients={idleClients} onChange={refresh} />}
      <Verify />
    </div>
  )
}
