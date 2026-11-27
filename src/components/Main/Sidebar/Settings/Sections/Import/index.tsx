import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import Segmented from '@/components/elements/Segmented'
import { useImport } from './useImport'
import ImportPane from './ImportPane'
import ExportPane from './Export'

type Pane = 'import' | 'export'

export default function Import() {
  const { t } = useTranslation()
  // Held here rather than in the import pane, so a file the OS opened with the
  // app, or another app's data, is claimed (and a picked one kept) whichever
  // pane is showing.
  const flow = useImport()
  const [pane, setPane] = useState<Pane>('import')
  // The pane slides in on a switch, not on arriving at the section.
  const [switched, setSwitched] = useState(false)

  const show = (next: Pane) => {
    setPane(next)
    setSwitched(true)
  }

  // A pick made while Export is showing can only be a file the OS opened with
  // the app or another app's data: bring the pane that shows it forward.
  const [seen, setSeen] = useState(flow.picked)
  if (flow.picked !== seen) {
    setSeen(flow.picked)
    if (flow.picked && pane !== 'import') show('import')
  }

  return (
    <div className="flex flex-col gap-4">
      <Segmented
        name={t('Import or export')}
        testidPrefix="settings-io"
        options={[
          { value: 'import', label: t('Import') },
          { value: 'export', label: t('Export') }
        ]}
        value={pane}
        onChange={show}
        className="self-start"
      />
      <div key={pane} className={switched ? 'animate-sheet' : undefined}>
        {pane === 'import' ? <ImportPane flow={flow} /> : <ExportPane />}
      </div>
    </div>
  )
}
