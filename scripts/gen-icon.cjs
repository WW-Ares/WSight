// Zero-dependency icon generator.
//   node scripts/gen-icon.js
// Produces src-tauri/icons/icon.png (256) and icon.ico, drawn as a gradient
// ring so the widgets stay recognisable in the tray.
const fs = require("node:fs");
const path = require("node:path");
const zlib = require("node:zlib");

let crcTable = null;
function crc32(buf) {
  if (!crcTable) {
    crcTable = new Int32Array(256);
    for (let n = 0; n < 256; n++) {
      let c = n;
      for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
      crcTable[n] = c;
    }
  }
  let c = -1;
  for (let i = 0; i < buf.length; i++) c = crcTable[(c ^ buf[i]) & 0xff] ^ (c >>> 8);
  return (c ^ -1) >>> 0;
}

function pngChunk(type, data) {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length, 0);
  const t = Buffer.from(type, "ascii");
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(Buffer.concat([t, data])), 0);
  return Buffer.concat([len, t, data, crc]);
}

/** linear interpolation between two [r,g,b] stops, t in 0..1 */
function lerp(a, b, t) {
  return [
    Math.round(a[0] + (b[0] - a[0]) * t),
    Math.round(a[1] + (b[1] - a[1]) * t),
    Math.round(a[2] + (b[2] - a[2]) * t),
  ];
}

const STOPS = [
  [0x4a, 0xa8, 0xff], // blue
  [0x52, 0xd3, 0xa4], // green
  [0xb9, 0x8c, 0xff], // purple
];

function gradient(t) {
  const clamped = Math.max(0, Math.min(0.9999, t));
  const seg = clamped * (STOPS.length - 1);
  const i = Math.floor(seg);
  return lerp(STOPS[i], STOPS[i + 1], seg - i);
}

function drawRing(size) {
  const raw = Buffer.alloc((size * 4 + 1) * size);
  const c = (size - 1) / 2;
  const outer = size * 0.44;
  const thickness = size * 0.115;
  const inner = outer - thickness;
  const feather = size > 64 ? 1.2 : 0.8;

  for (let y = 0; y < size; y++) {
    const rowStart = y * (size * 4 + 1);
    raw[rowStart] = 0; // filter: none
    for (let x = 0; x < size; x++) {
      const idx = rowStart + 1 + x * 4;
      const dx = x - c;
      const dy = y - c;
      const d = Math.sqrt(dx * dx + dy * dy);

      let alpha = 0;
      if (d <= outer && d >= inner) {
        alpha = 255;
      } else if (d > outer && d < outer + feather * 2) {
        alpha = Math.round(255 * (1 - (d - outer) / (feather * 2)));
      } else if (d < inner && d > inner - feather * 2) {
        alpha = Math.round(255 * (1 - (inner - d) / (feather * 2)));
      }
      if (alpha <= 0) continue;

      // hue progresses clockwise starting from the top
      const angle = Math.atan2(dy, dx) + Math.PI / 2;
      const t = ((angle / (Math.PI * 2)) % 1 + 1) % 1;
      const [r, g, b] = gradient(t);

      raw[idx] = r;
      raw[idx + 1] = g;
      raw[idx + 2] = b;
      raw[idx + 3] = alpha;
    }
  }

  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(size, 0);
  ihdr.writeUInt32BE(size, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 6; // RGBA
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    pngChunk("IHDR", ihdr),
    pngChunk("IDAT", zlib.deflateSync(raw, { level: 9 })),
    pngChunk("IEND", Buffer.alloc(0)),
  ]);
}

function toIco(pngs) {
  const header = Buffer.alloc(6);
  header.writeUInt16LE(0, 0);
  header.writeUInt16LE(1, 2);
  header.writeUInt16LE(pngs.length, 4);

  let offset = 6 + pngs.length * 16;
  const entries = [];
  for (const { size, buf } of pngs) {
    const e = Buffer.alloc(16);
    e[0] = size >= 256 ? 0 : size;
    e[1] = size >= 256 ? 0 : size;
    e[2] = 0;
    e[3] = 0;
    e.writeUInt16LE(1, 4);
    e.writeUInt16LE(32, 6);
    e.writeUInt32LE(buf.length, 8);
    e.writeUInt32LE(offset, 12);
    entries.push(e);
    offset += buf.length;
  }
  return Buffer.concat([header, ...entries, ...pngs.map((p) => p.buf)]);
}

const outDir = path.join(__dirname, "..", "src-tauri", "icons");
fs.mkdirSync(outDir, { recursive: true });

const sizes = [16, 24, 32, 48, 64, 128, 256];
const pngs = sizes.map((size) => ({ size, buf: drawRing(size) }));

fs.writeFileSync(path.join(outDir, "icon.png"), pngs[pngs.length - 1].buf);
fs.writeFileSync(path.join(outDir, "icon.ico"), toIco(pngs));
// Tauri also likes explicit sizes present
fs.writeFileSync(path.join(outDir, "32x32.png"), drawRing(32));
fs.writeFileSync(path.join(outDir, "128x128.png"), drawRing(128));
fs.writeFileSync(path.join(outDir, "icon-256.png"), drawRing(256));

console.log(`icons written to ${outDir}`);
