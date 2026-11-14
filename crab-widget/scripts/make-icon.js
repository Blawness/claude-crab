// Draws the pixel crab into a 512x512 PNG (no dependencies) for `tauri icon`.
const fs = require('fs');
const path = require('path');
const zlib = require('zlib');

const SIZE = 512;
const UNIT = 26; // one crab pixel
const SHELL = [0xd9, 0x77, 0x57, 255];
const INK = [0x1d, 0x1b, 0x1a, 255];

// Crab on an 18 x 11 grid: [x, y, w, h, color]
const rects = [
  [3, 8, 1, 3, SHELL], [6, 8, 1, 3, SHELL], [11, 8, 1, 3, SHELL], [14, 8, 1, 3, SHELL],
  [0, 3, 2, 2, SHELL], [16, 3, 2, 2, SHELL],
  [2, 0, 14, 8, SHELL],
  [5, 2, 1, 3, INK], [12, 2, 1, 3, INK],
];

const px = Buffer.alloc(SIZE * SIZE * 4);
const ox = (SIZE - 18 * UNIT) / 2;
const oy = (SIZE - 11 * UNIT) / 2;
for (const [x, y, w, h, c] of rects) {
  for (let j = oy + y * UNIT; j < oy + (y + h) * UNIT; j++) {
    for (let i = ox + x * UNIT; i < ox + (x + w) * UNIT; i++) {
      px.set(c, (j * SIZE + i) * 4);
    }
  }
}

const raw = Buffer.alloc((SIZE * 4 + 1) * SIZE);
for (let y = 0; y < SIZE; y++) {
  px.copy(raw, y * (SIZE * 4 + 1) + 1, y * SIZE * 4, (y + 1) * SIZE * 4);
}
const chunk = (type, data) => {
  const body = Buffer.concat([Buffer.from(type), data]);
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(zlib.crc32(body) >>> 0);
  return Buffer.concat([len, body, crc]);
};
const ihdr = Buffer.alloc(13);
ihdr.writeUInt32BE(SIZE, 0);
ihdr.writeUInt32BE(SIZE, 4);
ihdr.set([8, 6, 0, 0, 0], 8);
const png = Buffer.concat([
  Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
  chunk('IHDR', ihdr),
  chunk('IDAT', zlib.deflateSync(raw)),
  chunk('IEND', Buffer.alloc(0)),
]);
const out = path.join(__dirname, '..', 'app-icon.png');
fs.writeFileSync(out, png);
console.log('wrote', out);
