#!/usr/bin/env bun
/**
 * ascii-sharks in the terminal: upstream's World and AsciiRenderer, unmodified,
 * drawn through CellCanvas instead of a <canvas>. This file replaces only
 * upstream's engine.ts (frame loop, resize, pointer input).
 */
import { writeSync } from "node:fs";
import { parseArgs } from "node:util";
import { AsciiRenderer } from "./upstream/src/renderer";
import { World, type SharkKind, type SharkSpec } from "./upstream/src/world";
import { CellCanvas, DEFAULT_BG, nullCanvas } from "./cell-canvas";

const KINDS: SharkKind[] = ["classic", "hammerhead", "greathammer", "tiger", "mako", "thresher", "nurse", "zebra"];
const STEP = 1 / 60;
const MAX_STEPS = 4;
/** Height / width of a terminal cell when the terminal won't tell us. */
const FALLBACK_ASPECT = 2.1;

const HELP = `sharks — 終端機裡的 ASCII 鯊魚與魚群

用法: sharks [選項]

  -k, --sharks <種類,...>  要放哪些鯊魚，第一隻最大，後面依序縮小
                           預設：4 隻 classic
  -l, --list               列出所有鯊魚種類
  -f, --fps <n>            每秒畫面數（預設 30，越低越省資源）
  -S, --screensaver        螢幕保護模式：按任意鍵離開
      --aspect <n>         字元格高寬比（預設自動偵測，失敗時用 ${FALLBACK_ASPECT}）
  -h, --help               顯示這段說明

執行中：q / Esc 離開　p 暫停　滑鼠移動驚動魚群　點擊濺起水花`;

const { values: opts } = parseArgs({
  options: {
    sharks: { type: "string", short: "k" },
    list: { type: "boolean", short: "l" },
    fps: { type: "string", short: "f", default: "30" },
    screensaver: { type: "boolean", short: "S" },
    aspect: { type: "string" },
    help: { type: "boolean", short: "h" },
  },
});

function fail(msg: string): never {
  console.error(`sharks: ${msg}`);
  process.exit(2);
}

if (opts.help) {
  console.log(HELP);
  process.exit(0);
}
if (opts.list) {
  console.log(KINDS.join("\n"));
  process.exit(0);
}

let specs: SharkSpec[] | undefined;
if (opts.sharks) {
  const kinds = opts.sharks.split(",").map((k) => k.trim()).filter(Boolean);
  const bad = kinds.filter((k) => !KINDS.includes(k as SharkKind));
  if (bad.length) fail(`不認得的鯊魚種類：${bad.join(", ")}（用 -l 看清單）`);
  // Same sizing as upstream's demo.ts: first is biggest, the rest step down.
  specs = kinds.map((kind, i) => ({ kind: kind as SharkKind, size: Math.max(0.3, 1 - i * 0.1) }));
}

const fps = Number(opts.fps);
if (!(fps >= 1 && fps <= 120)) fail("--fps 要介於 1 到 120");
const aspectOverride = opts.aspect === undefined ? undefined : Number(opts.aspect);
if (aspectOverride !== undefined && !(aspectOverride >= 1 && aspectOverride <= 4)) fail("--aspect 要介於 1 到 4");

const { stdin, stdout } = process;
if (!stdout.isTTY || !stdin.isTTY) fail("需要在終端機裡執行");

// ── terminal setup / teardown ─────────────────────────────────────────

const ENTER = "\x1b[?1049h\x1b[?25l\x1b[2J";
const MOUSE_ON = "\x1b[?1003h\x1b[?1006h\x1b[?1004h";
const LEAVE = "\x1b[?1003l\x1b[?1006l\x1b[?1004l\x1b[0m\x1b[?25h\x1b[?1049l";

let timer: ReturnType<typeof setInterval> | undefined;

function quit() {
  clearInterval(timer);
  stdin.setRawMode(false);
  // Queued behind any frame still being written, so the screen is restored last.
  stdout.write(LEAVE, () => process.exit(0));
}

process.on("SIGTERM", quit);
process.on("SIGHUP", quit);
process.on("uncaughtException", (err) => {
  clearInterval(timer);
  writeSync(1, LEAVE);
  stdin.setRawMode(false);
  console.error(err);
  process.exit(1);
});

stdin.setRawMode(true);
stdin.resume();
writeSync(1, ENTER);

