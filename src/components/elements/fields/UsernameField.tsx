import { useMemo } from 'react'
import type { TKey } from '@/i18n'
import { useUsernames } from '@/hooks/useUsernames'
import { AtGlyph, UserGlyph } from '../../Main/icons'
import Field from './Field'
import { isEmail } from './formats'

export default function UsernameField({
  name = 'username',
  label = 'Username',
  required
}: {
  name?: string
  label?: TKey
  required?: boolean
}) {
  // What the lockbox's logins already sign in with, each marked as the address
  // or the handle it is: people sign in with the same few of both.
  const usernames = useUsernames()
  const suggestions = useMemo(
    () =>
      usernames.map(option => ({
        ...option,
        glyph: isEmail(option.value) ? <AtGlyph size={14} /> : <UserGlyph size={14} />
      })),
    [usernames]
  )

  return (
    <Field
      name={name}
      label={label}
      required={required}
      placeholder="octocat"
      prefix={<UserGlyph size={14} />}
      suggestions={suggestions}
    />
  )
}
