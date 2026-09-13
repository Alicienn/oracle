// Generates the tray icons from the source logo.
//
// The source artwork sits on an opaque off-white ground, which would read as a white
// tile in the Windows notification area. This strips that ground to alpha, trims the
// result to the mark's bounding box, and emits the sizes the tray needs.
//
// Run once after changing assets/logo.png:  node scripts/make-tray-icons.mjs

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

async function main() {
  mkdirSync(outDir, { recursive: true });
  const mark = await stripGround();

  // Padded square so the mark never touches the edge of its tray slot.
  const square = async (size) => {
    const inner = Math.round(size * 0.82);
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
            .resize(inner, inner, { fit: "contain", background: { r: 0, g: 0, b: 0, alpha: 0 } })
            .toBuffer(),
          gravity: "center",
        },
      ])
      .png()
      .toBuffer();
  };

  for (const size of [16, 20, 24, 32, 48, 64, 256]) {
    await sharp(await square(size)).toFile(resolve(outDir, `tray-${size}.png`));
  }

  // Tauri's tray API takes a single image; 32px covers the common DPI scalings.
  await sharp(await square(32)).toFile(resolve(outDir, "tray.png"));

  // Transparent mark for use inside the UI.
  await sharp(mark).resize(512, 512, { fit: "contain", background: { r: 0, g: 0, b: 0, alpha: 0 } })
    .toFile(resolve(root, "assets/mark.png"));

  console.log("tray icons written to", outDir);
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