/** Asks the terminal for its cell size in pixels (XTWINOPS 16) and returns height / width. */
function queryCellAspect(): Promise<number | undefined> {
  return new Promise((resolve) => {
    const onData = (d: Buffer) => {
      const m = /\x1b\[6;(\d+);(\d+)t/.exec(d.toString());
      if (!m) return;
      done(Number(m[1]) / Number(m[2]));
    };
    const done = (aspect?: number) => {
      clearTimeout(t);
      stdin.off("data", onData);
      resolve(aspect && aspect >= 1 && aspect <= 4 ? aspect : undefined);
    };
    const t = setTimeout(done, 250);
    stdin.on("data", onData);
    writeSync(1, "\x1b[16t");
  });
}

const aspect = aspectOverride ?? (await queryCellAspect()) ?? FALLBACK_ASPECT;

// ── world + renderer ─────────────────────────────────────────────────

// AsciiRenderer makes an offscreen canvas for its grid-line backdrop.
(globalThis as any).document = { createElement: () => nullCanvas };

const grid = new CellCanvas();
const renderer = new AsciiRenderer(grid as unknown as HTMLCanvasElement);
const world = new World(specs);

/** Previous frame as the terminal shows it, for diffing. */
let shownGlyph: string[] = [];
let shownFg = new Int32Array(0);
let shownBg = new Int32Array(0);

function layout() {
  const cols = Math.max(1, stdout.columns || 80);
  const rows = Math.max(1, stdout.rows || 24);

  // The renderer derives its cell width from the canvas width (width / 124,
  // clamped to 8–13 px). Pick the width it will settle on, so its grid comes
  // out exactly cols wide.
  let cw = 8;
  for (let w = 13; w >= 8; w--) {
    if (Math.min(13, Math.max(8, Math.round((cols * w) / 124))) === w) {
      cw = w;
      break;
    }
  }
  const rendererCh = Math.round(cw * 1.45);
  renderer.resize(cols * cw, rows * rendererCh, 1);

  // Terminal cells are taller than the renderer's 1.45:1 cells. Swap in the
  // real proportion so sharks aren't stretched; the cell arrays it sized are
  // still cols × rows, and every later use reads ch afresh.
  const ch = Math.max(cw + 1, Math.round(cw * aspect));
  (renderer as any).ch = ch;
  world.resize(cols * cw, rows * ch, cw, ch);
  grid.resize(cols, rows, cw, ch);

  shownGlyph = new Array(cols * rows).fill("");
  shownFg = new Int32Array(cols * rows);
  shownBg = new Int32Array(cols * rows).fill(-2);
  stdout.write("\x1b[0m\x1b[2J");
}

const sgrFg = (c: number) => `\x1b[38;2;${(c >> 16) & 255};${(c >> 8) & 255};${c & 255}m`;
const sgrBg = (c: number) =>
  c === DEFAULT_BG ? "\x1b[49m" : `\x1b[48;2;${(c >> 16) & 255};${(c >> 8) & 255};${c & 255}m`;

/** Writes only the cells that changed since the last frame, in one synchronized update. */
function flush() {
  // Terminal falling behind: skip; the diff catches up on a later frame.
  if (stdout.writableLength > 1 << 18) return;
  const { cols, glyph, fg, bg } = grid;
  const n = glyph.length;
  let out = "";
  let pen = -1;
  let curFg = -2;
  let curBg = -2;
  for (let i = 0; i < n; i++) {
    const g = glyph[i];
    const b = bg[i];
    const f = fg[i];
    if (g === shownGlyph[i] && b === shownBg[i] && (g === " " || f === shownFg[i])) continue;
    if (pen !== i || i % cols === 0) out += `\x1b[${((i / cols) | 0) + 1};${(i % cols) + 1}H`;
    if (b !== curBg) {
      out += sgrBg(b);
      curBg = b;
    }
    if (g !== " " && f !== curFg) {
      out += sgrFg(f);
      curFg = f;
    }
    out += g;
    pen = i + 1;
    shownGlyph[i] = g;
    shownBg[i] = b;
    shownFg[i] = f;
  }
  if (out) stdout.write(`\x1b[?2026h${out}\x1b[0m\x1b[?2026l`);
}

// ── input ────────────────────────────────────────────────────────────

let paused = false;

function onInput(d: Buffer) {
  const s = d.toString();
  if (opts.screensaver) {
    // Any key, but not the terminal's own replies (CSI …).
    if (s === "\x1b" || !s.startsWith("\x1b[")) quit();
    return;
  }
  if (s === "q" || s === "Q" || s === "\x03" || s === "\x1b") quit();
  if (s === "p" || s === "P") paused = !paused;
  if (s.includes("\x1b[O")) world.pointer.active = false;

  // SGR mouse reports: ESC [ < button ; col ; row (M press/motion, m release)
  for (const m of s.matchAll(/\x1b\[<(\d+);(\d+);(\d+)([Mm])/g)) {
    const btn = Number(m[1]);
    const x = (Number(m[2]) - 0.5) * grid.cw;
    const y = (Number(m[3]) - 0.5) * grid.ch;
    world.pointer.x = x;
    world.pointer.y = y;
    world.pointer.active = true;
    if (m[4] === "M" && (btn & 32) === 0 && (btn & 3) === 0 && !paused) world.splash(x, y);
  }
}

// ── main loop (same fixed-timestep scheme as upstream's engine.ts) ────

layout();
stdout.on("resize", layout);
stdin.on("data", onInput);
if (!opts.screensaver) writeSync(1, MOUSE_ON);

let last = performance.now();
let acc = 0;

timer = setInterval(() => {
  const now = performance.now();
  const dt = Math.min(0.1, (now - last) / 1000);
  last = now;
  if (paused) return;
  acc += dt;
  let steps = 0;
  while (acc >= STEP && steps < MAX_STEPS) {
    world.step(STEP);
    acc -= STEP;
    steps++;
  }
  if (steps === MAX_STEPS) acc = 0;
  renderer.render(world, dt);
  flush();
}, 1000 / fps);
