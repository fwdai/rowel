import { useTranslation } from 'react-i18next'
import Select from '@/components/elements/Select'
import SettingsRow from '@/components/elements/SettingsRow'
import { CARD } from '@/components/elements/tokens'
import type { ImportFormat } from '@/api/imports'
import { LayersGlyph } from '../../../../icons'
import { OTHERS } from './apps'

interface Props {
  active: string | null
  disabled: boolean
  onFormat: (format: ImportFormat) => void
}

// The apps that get no tile of their own, in one dropdown: picking one opens
// the file picker just as a tile does. The row names the one picked, and shows
// the prompt again once another source is chosen instead.
export default function OtherApps({ active, disabled, onFormat }: Props) {
  const { t } = useTranslation()
  const current = OTHERS.find(app => app.key === active)
  return (
    <div className={CARD}>
      <SettingsRow
        icon={<LayersGlyph size={16} />}
        iconActive={current !== undefined}
        label={t('Another app')}
        description={OTHERS.map(app => t(app.name)).join(', ')}
        control={
          <Select
            name="import_other_app"
            label={t('Another app')}
            value={current?.key ?? ''}
            disabled={disabled}
            onChange={e => {
              const app = OTHERS.find(a => a.key === e.target.value)
              if (app) onFormat(app.format)
            }}
            className="w-[236px] max-w-full"
          >
            <option value="" disabled>
              {t('Choose an app…')}
            </option>
            {OTHERS.map(app => (
              <option key={app.key} value={app.key} data-testid={`import-tile-${app.key}`}>
                {t(app.name)} · {t(app.hint)}
              </option>
            ))}
          </Select>
        }
      />
    </div>
  )
}
