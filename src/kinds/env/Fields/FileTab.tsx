import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { cx } from '@/utils/cx'
import { isMobile } from '@/lib/platform'
import { saveEnvFile } from '@/api/vault'
import { describeError } from '@/api/errors'
import IconButton from '@/components/elements/IconButton'
import Panel from '@/components/elements/Panel'
import SealedBlock from '@/components/elements/SealedBlock'
import { grow, useField } from '@/components/elements/fields'
import { requiredError } from '@/components/elements/fields/formats'
import { verbatimInput } from '@/components/elements/inputProps'
import { DownloadGlyph } from '@/components/Main/icons'
import { BOX, BOX_LINE } from './styles'

// Long files scroll inside the well rather than pushing the metadata line off
// the screen; the same cap holds the editor, which grows to it and then scrolls.
const WELL = 'max-h-[60vh] overflow-y-auto'

// The raw text. Reading, it is the shared sealed block a private key or secure
// note uses, taller for a file. Editing, a plain textarea over the same string
// — power users fix quoting or comments here, and the table is never out of
// date because there is only one source.
//
// "Save as file…" sits in this rail rather than the header's ⋯ menu: it is the
// file tab's own action, and keeping it kind-local means the shared header
// needs no per-kind hook. Desktop only — the mobile picker copies the file
// itself and cannot be told what permissions to give a plaintext secret.
export default function FileTab() {
  const { t } = useTranslation()
  const { value, set, editing, attempted } = useField('body')
  const { value: fileName } = useField('fileName')
  const [saveError, setSaveError] = useState<string | null>(null)
  const validationError = requiredError(value, true, attempted)

  // A dismissal is not an error. A failed commit stays beside the action that
  // initiated it, so the user never has to infer success from a closed dialog.
  const save = () => {
    setSaveError(null)
    saveEnvFile(fileName, value).catch((e: unknown) => setSaveError(describeError(e)))
  }

  if (editing)
    return (
      <Panel>
        <div className="px-3.5 py-3">
          <textarea
            name="body"
            aria-label={t('File')}
            value={value}
            rows={1}
            placeholder="KEY=value"
            {...verbatimInput}
            ref={grow}
            onChange={event => {
              grow(event.currentTarget)
              set(event.target.value)
            }}
            className={cx(
              'block min-h-6 w-full resize-none font-mono text-base leading-relaxed text-text',
              WELL,
              BOX,
              validationError ? 'border-bad' : BOX_LINE
            )}
          />
          {validationError && <div className="mt-1.5 text-base text-bad">{validationError}</div>}
        </div>
      </Panel>
    )

  return (
    <>
      <SealedBlock
        label="File"
        name="body"
        value={value}
        meta={fileName || undefined}
        mono
        lines={[value]}
        bodyClassName={WELL}
        actions={
          !isMobile && (
            <IconButton title={t('Save as file…')} testid="env-save-file" onClick={save}>
              <DownloadGlyph />
            </IconButton>
          )
        }
      />
      {saveError && (
        <div data-testid="env-save-error" className="mt-2 px-3.5 text-base text-bad">
          {saveError}
        </div>
      )}
    </>
  )
}
