import { useId } from 'react'
import { useTranslation } from 'react-i18next'
import { useApp } from '@/store'
import { biometryLabel } from '@/lib/biometry'
import Field from '@/components/elements/Field'
import Note from '@/components/elements/Note'
import { inputClass } from '@/components/elements/formStyles'

/**
 * What a step-up ask adds before a yes counts. Rust decides which: the OS
 * biometric prompt where the device gates a secure store with one — it runs
 * after the answer is sent, so here it is only announced — or the master
 * password everywhere else, typed here and verified against the key the
 * lockbox is open with. A wrong one is a refusal, not a retry.
 */
export default function StepUp({
  verify,
  password,
  onPassword
}: {
  verify: 'biometric' | 'password'
  password: string
  onPassword: (password: string) => void
}) {
  const { t } = useTranslation()
  const id = useId()
  const biometry = useApp(state => state.status?.biometric.type ?? 'touch')

  return (
    <div className="mt-4 flex flex-col gap-3" data-testid="agent-grant-step-up">
      <Note testid="agent-grant-step-up-note">
        {verify === 'biometric'
          ? t('This item needs a second check: {{biometry}} will be asked for once you allow.', {
              biometry: t(biometryLabel(biometry))
            })
          : t('This item needs a second check: your master password, before the agent can use it.')}
      </Note>
      {verify === 'password' && (
        <Field id={id} label={t('Master password')}>
          <input
            id={id}
            type="password"
            autoComplete="current-password"
            value={password}
            data-testid="agent-grant-password"
            onChange={event => onPassword(event.target.value)}
            className={inputClass}
          />
        </Field>
      )}
    </div>
  )
}
