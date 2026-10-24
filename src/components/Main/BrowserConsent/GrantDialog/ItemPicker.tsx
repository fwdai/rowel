import { useTranslation } from 'react-i18next'
import type { GrantCandidate } from '@/api/browser'
import RadioList from '@/components/elements/RadioList'

/**
 * Which of the page's logins the agent may be given, when the policy admits
 * more than one: title over username, as the extension's own picker would
 * show them. Rust narrows the grant to the one picked, so a yes never serves
 * a login the user was not shown.
 */
export default function ItemPicker({
  candidates,
  value,
  onChange
}: {
  candidates: GrantCandidate[]
  value: string
  onChange: (id: string) => void
}) {
  const { t } = useTranslation()

  return (
    <div className="mt-4">
      <RadioList
        options={candidates.map(candidate => ({
          value: candidate.id,
          label: candidate.title || t('Untitled'),
          meta: candidate.username ?? undefined
        }))}
        value={value}
        onChange={onChange}
        name={t('Login')}
        testidPrefix="agent-grant-item"
      />
    </div>
  )
}
