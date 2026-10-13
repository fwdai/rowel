import { useId, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { usePrefs } from '@/store'
import Chip from '@/components/elements/Chip'
import SettingsGroup from '@/components/elements/SettingsGroup'
import SettingsRow from '@/components/elements/SettingsRow'
import Toggle from '@/components/elements/Toggle'
import { TEXT_LINK } from '@/components/elements/tokens'
import { ActivityGlyph, LockGlyph, ShieldGlyph } from '@/components/Main/icons'
import BreachExplainer from './BreachExplainer'
import { setBreachCheck } from './breach'

// Breach monitoring is the one monitor with a switch; weak and reused are
// worked out on the device from what it already holds, so they are simply on.
export default function Monitors() {
  const { t } = useTranslation()
  const breachCheck = usePrefs(state => state.breachCheck)
  const [explained, setExplained] = useState(false)
  const explainerId = useId()

  return (
    <SettingsGroup label={t('Monitors')}>
      <SettingsRow
        label={t('Breach monitoring')}
        description={
          <>
            {t('Checks passwords against known leaks without revealing them.')}{' '}
            <button
              type="button"
              data-testid="settings-breach-explain"
              aria-expanded={explained}
              aria-controls={explainerId}
              onClick={() => setExplained(open => !open)}
              className={TEXT_LINK}
            >
              {explained ? t('Hide details') : t('How it works')}
            </button>
          </>
        }
        icon={<ShieldGlyph />}
        iconActive={breachCheck}
        control={
          <Toggle
            name="breachCheck"
            checked={breachCheck}
            onChange={setBreachCheck}
            aria-label={t('Breach monitoring')}
            testid="settings-breach-toggle"
          />
        }
      >
        {explained && <BreachExplainer id={explainerId} />}
      </SettingsRow>
      <SettingsRow
        label={t('Weak & reused passwords')}
        description={t('Checked on this device every time an item changes')}
        icon={<ActivityGlyph />}
        control={
          <Chip>
            <LockGlyph size={11} />
            {t('Always on')}
          </Chip>
        }
      />
    </SettingsGroup>
  )
}
