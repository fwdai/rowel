// Puts the browser extension's tree back under Rowel's paths after a
// `git subtree pull` of upstream KeePassXC-Browser (docs/browser-extension.md,
// Syncing with upstream). The fork keeps upstream's files under its own names
// (extension/rowel/ for keepassxc-browser/, icons/rowel*.png for
// keepassxc*.png, ...); git's rename detection carries upstream's changes over
// on the merge, and this script catches whatever still landed under an old
// name. The renames are extension/upstream-map.json:
//   - `paths`: whatever exists at the old path is `git mv`ed to the new one, in
//     order (a directory whole, merged into the new one if that exists; a `*`
//     glob file by file, with the match carried into the new name);
//   - `text`: the old strings are rewritten in the files `textFiles` match,
//     minus `textExclude` (upstream's third-party bundles and the locales).
// Idempotent: a second run has nothing to do.
//
// `--check` changes nothing and exits 1 listing any old path that exists or
// file that still names one (CI).
import { execFileSync } from 'node:child_process'
import { existsSync, mkdirSync, readdirSync, readFileSync, rmdirSync, statSync, writeFileSync } from 'node:fs'
import { dirname, join, posix, relative } from 'node:path'

const ROOT = 'extension'
const MAP = join(ROOT, 'upstream-map.json')
const SKIP_DIRS = new Set(['node_modules', 'build', 'test-results', 'playwright-report', '.git'])

const map = JSON.parse(readFileSync(MAP, 'utf8'))
const check = process.argv.includes('--check')
const at = p => join(ROOT, p)
const isDir = p => existsSync(p) && statSync(p).isDirectory()
const escape = s => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')

// Minimal glob: `**` spans directories, `*` and `?` stay within one name,
// `{a,b}` alternates. Matched against extension/-relative POSIX paths.
function glob(pattern) {
  let source = ''
  for (let i = 0; i < pattern.length; i++) {
    const c = pattern[i]
    if (c === '*' && pattern[i + 1] === '*') {
      source += pattern[i + 2] === '/' ? '(?:.*/)?' : '.*'
      i += pattern[i + 2] === '/' ? 2 : 1
    } else if (c === '*') source += '[^/]*'
    else if (c === '?') source += '[^/]'
    else if (c === '{') {
      const end = pattern.indexOf('}', i)
      source += `(?:${pattern.slice(i + 1, end).split(',').map(escape).join('|')})`
      i = end
    } else source += escape(c)
  }
  return new RegExp(`^${source}$`)
}

function* walk(dir) {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    if (SKIP_DIRS.has(entry.name)) continue
    const file = join(dir, entry.name)
    if (entry.isDirectory()) yield* walk(file)
    else yield file
  }
}

const todo = []
const gitMv = (from, to) => {
  todo.push(`${relative(ROOT, from)} -> ${relative(ROOT, to)}`)
  if (check) return
  mkdirSync(dirname(to), { recursive: true })
  execFileSync('git', ['mv', from, to], { stdio: 'inherit' })
}

// A directory moves whole unless the destination exists, in which case its
// contents are merged in (an upstream file git did not follow the rename for).
function move(from, to) {
  if (!existsSync(from)) return
  if (!existsSync(to)) return gitMv(from, to)
  if (!isDir(from) || !isDir(to)) {
    throw new Error(`both ${from} and ${to} exist; resolve by hand`)
  }
  for (const entry of readdirSync(from)) move(join(from, entry), join(to, entry))
  if (!check && readdirSync(from).length === 0) rmdirSync(from)
}

// `rowel/icons/keepassxc_*.png` -> `rowel/icons/rowel_*.png`, per match.
function moveGlob(from, to) {
  const dir = at(dirname(from))
  if (!isDir(dir)) return
  const pattern = new RegExp(`^${escape(posix.basename(from)).replace(/\\\*/g, '(.*)')}$`)
  for (const name of readdirSync(dir)) {
    const m = name.match(pattern)
    if (!m) continue
    let i = 1
    const target = posix.basename(to).replace(/\*/g, () => m[i++])
    move(join(dir, name), join(at(dirname(to)), target))
  }
}

for (const { from, to } of map.paths) {
  if (from.includes('*')) moveGlob(from, to)
  else move(at(from.replace(/\/$/, '')), at(to.replace(/\/$/, '')))
}

const include = map.textFiles.map(glob)
const exclude = map.textExclude.map(glob)
const rules = map.text.map(({ from, to, regex }) => ({
  from: new RegExp(regex ? from : escape(from), 'g'),
  to
}))
const stale = []
for (const file of walk(ROOT)) {
  const rel = relative(ROOT, file).split('\\').join('/')
  if (!include.some(g => g.test(rel)) || exclude.some(g => g.test(rel))) continue
  const source = readFileSync(file, 'utf8')
  const next = rules.reduce((text, rule) => text.replace(rule.from, rule.to), source)
  if (next === source) continue
  stale.push(rel)
  if (!check) writeFileSync(file, next)
}

if (check && (todo.length || stale.length)) {
  const lines = [...todo.map(t => `old path ${t}`), ...stale.map(f => `old name in ${f}`)]
  console.error(`Extension paths do not follow ${MAP} (run \`bun run extension:sync\`):\n  ${lines.join('\n  ')}`)
  process.exit(1)
}
console.log(
  check
    ? `Extension paths follow upstream-map.json.`
    : `Moved ${todo.length} path(s), rewrote ${stale.length} file(s).`
)
