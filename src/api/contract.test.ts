import { describe, expect, it } from 'vitest'
import * as ts from 'typescript'
import libRs from '../../src-tauri/src/lib.rs?raw'
import eventsRs from '../../src-tauri/src/events.rs?raw'
import errorRs from '../../src-tauri/crates/core/src/error.rs?raw'
import modelsRs from '../../src-tauri/crates/core/src/models.rs?raw'
import errorsTs from './errors.ts?raw'
import { EVENTS } from './events'
import { KINDS, completeEntry } from '@/kinds'
import type { Entry, LoginEntry } from './types'

// The bridge is written by hand on both sides. This is what keeps the two from
// drifting: every command the webview invokes must be one Rust registers, every
// event it listens for must be one Rust can emit, and every kind a rejection
// can carry must be one the webview knows how to say — and the other way round,
// so nothing is left registered, emitted or translated that no one uses.
//
// "Invokes" means *reached*, not merely *wrapped*. A wrapper in `@/api` that
// nothing under `src/` calls is exactly as dead as the command behind it, and a
// dead command is a liability rather than a loose end: `auth::setup` sat
// registered for a release, skipping the guards `setup_create` applies, and
// nothing here failed. So "reached" is read off the syntax tree, not the text:
// a file imports the wrapper as a value and uses that binding where a value
// goes — calls it, or hands it on as a callback. A re-export, a `typeof`, a
// type position or a mention in a comment or a string would otherwise keep a
// dead wrapper, and its command, alive.

// Every source under `src/`, production only. Test files and the test harness
// count for nothing: a wrapper kept alive by its own test is still dead.
const sources = (
  Object.entries(
    import.meta.glob('../**/*.{ts,tsx}', { query: '?raw', import: 'default', eager: true })
  ) as [string, string][]
).filter(([path]) => !/\.test\.tsx?$/.test(path) && !path.startsWith('../test/'))

// Vite normalises a glob key against the importing module, so the wrappers —
// this file's own directory, `@/api` — come back as `./<name>.ts`.
const isApi = (path: string) => path.startsWith('./')

// A set: one command invoked from two places is still one command.
const unique = (names: Iterable<string>) => [...new Set(names)].sort()

const names = (source: string, pattern: RegExp) =>
  unique([...source.matchAll(pattern)].map(m => m[1]))

// One module as a tree. Parent links on, so a node can be placed in its scope.
const parse = (path: string, source: string) =>
  ts.createSourceFile(
    path,
    source,
    ts.ScriptTarget.Latest,
    true,
    path.endsWith('x') ? ts.ScriptKind.TSX : ts.ScriptKind.TS
  )

const walk = (root: ts.Node, visit: (node: ts.Node) => void) => {
  visit(root)
  ts.forEachChild(root, child => walk(child, visit))
}

// The command a `<callee>('…')` names, when `node` is one; generics
// (`invoke<Settings>('…')`) sit beside the callee, not in it.
const commandNamed = (node: ts.CallExpression, callee: string): string | null => {
  const [first] = node.arguments
  return ts.isIdentifier(node.expression) &&
    node.expression.text === callee &&
    first !== undefined &&
    ts.isStringLiteral(first)
    ? first.text
    : null
}

// An identifier standing where a value goes. Not the name in an import or
// export specifier, not a `typeof`, not a type, not the `.name` half of a
// property access — those name the binding without reaching it.
const isValueUse = (node: ts.Identifier) => {
  const parent = node.parent
  if (ts.isImportSpecifier(parent) || ts.isExportSpecifier(parent)) return false
  if (ts.isTypeQueryNode(parent) || ts.isTypeReferenceNode(parent)) return false
  if (ts.isPropertyAccessExpression(parent) && parent.name === node) return false
  // `{ appStatus: 1 }` names a key; the shorthand `{ appStatus }` is the value.
  if (ts.isPropertyAssignment(parent) && parent.name === node) return false
  if (ts.isQualifiedName(parent)) return false
  return true
}

/**
 * What one module does with the bridge: the names it imports as values, the
 * imported names it then uses as values, and the commands it hands to `invoke`
 * itself.
 */
