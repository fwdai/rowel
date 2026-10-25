import { useTranslation } from 'react-i18next'
import type { WorkspaceColor } from '@/lib/workspaceColor'
import { cx } from '@/utils/cx'
import Field from '@/components/elements/Field'
import { CARD } from '@/components/elements/tokens'
import Preview from './Preview'
import NameField from './NameField'
import ColorSwatches from './ColorSwatches'
import AgentsToggle from './AgentsToggle'
import { nameTaken } from './nameTaken'

interface Props {
  name: string
  onName: (name: string) => void
  color: WorkspaceColor | null
  onColor: (color: WorkspaceColor) => void
  /** Every other workspace's label: a name can't be one of them. */
  taken: string[]
  preview: { seed: string; meta: string }
  /**
   * The agent-access switch, for a lockbox that exists: a new one starts off
   * and is switched from its edit page, so the create form leaves this out.
   */
  agents?: { value: boolean; onChange: (next: boolean) => void }
  disabled?: boolean
  testidPrefix: string
}

// What a workspace is called and how its tile looks, shared by the New and
// Edit sub-pages. The fields are the caller's: each decides what counts as
// ready to submit, and what the submit does.
export default function WorkspaceForm({
  name,
  onName,
  color,
  onColor,
  taken,
  preview,
  agents,
  disabled,
  testidPrefix
}: Props) {
  const { t } = useTranslation()

  return (
    <div className="flex flex-col gap-3">
      <Preview
        name={name}
        color={color}
        seed={preview.seed}
        meta={preview.meta}
        testid={`${testidPrefix}-preview`}
      />
      <div className={cx(CARD, 'flex flex-col gap-4 p-5')}>
        <NameField
          value={name}
          onChange={onName}
          duplicate={nameTaken(name, taken)}
          disabled={disabled}
          testidPrefix={testidPrefix}
        />
        <Field label={t('Color')}>
          <ColorSwatches
            value={color}
            onChange={onColor}
            label={t('Color')}
            disabled={disabled}
            testidPrefix={testidPrefix}
          />
        </Field>
        {agents && (
          <AgentsToggle
            value={agents.value}
            onChange={agents.onChange}
            disabled={disabled}
            testidPrefix={testidPrefix}
          />
        )}
      </div>
    </div>
  )
}
