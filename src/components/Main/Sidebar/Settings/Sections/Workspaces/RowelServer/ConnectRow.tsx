import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { cancelOrgConnect, resetOrgConnect, startOrgConnect, useOrg } from '@/store'
import { cx } from '@/utils/cx'
import Button from '@/components/elements/Button'
import Note from '@/components/elements/Note'
import SettingsRow from '@/components/elements/SettingsRow'
import { inputClass } from '@/components/elements/formStyles'
import { verbatimInput } from '@/components/elements/inputProps'
import { ServerGlyph } from '@/components/Main/icons'

/**
 * The way onto a server: its URL and Connect. The sign-in happens in the
 * browser, and Rust reports how it goes through `org:connect`, so the row
 * only waits on the store — with Cancel while the browser is out, and the
 * reason as a note when it failed.
 */
export default function ConnectRow() {
  const { t } = useTranslation()
  const connect = useOrg(state => state.connect)
  const [url, setUrl] = useState('')
  const pending = connect.status === 'pending'
  const ready = url.trim().length > 0

  const submit = () => {
    if (ready && !pending) startOrgConnect(url.trim())
  }
  const type = (next: string) => {
    setUrl(next)
    // A new URL is a new attempt: the last one's complaint goes with it.
    if (connect.status === 'error') resetOrgConnect()
  }

  const label = t('Server URL')

  return (
    <SettingsRow
      label={t('Connect to a server')}
      icon={<ServerGlyph />}
      description={t(
        "Sign in to your company's Rowel Server in the browser. This device is then enrolled as yours, once another of your devices or an admin approves it."
      )}
      testid="org-connect-row"
    >
      <div className="flex flex-wrap items-center gap-2">
        <input
          type="url"
          inputMode="url"
          value={url}
          placeholder="https://rowel.example.com"
          aria-label={label}
          disabled={pending}
          data-testid="org-connect-url"
          {...verbatimInput}
          onChange={event => type(event.target.value)}
          onKeyDown={event => event.key === 'Enter' && submit()}
          className={cx(inputClass, 'min-w-0 flex-1 basis-56')}
        />
        {pending ? (
          <Button variant="pale" testid="org-connect-cancel" onClick={cancelOrgConnect}>
            {t('Cancel')}
          </Button>
        ) : (
          <Button disabled={!ready} testid="org-connect" onClick={submit}>
            {t('Connect')}
          </Button>
        )}
      </div>
      {pending && (
        <Note testid="org-connect-waiting" className="mt-2">
          {t('Finish signing in in your browser…')}
        </Note>
      )}
      {connect.status === 'error' && (
        <Note tone="bad" testid="org-connect-error" className="mt-2">
          {connect.error || t('Something went wrong')}
        </Note>
      )}
    </SettingsRow>
  )
}
