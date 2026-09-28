import { useCallback, useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import {
  browserForgetClient,
  browserSetEnabled,
  browserStatus,
  type BrowserStatus
} from '@/api/browser'
import { describeError } from '@/api/errors'
import { useLatestRequest } from '@/hooks/useLatestRequest'
import { useUi } from '@/store'
import SettingsGroup from '@/components/elements/SettingsGroup'
import SettingsRow from '@/components/elements/SettingsRow'
import Toggle from '@/components/elements/Toggle'
import { ExtensionGlyph } from '@/components/Main/icons'
import Browsers from './Browsers'
import Clients from './Clients'

// Settings › Browser extension. The status is Rust's, read on the way in and
// replaced by the answer to every change, so nothing here is kept in a store.
export default function Browser() {
  const { t } = useTranslation()
  const [status, setStatus] = useState<BrowserStatus | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  // A forget can land after a toggle that started later; only the newest
  // answer is the status as it stands.
  const begin = useLatestRequest()
  const load = useCallback(
    (request: () => Promise<BrowserStatus>) => {
      const current = begin()
      return request()
        .then(next => {
          if (!current()) return
          setStatus(next)
          setError(null)
        })
        .catch(reason => current() && setError(describeError(reason)))
    },
    [begin]
  )

  // On the way in, and again whenever the consent dialog lets an extension in
  // behind this section's back (`browser:clients`). A refresh that arrives
  // while a change is in flight waits for it: taken at once it would be the
  // latest request, could read the status from before the change committed,
  // and would then have the change's own answer thrown away as stale.
  const inFlight = useRef(false)
  const refreshAfter = useRef(false)
  const clientsSeq = useUi(state => state.browserClientsSeq)
  useEffect(() => {
    if (inFlight.current) refreshAfter.current = true
    else void load(browserStatus)
  }, [load, clientsSeq])

  // One change at a time: the toggle and every Forget button wait for the
  // answer in flight, so two forgets cannot cross and leave the earlier one's
  // client on screen.
  const change = (request: () => Promise<BrowserStatus>) => {
    inFlight.current = true
    setBusy(true)
    void load(request).finally(() => {
      inFlight.current = false
      setBusy(false)
      if (refreshAfter.current) {
        refreshAfter.current = false
        void load(browserStatus)
      }
    })
  }
  const toggle = (enabled: boolean) => change(() => browserSetEnabled(enabled))
  const forget = (key: string) => change(() => browserForgetClient(key))

  const enabled = !!status?.enabled
  const label = t('Fill logins in your browser')

  return (
    <>
      <SettingsGroup label={t('Autofill')}>
        <SettingsRow
          label={label}
          icon={<ExtensionGlyph />}
          iconActive={enabled}
          description={t(
            "{{appName}} fills logins through the KeePassXC-Browser extension. Install it from your browser's store, then turn this on."
          )}
          control={
            <Toggle
              name="browser"
              checked={enabled}
              disabled={!status || busy}
              onChange={toggle}
              aria-label={label}
              testid="settings-browser-toggle"
            />
          }
        >
          {error && (
            <span data-testid="settings-browser-error" className="text-sm text-bad">
              {error}
            </span>
          )}
        </SettingsRow>
      </SettingsGroup>
      {status && (
        <>
          <Browsers browsers={status.browsers} enabled={enabled} />
          <Clients clients={status.clients} busy={busy} onForget={forget} />
        </>
      )}
    </>
  )
}
