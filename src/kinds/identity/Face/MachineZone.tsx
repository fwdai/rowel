import type { Doc } from './DocCell'
import { mrz } from './mrz'

/*
 * What a machine reads off a passport's data page: the MRZ, generated from the
 * fields themselves — check digits and all, see `mrz.ts` — in a strip along the
 * bottom edge, running the full width of the paper the way it is printed. The
 * one thing on the face set in the mono tier, because it is the one thing on it
 * that is machine text rather than something a person reads.
 */
export default function MachineZone({ doc }: { doc: Doc }) {
  return (
    <div className="overflow-hidden border-t border-(--face-rule) bg-(--face-tail) px-5 py-2 font-mono text-2xs leading-[1.6] tracking-[0.1em]">
      {mrz(doc).map((line, row) => (
        <div key={row} className="whitespace-pre">
          {line}
        </div>
      ))}
    </div>
  )
}
