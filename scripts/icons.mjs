// Regenerates every app icon from the sources in src-tauri/icons.
//
// rowel.png is the app icon as designed (the glyph on its padded, shadowed
// card). Desktop (macOS/Windows/Linux) and Android use it as-is through
// `tauri icon`, the natively recommended path. The tray takes tray.svg, the
// bare glyph. iOS masks icons into its own squircle, so it needs the
// full-bleed variant rowel-ios.svg: `tauri icon` has no iOS-only mode and
// always regenerates the whole set, so that variant is rendered into a temp
// dir and only its ios/ output is copied over — into src-tauri/icons/ios and
// the Xcode asset catalog.
import { execSync } from "node:child_process";
import { cpSync, mkdirSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";

const ICONS = "src-tauri/icons";
const XCASSETS = "src-tauri/gen/apple/Assets.xcassets/AppIcon.appiconset";

const run = (cmd) => execSync(cmd, { stdio: "inherit" });

run(`tauri icon ${ICONS}/rowel.png -o ${ICONS}`);
run(`tauri icon ${ICONS}/tray.svg -o ${ICONS}/tray -p 72`);

// The macOS icon, packed again by Apple's own tools. The small sizes `tauri icon`
// resamples come out with a pure white rim on the card, and macOS draws an icon
// like that on a flat white plate, dropping the card's gradient. sips and
// iconutil only exist on macOS; elsewhere the icns stays as `tauri icon` made it.
if (process.platform === "darwin") {
  const iconset = mkdtempSync(join(tmpdir(), "rowel-icns-")) + "/icon.iconset";
  try {
    mkdirSync(iconset);
    for (const size of [16, 32, 128, 256, 512]) {
      for (const [scale, suffix] of [[1, ""], [2, "@2x"]]) {
        const px = size * scale;
        run(`sips -z ${px} ${px} ${ICONS}/rowel.png --out ${iconset}/icon_${size}x${size}${suffix}.png >/dev/null`);
      }
    }
    run(`iconutil -c icns ${iconset} -o ${ICONS}/icon.icns`);
  } finally {
    rmSync(dirname(iconset), { recursive: true, force: true });
  }
}

const tmp = mkdtempSync(join(tmpdir(), "rowel-ios-icons-"));
try {
  run(`tauri icon ${ICONS}/rowel-ios.svg -o ${tmp}`);
  cpSync(join(tmp, "ios"), `${ICONS}/ios`, { recursive: true });
  cpSync(join(tmp, "ios"), XCASSETS, { recursive: true });
} finally {
  rmSync(tmp, { recursive: true, force: true });
}
