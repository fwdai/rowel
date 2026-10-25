import { isMobile } from '@/lib/platform'
import Hero from './Hero'
import Monitors from './Monitors'
import AgentActivity from './AgentActivity'

// Settings › Vault audit: where the vault stands, what keeps watching it, and
// what agents did with it. The last is the desktop's: a phone has no browser
// host or sidecar for an agent to come through, and Browser & agents is left
// off it for the same reason.
export default function Audit() {
  return (
    <>
      <Hero />
      <Monitors />
      {!isMobile && <AgentActivity />}
    </>
  )
}
