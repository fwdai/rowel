# Compact shell — the phone UI

How Rowel's iOS layout is built, and the rules every change to it follows. The
design reference is the "Keyring Mobile Prototype" (an iPhone mock: large-title
tab roots, a floating tab bar, a detail screen pushed from the right with a
bottom primary action, a bottom-sheet kind picker, a slide-up form, a
biometric-first lock screen). The prototype is a guideline for *layout and
motion only*. Brand, labels, i18n strings, kinds and features come from the
desktop app, which stays the source of truth.

## Status

| Slice | Scope | State |
|---|---|---|
| 1 | Foundations: `Frame` context, layout branches lifted out of leaves, glass token, rise keyframe, touch-visible copy buttons | ✅ |
| 2 | Compact shell as screens: derived screen selection, floating tab bar, large-title list root, top bar removed | ✅ |
| 3 | Detail read screen: nav row, kind header, container-query row geometry, tap-to-copy rows, bottom primary action | ✅ |
| 4 | Form screen: `Edit` split into `Title` + `Body`, slide-up form with Save/Cancel in the nav row, add picker as a bottom sheet | ✅ |
| 5 | Generator and Settings as tab roots; Archive reachable from Settings | ✅ |
| 6 | Lock screen: `useUnlock` hook, biometric-first compact layout, platform-correct biometric label | ✅ |

## The three rules

Platform differences are handled by exactly one of these, chosen by what kind
of difference it is. Nothing else branches on layout.

1. **Screen structure is composition, decided once in the shell.**
   `useLayout()` is called once per flow root and nowhere below it: `App.tsx`
   picks `LockScreen` or `Auth` for the `auth` flow, `Main/index.tsx` picks
   `Compact` or `Wide` for the vault. Each pair assembles its screens from
   shared content components (the lock screens from `Auth/useUnlock`). Leaf
   components never ask which shell they are in. Where a leaf must be framed
   differently (a dialog card on desktop, a sheet on a phone) it renders the
   `Frame` from `elements/Frame`, and the shell provides the implementation
   through context.
2. **Row and element geometry adapts with CSS container queries, not JS.**
   `FieldRow`, `Field`, the detail header and `Footer` stack label-over-value
   when their container is narrow. Tailwind v4's `@container` / `@max-*`
   variants do this with zero render branches, and every kind inherits it.
   The threshold is 420px and the container is whichever surface declares
   itself one — `Show/Read` and `Show/Edit` on the desktop, the detail
   scroller on the phone.
3. **Capability differences use compile-time constants and media features.**
   `isMobile` / `isIOS` from `lib/platform` (no updater, no drag-and-drop);
   what the device reports (the biometry kind, from the backend). Touch tiers
   key off `any-pointer-coarse`, not `pointer-coarse`: a device with a trackpad
   **and** a touchscreen still needs the 44pt targets and the always-visible
   copy button. All of it is constants or CSS, never `useLayout`.

Corollaries: no `compact` boolean props on shared components (a slot such as
`actions` is composition and is fine); no new navigation state when the store
already implies the screen; no new dependency for animation or routing.

A dialog that needs a different frame on a phone says so by describing its own
content, never the platform: `FrameProps.fit` is `'screen'` (the default — a
settings surface, the generator) or `'content'` (a short one, the add picker).
`Modal` always has the room and ignores it; `Sheet` reads it to choose between
a full-screen page and `elements/BottomSheet`. Sizes travel the same way —
`className`, `tile`, `glyph` — so `Show/Edit/Title` draws a 28px tile in the
pane and a 44px one on the phone without being told which it is. A dialog names
itself through `labelledBy` and carries its own heading; the frames add no title
bar of their own.

## Navigation model

There is no router. The compact shell derives its screen from state the store
already has:

| State | Screen |
|---|---|
| `entries.new !== null` or `entries.edit` | Form (slide-up) |
| `ui.settings` | Settings tab root (a section pushes its pane) |
| `generator.open` with no `apply`/`ssh` callback | Generator tab root |
| `entries.current !== null` | Detail (pushed from the right) |
| otherwise | List root for `ui.view` |

