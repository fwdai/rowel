import { useMemo } from 'react'
import type { TKey } from '@/i18n'
import { useUsernames } from '@/hooks/useUsernames'
import { AtGlyph } from '../../Main/icons'
import Field from './Field'
import { emailError, isEmail } from './formats'

export default function EmailField({
  name = 'email',
  label = 'Email'
}: {
  name?: string
  label?: TKey
}) {
  // The addresses the vault's logins already sign in with: a username that is
  // an email is as good a guess at this row as there is.
  const usernames = useUsernames()
  const emails = useMemo(() => usernames.filter(option => isEmail(option.value)), [usernames])

  return (
    <Field
      name={name}
      label={label}
      type="email"
      placeholder="name@example.com"
      prefix={<AtGlyph size={14} />}
      check={emailError}
      suggestions={emails}
    />
  )
}
