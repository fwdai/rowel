import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { auditVerify } from '@/api/browser'
import { describeError } from '@/api/errors'
import { useLatestRequest } from '@/hooks/useLatestRequest'
import Button from '@/components/elements/Button'
import Note from '@/components/elements/Note'
import SettingsGroup from '@/components/elements/SettingsGroup'
import SettingsRow from '@/components/elements/SettingsRow'
import { verificationWords } from './words'

type Result = { tone: 'good' | 'bad'; text: string }

// The one thing the log promises that a list of rows cannot show: that no
// row was taken out or changed. Every device's chain is walked by Rust on
// demand, and the answer stays under the button until the next press — a
// line, not a dialog, since a log that holds is the expected answer.
export default function Verify() {
  const { t } = useTranslation()
  const [running, setRunning] = useState(false)
  const [result, setResult] = useState<Result | null>(null)
  const begin = useLatestRequest()

  const run = () => {
    const current = begin()
    setRunning(true)
    auditVerify()
      .then(verification => {
        if (!current()) return
        setResult({
          tone: verification.ok ? 'good' : 'bad',
          text: verificationWords(t, verification.writers)
        })
      })
      .catch(reason => current() && setResult({ tone: 'bad', text: describeError(reason) }))
      .finally(() => current() && setRunning(false))
  }

  return (
    <SettingsGroup label={t('Log')}>
      <SettingsRow
        label={t('Verify the log')}
        description={t(
          "Walks every device's chain of rows and reports the first one that does not hold."
        )}
        control={
          <Button
            variant="ghost"
            size="md"
            loading={running}
            onClick={run}
            testid="settings-audit-verify"
          >
            {t('Verify')}
          </Button>
        }
      >
        {result && (
          <Note tone={result.tone} testid="settings-audit-verify-result">
            {result.text}
          </Note>
        )}
      </SettingsRow>
    </SettingsGroup>
  )
}
