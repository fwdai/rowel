import { useState, type ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import Button from '@/components/elements/Button'
import SettingsRow from '@/components/elements/SettingsRow'

interface Props {
  label: ReactNode
  description?: string
  // The row's 16px mark, drawn in SettingsRow's tile.
  icon?: ReactNode
  /** What the button offers while the row is folded ("Change…", "Show"). */
  action: string
  /** What it offers once unfolded. "Cancel" for a form; a list says "Hide". */
  collapse?: string
  /** On the row; the button that unfolds it takes `<testid>-toggle`. */
  testid?: string
  children: ReactNode
}

// A settings row whose control unfolds something beneath it — a short form, a
// list — so the card stays a list of one-line rows until something is asked
// for. What unfolds slides down under the row rather than appearing.
export default function ExpandableRow({
  label,
  description,
  icon,
  action,
  collapse,
  testid,
  children
}: Props) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)

  return (
    <SettingsRow
      label={label}
      description={description}
      icon={icon}
      testid={testid}
      control={
        <Button
          variant="pale"
          size="md"
          testid={testid && `${testid}-toggle`}
          expanded={open}
          onClick={() => setOpen(!open)}
        >
          {open ? (collapse ?? t('Cancel')) : action}
        </Button>
      }
    >
      {open && (
        <div className="grid animate-unfold">
          <div className="min-h-0 overflow-hidden">{children}</div>
        </div>
      )}
    </SettingsRow>
  )
}
