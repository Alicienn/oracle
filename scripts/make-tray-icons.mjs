// Generates every icon Oracle ships, from the source artwork.
//
// The artwork sits on an opaque off-white ground. Left as-is it reads as a white tile in
// the taskbar and the notification area, and the mark itself ends up small inside its own
// padding. This strips the ground to alpha, trims to the mark's bounding box, and re-lays
// it out on a transparent square so the orange fills the tile.
//
// Run after changing assets/logo.png:
//   node scripts/make-tray-icons.mjs && npx tauri icon assets/icon-source.png

import sharp from "sharp";
import { mkdirSync } from "node:fs";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const source = resolve(root, "assets/logo.png");
const outDir = resolve(root, "src-tauri/icons");

// Anything this close to the paper colour is background, not artwork.
const PAPER_THRESHOLD = 210;

async function stripGround() {
  const { data, info } = await sharp(source)
    .ensureAlpha()
    .raw()
    .toBuffer({ resolveWithObject: true });

  const { width, height, channels } = info;
  const out = Buffer.from(data);

  for (let i = 0; i < out.length; i += channels) {
    const r = out[i];
    const g = out[i + 1];
    const b = out[i + 2];

    // The paper is light and near-neutral; the mark is saturated terracotta.
    const isLight = r > PAPER_THRESHOLD && g > PAPER_THRESHOLD && b > PAPER_THRESHOLD;
    const spread = Math.max(r, g, b) - Math.min(r, g, b);

    if (isLight && spread < 24) {
      out[i + 3] = 0;
    }
  }

  return sharp(out, { raw: { width, height, channels } })
    .png()
    .toBuffer()
    .then((buf) => sharp(buf).trim({ threshold: 1 }).toBuffer());
}

/**
 * Lays the trimmed mark on a transparent square.
 *
 * `inset` is the fraction of the tile the mark occupies. Small tray slots need a sliver of
 * breathing room so neighbouring icons do not touch; an app icon should fill its tile, since
 * Windows already draws it inside its own padding.
 */
async function square(mark, size, inset) {
  const inner = Math.round(size * inset);

  return sharp({
    create: {
      width: size,
      height: size,
      channels: 4,
      background: { r: 0, g: 0, b: 0, alpha: 0 },
    },
  })
    .composite([
      {
        input: await sharp(mark)
          .resize(inner, inner, {
            fit: "contain",
            background: { r: 0, g: 0, b: 0, alpha: 0 },
          })
          .toBuffer(),
        gravity: "center",
      },
    ])
    .png()
    .toBuffer();
}

async function main() {
  mkdirSync(outDir, { recursive: true });
  const mark = await stripGround();

  // The source `tauri icon` regenerates the whole set from. Nearly edge to edge, and fully
  // transparent, so no white tile survives anywhere.
  await sharp(await square(mark, 1024, 0.96)).toFile(resolve(root, "assets/icon-source.png"));

  for (const size of [16, 20, 24, 32, 48, 64, 256]) {
    await sharp(await square(mark, size, 0.94)).toFile(resolve(outDir, `tray-${size}.png`));
  }

  // Tauri's tray API takes a single image; 32px covers the common DPI scalings.
  await sharp(await square(mark, 32, 0.94)).toFile(resolve(outDir, "tray.png"));

  // Transparent mark for use inside the UI.
  await sharp(mark)
    .resize(512, 512, { fit: "contain", background: { r: 0, g: 0, b: 0, alpha: 0 } })
    .toFile(resolve(root, "assets/mark.png"));

  console.log("icons written to", outDir);
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
