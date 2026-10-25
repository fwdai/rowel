import { useTranslation } from 'react-i18next'
import type { AgentGrantAsk } from '@/api/browser'

/**
 * What would happen if the user says yes, in one line per mode — and, for a
 * command, the line itself verbatim in a block, since that is the one process
 * the value goes to and the person should read what it is before allowing.
 */
export default function Target({ ask }: { ask: AgentGrantAsk }) {
  const { t } = useTranslation()
  const target = ask.targets[0]

  if (ask.mode === 'env') {
    return (
      <>
        <p className="mt-1.5 text-base text-text2">
          {t(
            'The value goes into the environment of that one command only, and its output is checked for it.'
          )}
        </p>
        <pre
          className="mt-3 overflow-x-auto rounded-md bg-field px-3 py-2 font-mono text-sm text-text"
          data-testid="agent-grant-command"
        >
          {target?.kind === 'command' ? target.line : ''}
        </pre>
      </>
    )
  }
  if (ask.mode === 'header') {
    return (
      <p className="mt-1.5 text-base text-text2">
        {t(
          'The proxy swaps the value into the request itself on the way out; the agent never sees it.'
        )}
      </p>
    )
  }
  return (
    <p className="mt-1.5 text-base text-text2">
      {t('It would fill the login through the browser extension, as you would yourself.')}
    </p>
  )
}