interface Usage {
  imported: Set<string>
  used: Set<string>
  direct: Set<string>
}

export const usage = (path: string, source: string): Usage => {
  const imported = new Set<string>()
  const used = new Set<string>()
  const direct = new Set<string>()
  const tree = parse(path, source)
  walk(tree, node => {
    if (ts.isImportDeclaration(node) && !node.importClause?.isTypeOnly) {
      const bindings = node.importClause?.namedBindings
      if (bindings && ts.isNamedImports(bindings)) {
        for (const element of bindings.elements) {
          if (!element.isTypeOnly) imported.add(element.name.text)
        }
      }
    }
  })
  walk(tree, node => {
    if (ts.isIdentifier(node) && imported.has(node.text) && isValueUse(node)) used.add(node.text)
    if (ts.isCallExpression(node)) {
      const command = commandNamed(node, 'invoke')
      if (command) direct.add(command)
    }
  })
  return { imported, used, direct }
}

/**
 * The command each `@/api` wrapper reaches, and the file that declares it: an
 * exported `const` whose initializer contains `call('<command>')`. Read per
 * declaration off the tree, so one with no call of its own cannot borrow the
 * next one's.
 */
const wrappers = () => {
  const found = new Map<string, { command: string; declaredIn: string }>()
  for (const [declaredIn, source] of sources.filter(([path]) => isApi(path))) {
    walk(parse(declaredIn, source), node => {
      if (!ts.isVariableStatement(node)) return
      if (!node.modifiers?.some(m => m.kind === ts.SyntaxKind.ExportKeyword)) return
      for (const declaration of node.declarationList.declarations) {
        if (!ts.isIdentifier(declaration.name) || !declaration.initializer) continue
        let command: string | null = null
        walk(declaration.initializer, inner => {
          if (ts.isCallExpression(inner)) command ??= commandNamed(inner, 'call')
        })
        if (command) found.set(declaration.name.text, { command, declaredIn })
      }
    })
  }
  return found
}

const invoked = () => {
  const usages = sources.map(([path, source]) => [path, usage(path, source)] as const)
  const reached = [...wrappers()]
    .filter(([wrapper, { declaredIn }]) =>
      usages.some(([path, { used }]) => path !== declaredIn && used.has(wrapper))
    )
    .map(([, { command }]) => command)
  // Commands handed straight to `invoke`, outside the wrappers: the dev-only
  // E2E bridge is the only one.
  const direct = usages.filter(([path]) => !isApi(path)).flatMap(([, { direct }]) => [...direct])
  return unique([...reached, ...direct])
}

const registered = () => {
  // Up to the `])` that closes the macro: a `#[cfg(...)]` inside it has a `]` too.
  const list = libRs.match(/generate_handler!\[([\s\S]*?)\]\)/)?.[1] ?? ''
  return names(list, /::([a-z_0-9]+),?\s*$/gm)
}

const emitted = () => names(eventsRs, /"([a-z]+(?::[a-z]+)+)"/g)

// The arms of `Error::kind()` — `Error::Locked => "locked",` — read out of the
// function body so an `#[error("…")]` attribute elsewhere in the file cannot be
// mistaken for one.
const kinds = () => {
  const body = errorRs.match(/fn kind\(&self\)[\s\S]*?\n {4}\}/)?.[0] ?? ''
  return names(body, /=>\s*"([A-Za-z]+)"/g)
}

// The `BackendErrorKind` union, up to the blank line that ends it.
const declared = () =>
  names(errorsTs.match(/BackendErrorKind =([\s\S]*?)\n\n/)?.[1] ?? '', /'(\w+)'/g)

// The arms of `describeError`.
const translated = () => names(errorsTs, /case '(\w+)':/g)

// The kinds that deliberately show Rust's message instead of a translation:
// a path, a parser position, a driver's words, or a sentence Rust built for one
// call site. `errors.ts` says why for each; everything else has to be copy the
// user can read in their own language.
const RAW = ['unsupported', 'io', 'serde', 'crypto', 'other']

