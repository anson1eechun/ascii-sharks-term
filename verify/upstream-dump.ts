/**
 * The TypeScript side of the differential test: runs upstream's World and
 * AsciiRenderer (through cell-canvas.ts) on a fixed seed and scene, and writes
 * the final frame in the same format as the Rust test `dump_frame`.
 *
 *   bun verify/upstream-dump.ts <out.txt> [steps=240]     (env SHARKS_DUMP_FORCE=1 to force eat/respawn)
 */
const root = `${import.meta.dir}/..`;

// mulberry32, identical to rs/src/math.rs random().
let state = 42;
Math.random = () => {
  state = (state + 0x6d2b79f5) | 0;
  let t = Math.imul(state ^ (state >>> 15), 1 | state);
  t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
  return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
};

const { AsciiRenderer } = await import(`${root}/upstream/src/renderer.ts`);
const { World } = await import(`${root}/upstream/src/world.ts`);
const { CellCanvas, DEFAULT_BG, nullCanvas } = await import(`${root}/cell-canvas.ts`);
(globalThis as any).document = { createElement: () => nullCanvas };

const [outPath, stepsArg] = process.argv.slice(2);
const steps = Number(stepsArg ?? 240);
const force = Boolean(process.env.SHARKS_DUMP_FORCE);

// Must match dump_frame in rs/src/main.rs.
const cols = 150, rows = 42, cw = 13, ch = 27;
const kinds = ["hammerhead", "tiger", "zebra", "greathammer", "thresher"];
const world = new World(kinds.map((kind, i) => ({ kind, size: Math.max(0.3, 1 - i * 0.1) })));
const grid = new CellCanvas();
const renderer: any = new AsciiRenderer(grid);
renderer.resize(cols * cw, rows * Math.round(cw * 1.45), 1);
renderer.ch = ch;
world.resize(cols * cw, rows * ch, cw, ch);
grid.resize(cols, rows, cw, ch);

let trace = "";
for (let i = 0; i < steps; i++) {
  if (i === 100) world.splash(900, 500);
  if (force && i === 50) {
    world.sharks.forEach((sh: any, k: number) => {
      sh.hunger = 1;
      world.fish[k].x = sh.nodes[0];
      world.fish[k].y = sh.nodes[1];
    });
  }
  if (force && i === 60) {
    for (const f of world.fish.slice(10, 16)) {
      f.alive = false;
      f.respawnAt = world.time;
    }
  }
  world.step(1 / 60);
  const sx = world.fish.reduce((s: number, f: any) => s + f.x, 0);
  const sy = world.fish.reduce((s: number, f: any) => s + f.y, 0);
  const s0 = world.sharks[0];
  trace += `${i} ${sx.toFixed(10)} ${sy.toFixed(10)} ${s0.nodes[0].toFixed(6)} ${s0.nodes[1].toFixed(6)}\n`;
  if (i % 2 === 1) renderer.render(world, 1 / 30);
}

const alive = world.fish.filter((f: any) => f.alive).length;
const sum = world.fish.reduce((s: number, f: any) => s + f.x + f.y, 0);
let out = `world alive=${alive} sum=${sum.toFixed(6)}\n`;
for (const sh of world.sharks) out += `shark ${sh.id} ${sh.nodes[0].toFixed(4)} ${sh.nodes[1].toFixed(4)}\n`;
const hex = (v: number) => v.toString(16).padStart(6, "0");
for (let i = 0; i < cols * rows; i++) {
  const g = grid.glyph[i], f = grid.fg[i], b = grid.bg[i];
  if (g === " " && b === DEFAULT_BG) continue;
  out += `${i} ${g} ${g === " " ? "-" : hex(f)} ${b === DEFAULT_BG ? "def" : hex(b)}\n`;
}
await Bun.write(`${outPath}.trace`, trace);
await Bun.write(outPath, out);
