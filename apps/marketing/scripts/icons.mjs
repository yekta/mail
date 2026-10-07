// Draws the favicons and the touch icon from the mark, black on the light tile of the apps' icon
// (apps/macos/Resources/AppIcon.icon). Run it with `pnpm icons`.
import { writeFileSync } from "node:fs"
import sharp from "sharp"

const MARK =
  "M17.47 14.216l2.71-10.727H24l-4.55 16.715h-3.843l-3.56-11.08-3.583 11.08H4.55L0 3.49h3.96l2.76 10.703L10.184 3.49h3.819l3.466 10.727z"
const TOP = "#fff"
const BOTTOM = "#e5e5e5"
const GRADIENT = `<linearGradient id="fill" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="${TOP}"/><stop offset="1" stop-color="${BOTTOM}"/></linearGradient>`

/** The mark, black, centred in a `canvas`-wide square and `ratio` of its width. */
function mark(canvas, ratio) {
  const size = canvas * ratio
  const offset = Number(((canvas - size) / 2).toFixed(2))
  return `<path transform="translate(${offset} ${offset}) scale(${Number((size / 24).toFixed(4))})" d="${MARK}" fill="#000"/>`
}

const svg = (canvas, body) =>
  `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${canvas} ${canvas}">${body}</svg>\n`

/** A rounded tile, for places that show the icon as it is. */
const tile = (ratio) =>
  svg(
    32,
    `<defs>${GRADIENT}</defs><rect width="32" height="32" rx="8" fill="url(#fill)"/>${mark(32, ratio)}`
  )

/** A full square, for places that cut their own shape out of it. */
const square = (ratio) =>
  svg(
    32,
    `<defs>${GRADIENT}</defs><rect width="32" height="32" fill="url(#fill)"/>${mark(32, ratio)}`
  )

// How much of a tile's width the mark takes up: as much as in the apps' icon.
const MARK_IN_TILE = 20 / 32
const MARK_IN_APPLE_TILE = 624 / 1024

/** Rasterised at twice the size it is asked for, then scaled down. */
function png(source, size) {
  const canvas = Number(source.match(/viewBox="0 0 (\d+)/)[1])
  const density = (72 * 2 * size) / canvas
  return sharp(Buffer.from(source), { density })
    .resize(size, size)
    .png()
    .toBuffer()
}

/** An .ico file holding the PNGs as they are. */
function ico(images) {
  const header = Buffer.alloc(6)
  header.writeUInt16LE(1, 2)
  header.writeUInt16LE(images.length, 4)
  let offset = header.length + 16 * images.length
  const entries = images.map(({ size, data }) => {
    const entry = Buffer.alloc(16)
    entry.writeUInt8(size, 0)
    entry.writeUInt8(size, 1)
    entry.writeUInt16LE(1, 4)
    entry.writeUInt16LE(32, 6)
    entry.writeUInt32LE(data.length, 8)
    entry.writeUInt32LE(offset, 12)
    offset += data.length
    return entry
  })
  return Buffer.concat([
    header,
    ...entries,
    ...images.map((image) => image.data),
  ])
}

const icoImages = await Promise.all(
  [16, 32, 48].map(async (size) => ({
    size,
    data: await png(tile(MARK_IN_TILE), size),
  }))
)
const files = {
  "favicon.svg": tile(MARK_IN_TILE),
  "favicon.ico": ico(icoImages),
  "apple-touch-icon.png": await png(square(MARK_IN_APPLE_TILE), 180),
}
for (const [name, data] of Object.entries(files))
  writeFileSync(`public/${name}`, data)
