import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import type { AgentEvent } from '@/api/browser'
import { useDates } from '@/hooks/useDates'
import Chip from '@/components/elements/Chip'
import Note from '@/components/elements/Note'
import SettingsGroup from '@/components/elements/SettingsGroup'
import SettingsRow from '@/components/elements/SettingsRow'
import { TEXT_LINK } from '@/components/elements/tokens'
import { modeWords } from '../../Browser/Agents/mode'
import { actorWords, outcomeWords } from './words'

// A week of agent use is a few rows for most people and a few hundred for a
// busy one; the first screen shows this many and the rest wait behind one
// link, so the card below is not pushed off the pane.
const FIRST = 20

// The last 7 days of the log, newest first: what was used, by whom, where,
// and how it went. The row is the item when the lockbox still has it, else
// the place the use was headed for — a denied fill names the site even when
// no item was ever picked.
export default function Events({ events }: { events: AgentEvent[] }) {
  const { t } = useTranslation()
  const { dateTime } = useDates()
  const [all, setAll] = useState(false)
  const shown = all ? events : events.slice(0, FIRST)

  const who = (event: AgentEvent) => {
    const kind = actorWords(t, event.actorKind)
    return event.actorName ? `${event.actorName} (${kind})` : kind
  }

  return (
    <SettingsGroup label={t('Agent activity')}>
      {events.length === 0 ? (
        <Note testid="settings-agent-activity-empty" className="px-4 py-3.5">
          {t('No agent activity in the last 7 days')}
        </Note>
      ) : (
        <>
          {shown.map(event => (
            <SettingsRow
              key={event.id}
              testid="settings-agent-event"
              label={event.itemTitle ?? event.target ?? t('Untitled')}
              description={[
                dateTime(new Date(event.tsMs).toISOString()),
                who(event),
                event.itemTitle ? event.target : null,
                event.mode && modeWords(t, event.mode)
              ]
                .filter(Boolean)
                .join(' · ')}
              control={
                <Chip
                  tone={event.event === 'used' ? 'good' : 'bad'}
                  testid="settings-agent-event-outcome"
                >
                  {outcomeWords(t, event)}
                </Chip>
              }
            />
          ))}
          {!all && events.length > FIRST && (
            <div className="px-4 py-3.5">
              <button
                type="button"
                data-testid="settings-agent-events-all"
                onClick={() => setAll(true)}
                className={`text-base ${TEXT_LINK}`}
              >
                {t('Show all {{total}}', { total: events.length })}
              </button>
            </div>
          )}
        </>
      )}
    </SettingsGroup>
  )
}
