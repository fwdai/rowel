import type { ReactNode } from 'react'
import { DropdownItem } from '@/components/elements/Dropdown'

interface Props {
  /** At row size (16px), in the tile a settings row draws its mark in. */
  icon: ReactNode
  label: string
  description: string
  onClick: () => void
  testid: string
}

// One way to one more lockbox, as an item of the Add row's menu: its name, and
// under it the line that tells it from the other two.
export default function Way({ icon, label, description, onClick, testid }: Props) {
  return (
    <DropdownItem testid={testid} onClick={onClick} className="py-2">
      <span className="grid h-8 w-8 flex-none place-items-center rounded-sm bg-tile text-text2">
        {icon}
      </span>
      <span className="min-w-0 flex-1">
        <span className="block font-medium">{label}</span>
        <span className="mt-0.5 block text-sm text-text2">{description}</span>
      </span>
    </DropdownItem>
  )
}
