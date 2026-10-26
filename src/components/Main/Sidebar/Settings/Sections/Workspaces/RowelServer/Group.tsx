import { useEffect, useState } from 'react'
import { orgEnrollmentFingerprint } from '@/api/org'
import { providerName } from '@/api/provider'
import { loadOrgConnections, useOrg } from '@/store'
import SettingsGroup from '@/components/elements/SettingsGroup'
import FingerprintRow from './FingerprintRow'
import ConnectRow from './ConnectRow'
import ConnectionCard from './ConnectionCard'

/**
 * The Rowel Server group itself: this device's enrollment key, the way to
 * connect to a company's server, and one card per server connected — its
 * devices to approve and its lockboxes to add. Mounted only where the build
 * offers the flow (see `index.tsx`), so its reads reach a Rust that answers.
 *
 * The fingerprint is read once here and handed down: the key row shows it,
 * and a connection still waiting for approval shows it again beside the
 * sentence that says who compares it.
 */
export default function Group() {
  const connections = useOrg(state => state.connections)
  const [fingerprint, setFingerprint] = useState<string | null>(null)

  useEffect(() => {
    void loadOrgConnections()
  }, [])

  useEffect(() => {
    let live = true
    orgEnrollmentFingerprint()
      .then(value => {
        if (live) setFingerprint(value)
      })
      .catch(() => {})
    return () => {
      live = false
    }
  }, [])

  return (
    <SettingsGroup label={providerName('rowelServer')}>
      <FingerprintRow fingerprint={fingerprint} />
      <ConnectRow />
      {connections?.map(connection => (
        <ConnectionCard key={connection.url} connection={connection} fingerprint={fingerprint} />
      ))}
    </SettingsGroup>
  )
}