// The wire name of every text field Rust's `Entry` may leave off: an
// `Option<String>` under `skip_serializing_if`, sent under its `rename` when it
// has one. Read off the struct body, so a field added to Rust shows up here
// without anyone remembering to list it.
const omittableStrings = () => {
  const body = modelsRs.match(/pub struct Entry \{([\s\S]*?)\n\}/)?.[1] ?? ''
  const fields = body.matchAll(
    /#\[serde\(([^\]]*?)\)\]\s*pub (\w+): Option<String>/g
  )
  return unique(
    [...fields]
      .filter(([, attrs]) => attrs.includes('skip_serializing_if'))
      .map(([, attrs, name]) => attrs.match(/rename = "(\w+)"/)?.[1] ?? name)
  )
}

// Optional on the webview side as well, so nothing has to fill them in.
const OPTIONAL_IN_TS = ['createdAt', 'updatedAt', 'password_updated_at']

describe('the Rust/webview contract', () => {
  // The TS entry types promise a string for every field of a kind, while Rust
  // sends `Option<String>` and drops the `None`s — so `completeEntry` fills the
  // gaps from the kind's empty draft. This is what keeps the two shapes in step:
  // a text field Rust can omit has to have a default in some kind, or it arrives
  // as `undefined` where a string was typed.
  it('has a default for every text field Rust may leave off an entry', () => {
    const fields = omittableStrings()
    expect(fields.length).toBeGreaterThan(20)
    const defaulted = new Set(KINDS.flatMap(kind => Object.keys(kind.defaults)))
    const orphans = fields.filter(f => !OPTIONAL_IN_TS.includes(f) && !defaulted.has(f))
    expect(orphans).toEqual([])
  })

  it('completes a bare entry the way the kind types describe it', () => {
    const bare = { id: 'l1', type: 'login', title: 'Site', password: 'pw' } as Entry
    const login = completeEntry(bare) as LoginEntry
    expect(login.password).toBe('pw')
    expect(login.username).toBe('')
    expect(login.email).toBe('')
    expect(login.otp).toBe('')
    expect(login.website).toBe('')
    expect(login.note).toBe('')
  })

  it('invokes exactly the commands Rust registers', () => {
    expect(invoked()).toEqual(registered())
  })

  it('listens for exactly the events Rust emits', () => {
    expect(Object.values(EVENTS).sort()).toEqual(emitted())
  })

  it('knows exactly the error kinds Rust can return', () => {
    expect(declared()).toEqual(kinds())
  })

  it('translates every kind that is not raw diagnostics', () => {
    expect(translated()).toEqual(kinds().filter(kind => !RAW.includes(kind)))
  })
})

// What the tripwire above accepts as "reached", pinned so it cannot quietly
// widen back into text matching.
describe('what counts as reaching a wrapper', () => {
  const probe = (body: string) =>
    usage('probe.ts', `import { appStatus } from '@/api/app'\n${body}\n`).used.has('appStatus')

  it('using the imported name as a value: a call, or handed on as one', () => {
    for (const body of [
      'void appStatus()',
      'void refresh().then(appStatus)',
      'const probe = { appStatus }',
      'const run = useLatestRequest(appStatus)'
    ]) {
      expect(probe(body), body).toBe(true)
    }
  })

  it('not a mention that never reaches it', () => {
    for (const body of [
      '// appStatus() would go here',
      "const label = 'appStatus()'",
      'type Probe = typeof appStatus',
      'let probe: appStatus',
      'export { appStatus }',
      'const other = { appStatus: 1 }; void other.appStatus'
    ]) {
      expect(probe(body), body).toBe(false)
    }
  })

  it('a type-only import is not an import of the wrapper', () => {
    const { imported, used } = usage(
      'probe.ts',
      "import type { appStatus } from '@/api/app'\nlet probe: typeof appStatus\n"
    )
    expect(imported.has('appStatus')).toBe(false)
    expect(used.has('appStatus')).toBe(false)
  })

  it('a bare invoke names its command', () => {
    const { direct } = usage(
      'e2e.ts',
      "import { invoke } from '@tauri-apps/api/core'\nvoid invoke<void>('e2e_reset', { mode })\n"
    )
    expect([...direct]).toEqual(['e2e_reset'])
  })
})
