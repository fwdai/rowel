import { useTranslation } from 'react-i18next'
import type { TKey } from '@/i18n'
import { isOpenableUrl, openLink } from '@/api/openLink'
import { useFavicon } from '@/hooks/useFavicon'
import { ExternalGlyph, GlobeGlyph } from '../../Main/icons'
import IconButton from '../IconButton'
import Field from './Field'
import { useField } from './context'
import { hostOf, normalizeUrl } from './formats'

export default function UrlField({
  name = 'website',
  label = 'Website'
}: {
  name?: string
  label?: TKey
}) {
  const { t } = useTranslation()
  const { value } = useField(name)
  const icon = useFavicon(hostOf(value) || undefined)

  return (
    <Field
      name={name}
      label={label}
      placeholder="example.com"
      normalize={normalizeUrl}
      prefix={
        icon ? (
          <img src={icon} alt="" className="h-4 w-4 rounded-sm object-cover" />
        ) : (
          <GlobeGlyph size={14} />
        )
      }
      actions={
        // Offered only for a link the OS opener will actually take (http/https),
        // so there is no button that answers a click by doing nothing.
        isOpenableUrl(value) ? (
          <IconButton title={t('Open')} onClick={() => openLink(value)}>
            <ExternalGlyph />
          </IconButton>
        ) : undefined
      }
    />
  )
}
