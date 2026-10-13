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
panes, and carries the lock control the vanished top bar used to. The rows
step up to the phone's type tier through `max-md:` inside `SettingsRow`, and
a sub-page's one action (Create, Save, Delete) is the same bottom slab the
generator root draws, lifted over the tab bar; the nav row's Back is its
Cancel.

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
a key's comment and fingerprint, a scope list, the one-time code), a check in
the value's place confirms it, and the
bottom button covers the main secret — so a rail holds only the eye and the
open-link. Every row's copy button sits in `ROW_COPY`, which is what hides it
there; the rows that draw their own geometry rather than rendering through
`FieldRow` (a custom pair, an env variable) take the phone grid from the same
exported classes, so one row on the phone is one row whatever it holds. The
one-time code is a row under the password with a countdown ring in its rail
(`fields/Otp/Countdown`, a conic gradient on a registered `--otp-left`), on
both shells; the editor's row is the secret's own box, with the live code as
a meta line under it once the secret parses. Custom fields are the card's own
last band, not a section or a card of their own: behind a gutter cut through
the card (`BAND_GUTTER`, the identity editor's cut between its bands) the
user's rows take the fixed rows' geometry, the user's label as the caption, and
the band ends on the "+ Add field" row (`fields/CustomFields`), which appends
a pair and hands it the caret — on the phone the label is the caption over
the value with no underline of its own ("Field name" in its place until it is
typed), and the editor's two controls — conceal, remove — are glyphs alone in
the rail at a compact 32px with 16px glyphs (two 44px blocks were a toolbar),
in the fixed rows' control ink, each still a 44px target through its slop. Under the rows,
files, passkeys and tags are sections (`SECTION_LABEL`: 12px
uppercase, tracking pulled in, inset to the rows), every "add" — under the
rows and the one inside the card — is the same dashed pill (`ADD_PILL`,
`ACTION_ROW`), and the dates are one centred
meta line at the end — `Compact/Detail/Meta`, handed to `Show/Body` as its
`Footer` and to `Edit/Body` as the editor's, which reads the same facts the
desktop's strip does. Files are a plain list, not
a card and not a gallery (`Show/Attachments/Row`): each a line with a 40px
thumbnail — the picture for an image, the kind's page for anything else —
beside its name and "670 KB · PNG", hairlines between the lines, on the
screen's own ground with nothing around them, as Mail and Notes list
attachments. On the desktop the lines are rounded tiles instead — lit on
hover, reaching 8px past the content on each side, air between them and no
hairlines — since a hairline under a rounded tile reads as the tile's edge. The lines, their "Attachments" heading
(`SECTION_LABEL_FLUSH`), the verbs under the sections and the notes under
them all stand on the pane outside any card, so they share the cards' margin
and not the inset of the text within one — through one wrapper, `EDGE_GUTTER`,
4px in from the cards' edge (a glyph or a capital set dead on that line reads
as starting before it), and none of them carries an inset of its own. A label
over a card (`SECTION_LABEL`) keeps the card's inset, aligned with its
captions as iOS aligns a group's header with its cells.
No controls on a line on the phone: the whole line
opens the file, and saving and removing live in the preview
(`Attachments/Preview`), one view for both shells — the screen on the phone,
a lightbox over the dimmed window on the desktop, told apart by
`max-md:`/`md:` on the same elements — with the
file's name centred in its bar, Close at the bar's left on the phone (where
a pushed screen keeps its back) and the `esc` close at its right on the
desktop (`elements/EscClose`, the one every dialog header draws), after the
file's own actions, which stand on glass tiles there (`bg-glass`, white
under the pointer) since bare glyphs on the blurred ground read as
watermarks. A picture is fitted to the stage but never past its actual size
(one image pixel per device pixel). The
lightbox starts under the wide shell's 38px chrome bar (`BELOW_CHROME`, the
same number as the bar's `CHROME_BAR`), so its Close never sits on the
macOS traffic lights and the bar stays the window's drag handle. On the
desktop the same two actions also stand at the line's end, shown to the
pointer and to a Tab (`HOVER_ONLY`, gated on `isMobile` rather than a
breakpoint, since it is the hover that a phone lacks), as every value row
offers its copy. The preview knows its list: the caption counts ("2 of 5"),
the arrow keys step to the neighbours, and the desktop floats a round control
at each edge of the stage for the pointer; removing the open file moves on to
the next. A picture larger than the stage is a press away from its actual
size on the desktop (`Preview/Picture`), the pointer saying which way.
Which file is open and which is being removed are the section's
(`Attachments/index`), so a row's action and the preview's bar ask through
one `ConfirmSheet`, rendered after the preview so it is the dialog on top.
Removing asks first through `elements/ConfirmSheet`: the platform's action
sheet on the phone, a red verb over a bold Cancel from the bottom edge, and
the same two rows on a small card on the desktop. The bytes come to the webview only for
the file being shown (`attachment_read`), never with the list — a row's
thumbnail is the backend's small PNG of the picture (`attachment_thumbnail`),
turned the way its EXIF says, asked for as the row comes on screen and made
one picture at a time, so a list of images costs kilobytes whatever the files
weigh and never a burst of decodes; a picture the
engine cannot decode (a TIFF or a BMP off Apple platforms) is fetched again as
the backend's PNG of it (`attachment_render`), so what an image shows does
not depend on the OS. Under the list, and alone on an entry with no files,
"Attach file" behind a paperclip: on the phone the same dashed pill "Add tag"
is (`ACTION_ROW`, the `max-md:` twin of `TagChip`'s `ADD_PILL`) — the
outline of a thing not yet there, 32px with a 44px hit area — so the two
verbs under the rows are visibly one control; on the desktop the accent text
line `AddAction` draws. Not a card cell: a boxed button under every section
made the screen a stack of cards rather than content with quiet verbs.

Tags stand in the same gutter as the files, under a `SECTION_LABEL_FLUSH`
heading, and are no more a card than they are: the chips themselves are the
content, with a dashed "Add tag" chip at
their end. One element draws a tag wherever it appears — on the entry, in the
editor with its ×, under the search field as the list's filter, and as the
add control — `elements/TagChip`: a 32px pill (24 on the desktop), no `#`
in front of it (a tag is not a hashtag; the list's title names the tag bare
too), and a hit area reaching the 44px the platform asks for. At rest it is
the cards' own white on the screen's grey ground, in an edge tinted with the
accent at a quarter — a step firmer than a card's hairline, so the pill holds
its shape beside the content without colouring it. Inside the picker's field,
which is itself the white detail surface, the same chip takes the tile grey
(`tone="field"`), since white on white would be no chip at all. The list's
filter chip is the one heavy one — the accent, solid — because there the tag
is what the screen is about; the picker's waiting chips are bare outlines in
the secondary ink. Pressing a tag while reading goes to everything filed
under it. Adding or taking one off never needs the editor: the add chip opens
`elements/TagPicker`, a bottom sheet holding one token field — the entry's
tags as chips inside it, the caret after them, as Mail's recipients field and
Notion's multi-select hold theirs — and, floating under it, the rest of the
vault's tags as outlined chips (busiest first, with counts) to tap into the
field. Typing narrows the floating chips; when nothing answers to it, a
dashed "Create …" chip stands first, and Return does the same; a name that
differs only by case is the tag already there; Backspace in the empty field
takes the last tag off. Nothing in the sheet is a card or a list: the chips
are the content, and a tag looks there as it does on the entry.
While reading, the sheet's choices are held in `Meta` and written back once
when it closes, if they changed (`saveTags`: the revealed entry saved with the
new list, so the secrets go back as they came and the row restamps like any
save); in the editor the sheet writes the draft directly and each chip's ×
takes a tag off. The picker is mounted on `document.body` — see the note on
overlays below — and takes the caret as it opens, which `useDialogFocus`
leaves where it is.

Overlays are not screens and do not live in the shell div. That div carries
`viewportStyle`'s translate, which makes it the containing block of anything
`fixed` inside it, so a sheet mounted there would take the keyboard offset
twice. The shell is sized from the visual viewport only while a field has the
focus (`useVisualViewport` answers null otherwise): iOS never shrinks the
layout viewport for the keyboard, so the shell shrinks itself to keep the
caret in view, but at rest it is the plain full-height box, so a stale
viewport reading — the Simulator's accessory strip, left behind after a field
went away — can never leave a band of the page's ground under the tab bar. `Generator/Attached`, and every fixed overlay after it, is a sibling of
the shell; one opened from inside a screen (the tag picker, which has a field
to type into) portals itself to `document.body` for the same reason.

Before any of that there is the lock, which is not a screen of the vault but a
flow of its own: `App` renders `Auth/LockScreen` on compact and `Auth` on wide,
both driven by `Auth/useUnlock` (attempt phase, lockout countdown, mascot gaze,
the eyebrow's text and tone). The desktop leads with the passphrase card and
keeps biometrics as its end segment, centred as one block on the auth ground.
The phone lays the same parts out in three bands over the whole ground
(`AuthShell fill`): the brand and the lockbox chip under the safe area; at the
exact centre of the screen the status line over the 88px biometric tile when
a key is enrolled, or over the card; and by the footer "Enter Master Password"
as a plain 44px text control — the backup for the day Face ID does not
answer, not a second button the size of the first. The ground is sized to the
visual viewport (`viewportStyle`), so the keyboard the card brings up
shortens it rather than covering the card. Which biometry the copy names is
`lib/biometry` — `biometryLabel(type)`
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

Screens are layered, not swapped: `Compact/Stack` keeps the screen a push came
from mounted under the pushed one — the tab root (with its tab bar) under the
entry or the form, the settings root under a section pane, the pane under its
sub-page — so it keeps its scroll and is there to come back to, and so the
motion has something to happen over. Under a screen the layer is `inert`, and
its geometry is one number, `--pop` (`.stack-under` in `theme.css`): how far
the screen over it has been popped. A push (`animate-push`, the whole width
from the right) transitions it from 1 to 0, which recedes the layer a third of
its width and dims it, as iOS's stack does; a form (`animate-rise`, from the
bottom) only dims it. There are no exit animations: the way back by a tap is a
cut, since the store has already dropped what the screen was showing.

A screen pushed from the right can also be popped the iOS way: a drag in from
the left edge takes it with the finger, and the layer under it comes along —
`--pop` written per frame — out of its receded rest and up to full width and
brightness as the screen clears. Letting go past a third of the width (or a
flick) slides it the rest of the way out and runs the same handler its back
control does (`Compact/useSwipeBack`, on the entry screen and both settings
panes, under the same `settingsLocked`); short of that, both spring back. The
form is not among them: it rose from the bottom and its only exit is the
guarded Cancel. The drag starts on a 24px strip the screen lays over the left
edge of its scroller, `touch-action: none` — the way the bottom sheet's
grabber owns its drag. A touch that starts inside a scroller is the webview's
to scroll the moment it moves on an axis the scroller has room on, and the
pointer under the swipe is cancelled mid-drag; on the strip there is no
scroller to claim it. A tap on the strip is passed on to what lies under it.
Every one of these screens scrolls through `SCREEN_SCROLLER` (`Compact/chrome`).

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
- Bottom sheets carry a grabber and dismiss on scrim tap, swipe or Escape,
  with no close icon (a screen-reader-only Close sits in the grabber row);
  full-screen forms
  put Cancel on the left and Save on the right of the nav row.
- Where a photo or a file comes from is one sheet, `elements/SourceSheet`:
  Take Photo · Photo Library · Choose File, under the names the system gives
  them, each row opening the system's own picker (the camera through the
  webview's capture input, the other two through the Rust dialog). Attach and
  Scan both ask with it. A sheet never stacks on a sheet: a row of the Add
  picker that needs one closes the picker and opens it in its place, as
  Receive does. A lock drops every sheet with the session (`resetUi`); a
  system picker already up is the OS's and outlives it until dismissed.
- The primary action of a detail screen is a bottom, thumb-reachable button.
- Copy on tap for value rows; the feedback is a check in the value's place,
  on the element that was tapped.
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
- Not adopted from the prototype: share button, recent searches, lockbox
  switcher, card face art. The Rowel mascot stays on the lock screen.

## Follow-ups

What the six slices deliberately left behind, smallest first:

- **The length slider has no touch tier.** The switches, segments, chips and
  inline buttons the generator and Settings share take theirs through
  `any-pointer-coarse:` inside each element; the range input is the one
  control still at the desktop's size.
- **`big` secrets truncate when stacked.** A long SSH private key in a narrow
  container clips rather than wrapping; the stacked row needs its own
  presentation for the multi-line value tier.