The order is what makes an *open* land. Both roots can be opened while a row is
selected (⌘G, ⌘, , the Settings row of a menu), so they come before the
selection — behind it, `generator.open` would go true and nothing visible would
happen. A draft still outranks all of it: it is the one screen with unsaved
work on it. Tabs need no rule, because `setView` clears the selection anyway.

The one exception is which settings pane is open: that is `useState` inside
`Compact/Settings`, because `ui.settingsSection` is the *wide* modal's nav
selection and persists, so a phone reading it would open Settings already
inside a pane. It is one level deep and resets with the screen.

Every row is its own screen — one screen, `Compact/Entry`, with two faces:
`Detail/Read` (nav row, kind header, bottom primary action) and `Form/Editor`
(Cancel/Save in the nav row over one `@container` scroller). One component for
both, the way the desktop's `Aside/Show` is one pane for both, because the
decrypt has to be shared. As of slice 5 there are also the two remaining roots.
The generator splits by *how it was
opened*, not by shell: standalone it is `Compact/Generator` (large title, mode
switch, `Generator/Panel`, a bottom Use & copy), and opened from a password row
it is `Generator/Attached` — the same `Dialog` the wide shell mounts, framed as
a page sheet. Both take `useGeneratorDialog`, so the two shells generate the
same way. Settings is a root list of rows over the desktop's own `Section`
panes, and carries the lock control the vanished top bar used to.

The three pushed headers — the entry's, the form's, a settings pane's — are one
`Compact/NavBar` (leading · title · trailing) with `Compact/BackButton` in the
leading slot. Everything they measure themselves against lives in
`Compact/chrome.ts`, including the 44px `TOUCH` tier the desktop's 28px
controls are dressed in through their `className`.

The list root is the only pane there is, so it also carries what the wide shell
puts in its detail pane: the whole-view empty heroes, and on the audit view the
score panel under the groups (`Body/Aside/Audit`, the same call `Body/Aside`
makes). Both travel as `ListColumn`'s `footer`, which sits in the scroller but
*outside* the `role="listbox"` — a hero's buttons are not options.

Both faces compose parts the desktop's `Aside/Show` also composes —
`Identity`, `Eyebrow`, `Show/Body`, `Edit/Title`, `Edit/Body`, `MoreMenu`, and
the `useDraft` / `usePrimaryAction` / `useShown` / `useDelete` hooks. Neither
shell passes the other a layout flag; each only decides where the parts go.
`useShown` is called once, by the screen rather than by either face, so
stepping into edit decrypts nothing again. It still holds the form's frame
behind `held` exactly as `Show` does — an editor seeded from a reveal that has
not landed would discard whatever is typed first — but that can now only happen
for an edit asked for before the *first* reveal landed, and the held frame
(`Form/Held`) carries a working Cancel so a reveal that never arrives is not a
dead end.

