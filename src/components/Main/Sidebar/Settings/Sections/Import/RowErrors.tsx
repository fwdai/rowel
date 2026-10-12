import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import type { RowError } from '@/api/imports'

const MAX_SHOWN = 20

interface Props {
  rows: RowError[]
  // An error's row was not read; a warning's row was, with a note about what
  // it left behind. The heading and its colour say which.
  kind: 'error' | 'warning'
}

export default function RowErrors({ rows, kind }: Props) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  if (rows.length === 0) return null

  return (
    <div className="text-base" data-testid={`import-row-${kind}s`}>
      <span
        className={`cursor-pointer select-none ${kind === 'error' ? 'text-bad' : 'text-text2'}`}
        onClick={() => setOpen(!open)}
      >
        {kind === 'error'
          ? t('{{count}} rows could not be read', { count: rows.length })
          : t('{{count}} rows imported with a note', { count: rows.length })}{' '}
        {open ? '▾' : '▸'}
      </span>
      {open && (
        <ul className="mt-1.5 max-h-[140px] list-disc overflow-y-auto pl-5 text-text3">
          {rows.slice(0, MAX_SHOWN).map((e, i) => (
            <li key={i}>
              {t('Row {{row}}', { row: e.row })}: {e.message}
            </li>
          ))}
          {rows.length > MAX_SHOWN && (
            <li>
              … {t('{{count}} more', { count: rows.length - MAX_SHOWN })}
            </li>
          )}
        </ul>
      )}
    </div>
  )
}
