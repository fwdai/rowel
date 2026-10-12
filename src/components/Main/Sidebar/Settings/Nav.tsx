import { useTranslation } from 'react-i18next'
import { cx } from '@/utils/cx'
import { useVault, usePrefs, type Section } from '@/store'
import Chip from '@/components/elements/Chip'
import { LABEL } from '@/components/elements/tokens'
import { auditCounts } from '@/utils/vaultScore'
import Footer from './Footer'
import { GROUPS, SECTIONS } from './sections'

interface Props {
  section: Section
  onSelect: (section: Section) => void
  /** Every other section is out of reach for now (see `settingsLocked`). */
  disabled?: boolean
}

// Open audit findings, as the audit row's badge: zero (or no audit run yet)
// reads as nothing to show. Breaches count only while monitoring is on — the
// section itself shows none otherwise, and an audit that ran with the check
// on can still be the one in the store after it was switched off.
function useAuditIssues(): number {
  const audit = useVault(state => state.audit)
  const breachCheck = usePrefs(state => state.breachCheck)
  if (!audit) return 0
  const { weak, reused, breached } = auditCounts(audit)
  return weak + reused + (breachCheck ? breached : 0)
}

export default function Nav({ section, onSelect, disabled = false }: Props) {
  const { t } = useTranslation()
  const issues = useAuditIssues()

  return (
    <nav className="flex w-[240px] flex-none flex-col border-r border-line bg-rail p-4">
      <div className="px-2 text-xl font-semibold tracking-display text-text">{t('Settings')}</div>
      <div className="flex min-h-0 flex-1 flex-col overflow-y-auto">
        {GROUPS.map((group, index) => (
          <div key={group.key}>
            <div className={cx(LABEL, 'px-2 pb-2', index === 0 ? 'pt-4' : 'pt-5')}>
              {t(group.label)}
            </div>
            <ul className="m-0 flex flex-col gap-0.5 p-0">
              {SECTIONS.filter(item => item.group === group.key).map(({ key, label, Glyph }) => {
                const active = key === section
                const badge = key === 'audit' && issues > 0 ? issues : null
                return (
                  <li key={key}>
                    <button
                      type="button"
                      aria-current={active ? 'page' : undefined}
                      data-testid={`settings-nav-${key}`}
                      disabled={disabled && !active}
                      onClick={() => onSelect(key)}
                      className={cx(
                        'group flex h-10 w-full items-center gap-2.5 rounded-sm px-2 text-left text-base transition-colors',
                        active
                          ? 'bg-lens font-medium text-text shadow-lens'
                          : 'text-text2 hover:bg-hover hover:text-text focus-visible:bg-hover',
                        disabled && !active ? 'cursor-default opacity-50' : 'cursor-pointer'
                      )}
                    >
                      {/* The glyph a tier under the label: a 16px stroke lays
                          down more ink than 13px text, so the same token reads
                          darker on it. text3 is the glyph tier for exactly
                          that, and hover lifts both to the full ink together. */}
                      <span
                        className={cx(
                          'grid h-7 w-7 flex-none place-items-center rounded-sm transition-colors',
                          active
                            ? 'bg-accent text-accent-fg'
                            : 'bg-tile text-text3 group-hover:text-text'
                        )}
                      >
                        <Glyph size={16} />
                      </span>
                      <span className="min-w-0 flex-1 truncate">{t(label)}</span>
                      {badge !== null && (
                        <Chip tone="accent" testid="settings-nav-audit-badge">
                          {badge}
                        </Chip>
                      )}
                    </button>
                  </li>
                )
              })}
            </ul>
          </div>
        ))}
      </div>
      <Footer />
    </nav>
  )
}
