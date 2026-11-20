import type { TKey } from '@/i18n'
import { useUsernames } from '@/hooks/useUsernames'
import { UserGlyph } from '../../Main/icons'
import Field from './Field'

export default function UsernameField({
  name = 'username',
  label = 'Username',
  required
}: {
  name?: string
  label?: TKey
  required?: boolean
}) {
  const usernames = useUsernames()

  return (
    <Field
      name={name}
      label={label}
      required={required}
      placeholder="octocat"
      prefix={<UserGlyph size={14} />}
      suggestions={usernames}
    />
  )
}
