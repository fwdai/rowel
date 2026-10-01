import { useMemo, useState } from 'react'
import { useTranslation } from 'react-i18next'
import Panel from '@/components/elements/Panel'
import Segmented from '@/components/elements/Segmented'
import IconButton from '@/components/elements/IconButton'
import { CustomFieldsField, NoteField, useField } from '@/components/elements/fields'
import { META } from '@/components/elements/tokens'
import { EyeGlyph, EyeOffGlyph } from '@/components/Main/icons'
import { bandsOf, parseEnv, varsOf } from '../parse'
import DropTarget from './DropTarget'
import FileTab from './FileTab'
import Table from './Table'

type Tab = 'variables' | 'file'

// The file is the entry's one secret; the table is a view over it. Two faces
// share the same draft key, so switching tabs never has anything to sync — the
// File tab shows what the rows were parsed from, and edits on either side land
// in the same string.
export default function Fields() {
  const { t } = useTranslation()
  const { value: body, editing } = useField('body')
  const { value: note } = useField('note')
  const [tab, setTab] = useState<Tab>('variables')
  const [showAll, setShowAll] = useState(false)
  // Parsed once per body, here, for the count and the table both — not once
  // per reveal, filter keystroke or pending-row keystroke, none of which
  // change the file.
  const { vars, bands } = useMemo(() => {
    const lines = parseEnv(body)
    return { vars: varsOf(lines), bands: bandsOf(lines) }
  }, [body])
  const count = vars.length
  const variables = tab === 'variables'

  // Leaving the Variables face takes its eye off screen, so a reveal-all left
  // on would still be on when the face came back with nothing to say so. The
  // File tab masks itself afresh on each visit; this face does the same.
  const switchTo = (next: Tab) => {
    setShowAll(false)
    setTab(next)
  }

  return (
    <>
      {/* Compact keeps the two segments side by side; the count and the eye
          drop under them. */}
      <div className="mb-3 flex items-center justify-between gap-3 @max-[420px]:flex-wrap">
        <Segmented
          options={[
            { value: 'variables', label: t('Variables') },
            { value: 'file', label: t('File') }
          ]}
          value={tab}
          onChange={switchTo}
          testidPrefix="env-tab"
        />
        <div className="flex items-center gap-2 @max-[420px]:w-full @max-[420px]:justify-end">
          <span className={META}>{t('{{count}} variables', { count })}</span>
          {/* One eye for the whole panel, mirroring the card face. Editing
              starts revealed, so it has nothing to do there. */}
          {variables && !editing && (
            <IconButton
              title={showAll ? t('Hide') : t('Reveal')}
              active={showAll}
              testid="env-reveal-all"
              onClick={() => setShowAll(!showAll)}
            >
              {showAll ? <EyeOffGlyph /> : <EyeGlyph />}
            </IconButton>
          )}
        </div>
      </div>

      {/* Read and edit are different subtrees (see Show), so the table — and
          with it which rows are revealed — is mounted fresh on every mode
          switch, as every other kind's field set is. */}
      {variables ? <Table vars={vars} bands={bands} revealAll={showAll} /> : <FileTab />}

      {/* Renders nothing outside edit mode; it writes the same `body`. */}
      <DropTarget />

      {/* Reading, an empty note is no panel at all — like the card's aside. */}
      {(editing || note !== '') && (
        <Panel className="mt-3">
          <NoteField label="Note" />
        </Panel>
      )}

      {/* What the file has no line for — the account it belongs to, where it
          is deployed — as on every other kind. */}
      <CustomFieldsField />
    </>
  )
}
