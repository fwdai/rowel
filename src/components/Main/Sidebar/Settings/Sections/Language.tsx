import { useTranslation } from 'react-i18next'
import { usePrefs, setPref, DATE_FORMATS } from '@/store'
import { LANGUAGES } from '@/i18n'
import { dates } from '@/utils/time'
import { useNow } from '@/hooks/useNow'
import { cx } from '@/utils/cx'
import SettingsRow from '@/components/elements/SettingsRow'
import MenuSelect from '@/components/elements/MenuSelect'
import AccentSwatches from '@/components/elements/AccentSwatches'
import { CARD, CARD_OPEN, LABEL } from '@/components/elements/tokens'
import { CalendarGlyph, GlobeGlyph, PaletteGlyph } from '@/components/Main/icons'
import ThemePicker from './General/ThemePicker'

// Each language named in its own tongue, with its locale code beside it.
const LOCALES = Object.entries(LANGUAGES).map(([value, label]) => ({ value, label, meta: value }))

// The General section (key `language`): appearance, then language & region.
export default function Language() {
  const { t, i18n } = useTranslation()
  const accent = usePrefs(state => state.accent)
  const format = usePrefs(state => state.dateFormat)

  // Each row label doubles as its control's accessible name.
  const languageLabel = t('Language')
  const formatLabel = t('Date format')
  const accentLabel = t('Accent')

  // Each pattern shown as what it makes of today, with its letters beside it.
  // Re-read each minute so the labels roll over at midnight.
  const today = useNow(60_000)
  const formats = DATE_FORMATS.map(value => ({
    value,
    label: dates(value).shortDate(today),
    meta: value
  }))

  return (
    <>
      <section className="mb-7">
        <div className={cx(LABEL, 'mb-2')}>{t('Appearance')}</div>
        <ThemePicker />
        <div className={cx(CARD, 'mt-3')}>
          <SettingsRow
            label={accentLabel}
            icon={<PaletteGlyph />}
            control={
              <AccentSwatches
                name={accentLabel}
                value={accent}
                onChange={next => setPref('accent', next)}
                testidPrefix="settings-accent"
              />
            }
          />
        </div>
      </section>

      <section className="mb-7">
        <div className={cx(LABEL, 'mb-2')}>{t('Language & region')}</div>
        {/* The language and date menus hang out of their card. */}
        <div className={CARD_OPEN}>
          <SettingsRow
            label={languageLabel}
            description={t('Menus, dialogs and autofill prompts. Applies instantly.')}
            icon={<GlobeGlyph />}
            control={
              <MenuSelect
                label={languageLabel}
                options={LOCALES}
                value={i18n.resolvedLanguage ?? ''}
                onChange={locale => void i18n.changeLanguage(locale)}
                testidPrefix="settings-locale"
              />
            }
          />
          <SettingsRow
            label={formatLabel}
            description={t('Used in item history, expiry dates and audits')}
            icon={<CalendarGlyph />}
            control={
              <MenuSelect
                label={formatLabel}
                options={formats}
                value={format}
                onChange={next => setPref('dateFormat', next)}
                testidPrefix="settings-date-format"
              />
            }
          />
        </div>
      </section>
    </>
  )
}
