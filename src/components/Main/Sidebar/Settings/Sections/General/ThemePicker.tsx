import { useSyncExternalStore } from 'react'
import { useTranslation } from 'react-i18next'
import { usePrefs, setPref } from '@/store'
import { prefersDark, type ThemePreference } from '@/theme'
import type { TKey } from '@/i18n'
import { useRadioNav } from '@/hooks/useRadioNav'
import { cx } from '@/utils/cx'
import { META, PICK_CARD, PICKED, UNPICKED } from '@/components/elements/tokens'
import ThemeMockup from './ThemeMockup'

const THEMES: { value: ThemePreference; label: TKey }[] = [
  { value: 'system', label: 'System' },
  { value: 'light', label: 'Light' },
  { value: 'dark', label: 'Dark' }
]

// The OS appearance, live: the System card says which way it currently falls,
// and that has to follow a change made while Settings is open. The preference
// store re-applies the theme on that change but nothing in it re-renders, so
// the card subscribes to the media query itself. No matchMedia (jsdom, a
// locked-down webview) means the hint just reads the initial answer.
const subscribeToOs = (onChange: () => void) => {
  const query = window.matchMedia?.('(prefers-color-scheme: dark)')
  query?.addEventListener('change', onChange)
  return () => query?.removeEventListener('change', onChange)
}

// The theme as three preview cards rather than three words: a radiogroup like
// Segmented (one tab stop, arrows select), each option a picture of the palette.
export default function ThemePicker() {
  const { t } = useTranslation()
  const theme = usePrefs(state => state.theme)
  const systemDark = useSyncExternalStore(subscribeToOs, prefersDark, () => false)
  const choose = (next: ThemePreference) => setPref('theme', next)
  const { ref, onKeyDown } = useRadioNav(
    THEMES.map(option => option.value),
    theme,
    choose
  )
  const selected = THEMES.some(option => option.value === theme)

  return (
    <div
      ref={ref}
      role="radiogroup"
      aria-label={t('Theme')}
      onKeyDown={onKeyDown}
      className="grid grid-cols-3 gap-2 md:gap-3"
    >
      {THEMES.map(({ value, label }, index) => {
        const active = value === theme
        return (
          <button
            key={value}
            type="button"
            role="radio"
            aria-checked={active}
            tabIndex={active || (!selected && index === 0) ? 0 : -1}
            data-testid={`settings-theme-${value}`}
            onClick={() => choose(value)}
            className={cx(PICK_CARD, 'p-1.5', active ? PICKED : UNPICKED)}
          >
            <ThemeMockup theme={value} />
            <span className="flex items-center gap-2 px-1.5 pt-2.5 pb-1">
              <span
                className={cx(
                  'grid h-4 w-4 flex-none place-items-center rounded-full border',
                  active ? 'border-accent' : 'border-line2'
                )}
              >
                {active && <span className="h-2 w-2 rounded-full bg-accent" />}
              </span>
              <span className="min-w-0 truncate text-base font-medium text-text">{t(label)}</span>
              {value === 'system' && (
                <span className={cx(META, 'ml-auto max-md:hidden')}>
                  {systemDark ? t('now dark') : t('now light')}
                </span>
              )}
            </span>
          </button>
        )
      })}
    </div>
  )
}