The read screen is laid out as the platform's grouped list, with Passwords and
1Password as the references. The header is one row: a 44px tile, the title,
and the star as a 44px header action, so the row's two ends weigh the same;
under the title only what the kind has to add that no row says (`Eyebrow`
`bare` — a document's type), never the kind's name or the host. Rows take the
grouped list's geometry through `max-md:` in `fields/Row` and `tokens.ts`:
sentence-case 12px captions (`ROW_LABEL`) 2px over the value, 8px over and
under, and the rail pulled into the padding so a glyph sits 16px from the
card's edge like the caption does. There is no copy button on a phone row —
the value is the copy control (`fields/CopyValue`, the one element every
row's value renders through: the typed fields, a custom pair, an env variable,
a key's comment and fingerprint, a scope list), the toast confirms it, and the
bottom button covers the main secret — so a rail holds only the eye and the
open-link. Every row's copy button sits in `ROW_COPY`, which is what hides it
there; the rows that draw their own geometry rather than rendering through
`FieldRow` (a custom pair, an env variable) take the phone grid from the same
exported classes, so one row on the phone is one row whatever it holds. The
one-time code is a row under the password with a countdown ring in its rail
(`fields/Otp/Countdown`, a conic gradient on a registered `--otp-left`), on
both shells; the editor keeps the dial as the live preview. Under the rows,
files, passkeys, custom fields and tags are sections (`SECTION_LABEL`: 12px
uppercase, tracking pulled in, inset to the rows), every "add" is the same
card-row cell (`ACTION_ROW`), and the dates are one centred meta line at the
end — `Compact/Detail/Meta`, handed to `Show/Body` as its `Footer`, which
reads the same facts the desktop's strip does.

Overlays are not screens and do not live in the shell div. That div carries
`viewportStyle`'s translate, which makes it the containing block of anything
`fixed` inside it, so a sheet mounted there would take the keyboard offset
twice. `Generator/Attached`, and every fixed overlay after it, is a sibling of
the shell.

Before any of that there is the lock, which is not a screen of the vault but a
flow of its own: `App` renders `Auth/LockScreen` on compact and `Auth` on wide,
both driven by `Auth/useUnlock` (attempt phase, lockout countdown, mascot gaze,
the eyebrow's text and tone). The desktop leads with the passphrase card and
keeps biometrics as its end segment; the phone leads with an 88px biometric
tile when a key is enrolled and reveals the same card under "Enter Master
Password". Which biometry the copy names is `lib/biometry` — `biometryLabel(type)`
and `biometryGlyph(type)`, from `app_status.biometric.type` (`LAContext.biometryType`
on Apple, the fingerprint everywhere else), read off the app store's `status`
by `App` and handed down as `biometric` / `biometry` — so `Masterpass` says the
same thing wherever it is drawn. The card also survives a late probe: once the
user has typed into it, it stays even if `biometric` flips true underneath
(`src/test/lock.test.tsx`).

Overlays that stay overlays on a phone: the add picker (`fit="content"`, so a
bottom sheet) and the generator opened from a password row (a page sheet). Both
go through `Frame`.

Tabs (labels are the desktop's i18n keys): **All Items · Favorites · Generator
· Settings**. Archive is a row in Settings on compact. A list tab closes
settings and the standalone generator and calls `setView`; Generator and
Settings are one screen slot between them, so each closes the other.

Transitions are mount animations only (`animate-sheet` slides in from the
right, `animate-rise` slides up, `animate-fade` for roots). No exit animations.

A screen pushed from the right can also be popped the iOS way: a drag in from
the left edge takes it with the finger, and letting go past a third of the width
(or a flick) slides it out and runs the same handler its back control does
(`Compact/useSwipeBack`, on the entry screen and both settings panes, under the
same `settingsLocked`). The form is not among them: it rose from the bottom and
its only exit is the guarded Cancel.

## iOS conventions to follow

- Touch targets ≥ 44×44pt. Rows ≥ 44pt tall; list rows in the design are 64pt.
- One tier for every icon action in a header corner — sort, regenerate, the
  sync chip, the entry's menu and star: `HEADER_ACTION` in `chrome.ts`, a 44px
  target over a 20px glyph with no box of its own, the nav bar's proportion on
  both platforms. The glyph is sized by the button, not at each call site, so
  a control shared with the desktop needs no size prop. The one control set
  into the search field (the tag filter) is `FIELD_ACTION`: a 36px tile inset
  4px, with the field's full 44px as its hit area.
- Safe areas: `env(safe-area-inset-top/bottom)` on anything touching an edge.
- A 24px semibold title on tab roots, centred with its 44px actions on a 56px
  row flush against the safe area (`ROOT_HEADER`), no top bar and no app-name
  eyebrow (the home screen icon already said whose vault it is). Pushed screens
  get a nav row on the same 56px footing: a back control that carries the
  previous screen's title, trailing text/icon actions. Root and pushed headers
  share one top edge, so nothing jumps when a screen is pushed or popped.
  The list root keeps its header short — 8px from the title row to the 44px
  search field, and the field's run-out before the first row is the row
  caption's own height — since the list is what the screen is for. Sort has
  the title row's corner, as on the desktop; the tag filter sits in the end
  of the search field, beside the query it narrows the list with.
- Tab bar: floating pill, glass surface (`bg-glass` + `backdrop-blur`), icon
  over a 10px label, resting in the secondary ink (≥ 4.5:1 on the glass, for
  the label's sake). The selected tab is marked by one accent-soft lens that
  slides between tabs with a spring (`ease-spring`), as the system bar's does.
  The centre slot is a notch (a radial `mask` on the pill) with the vault's one
  action, Add, resting in it: a 56px accent disc standing half above the glass
  with a ring of ground around it — a verb placed on the bar, not a fifth tab.
  Content scrolls underneath (the scroller reserves bottom padding; see
  `chrome.ts` for the geometry every piece of bottom chrome is measured from).
- Bottom sheets carry a grabber and dismiss on scrim tap; full-screen forms
  put Cancel on the left and Save on the right of the nav row.
- The primary action of a detail screen is a bottom, thumb-reachable button.
- Copy on tap for value rows; the feedback is the existing copy toast.
- No hover-only UI. No tooltips as the sole label.
- Keyboard: anything full-screen with inputs is sized from
  `useVisualViewport` (`viewportStyle`), like `Sheet` already is.
- Respect `prefers-reduced-motion` (already global in `theme.css`).
- Use system type (`--font-sans` is SF on Apple platforms). Don't add webfonts.

## Theming

Light and dark are both first-class. Only the `--c-*` tokens in
`styles/theme.css` (exposed as `bg-app`, `text-text2`, `border-line`,
`bg-accent-soft`, …) may be used for colour. No hex literals or
`dark:` variants in components: a new surface is a new token defined in both
palettes. Prototype-only colours map to existing tokens (`--list` → `bg-list`,
`--card` → `bg-card`, `--tile` → `bg-tile`, `--field` → `bg-field`,
`--glass` → new `bg-glass`, `--screen` → new `bg-screen`).

`bg-screen` is the phone's one ground: every compact screen — the tab roots,
the pushed entry and form, a page sheet — is painted on it, and cards, the
search field and the bottom sheet sit white (`bg-card`, `bg-detail`) on top.
It is never used by the wide shell, whose panes keep `bg-app` / `bg-list` /
`bg-detail`. A shared component that is a pane on the desktop and the whole
screen on the phone (`ListColumn`, `SearchBox`) switches surface with
Tailwind's `max-md:` variant — the same 768px cut `useLayout` picks the shell
on — never with a prop.

## Testing

- `bun run typecheck`, `bun run lint` (0 warnings), `bun run test` must stay
  green. The e2e suite runs the wide shell and must be untouched by compact work.
- Compact behaviour is covered in `src/test/compact.test.tsx` through
  `setLayout('compact')`; each new screen adds cases there.
- Existing `data-testid`s are preserved. New compact controls get their own.
- The wide shell renders identically after each slice.

## Product decisions taken

- Search stays the inline field that filters the list live (no separate search
  screen). Styled at 44pt.
- A read row's value is itself the copy affordance, on both shells. Text
  selection is off app-wide (`select-none`), so a press on it has no other
  meaning, and the desktop keeps its copy button beside it.
- The list stays flat and sorted (no Pinned/Recent groups).
- Unlock leads with the biometric tile when biometrics are enrolled, else the
  passphrase card. No auto-prompt on launch.
- Not adopted from the prototype: share button, recent searches, vault
  switcher, card face art. The Rowel mascot stays on the lock screen.

## Follow-ups

What the six slices deliberately left behind, smallest first:

- **Generator controls have no touch tier.** The length slider and the toggle
  rows are the desktop's sizes inside `Generator/Panel`, which both shells
  share; giving them a touch tier means sizing them through the panel rather
  than around it.
- **`big` secrets truncate when stacked.** A long SSH private key in a narrow
  container clips rather than wrapping; the stacked row needs its own
  presentation for the multi-line value tier.
