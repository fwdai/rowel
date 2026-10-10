import { useTranslation } from 'react-i18next'
import type { Passkey } from '@/api/types'
import Panel from '../../Panel'
import { SECTION_LABEL } from '../../tokens'
import { useFields } from '../context'
import PasskeyRow from './Row'

// A draft array is not necessarily passkeys — a login also carries tags — so
// the key is narrowed rather than trusted. `rpId` is the field every passkey
// has and no tag ever does.
const isPasskey = (value: unknown): value is Passkey =>
  typeof value === 'object' && value !== null && 'rpId' in value

/**
 * The passkeys a login holds, in both modes.
 *
 * A passkey is issued by the site, not typed here, so there is nothing about
 * one to edit — only the decision to keep it. The editing face is therefore the
 * reading face plus a remove button, and the block disappears entirely when
 * there are no passkeys instead of offering an empty row to fill in (creating
 * one is the authenticator's job, not the editor's).
 */
export default function Passkeys({ name = 'passkeys' }) {
  const { t } = useTranslation()
  const { entry, set } = useFields()
  const raw = entry[name]
  const passkeys: Passkey[] = Array.isArray(raw) ? raw.filter(isPasskey) : []

  if (passkeys.length === 0) return null

  return (
    <div className="mt-4">
      <span className={SECTION_LABEL}>{t('Passkeys')}</span>
      <Panel>
        {passkeys.map(passkey => (
          <PasskeyRow
            key={passkey.credentialId}
            passkey={passkey}
            onRemove={
              set
                ? () => set(name, passkeys.filter(kept => kept !== passkey))
                : undefined
            }
          />
        ))}
      </Panel>
    </div>
  )
}
