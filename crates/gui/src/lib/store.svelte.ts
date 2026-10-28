// The page's one store: the reducer's state plus the IPC around it. Messages come in on one event and go
// through `apply`; the effects it returns are core questions, answered here and folded back in.
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { Msg, Word } from "./protocol";
import { apply, expire, initial, say, type Effect, type State } from "./reducer";

export interface Theme {
  name: string;
  light: boolean;
  names: string[];
  vars: Record<string, string>;
  said: string | null;
}

interface Sent {
  seq: number;
  msg: Msg;
}

interface Boot {
  theme: Theme;
  statuses: Word[];
  replay: Sent[];
}

export const app = $state({
  model: initial() as State,
  theme: null as Theme | null,
  /** Why the config would not build, shown where somebody launched from the Finder will see it. */
  error: null as string | null,
  /** The worker has dropped its sender. */
  gone: false,
});

// A floor plus reading time, as the terminal's: one length fitted "saved track 12" and not a long error.
const FLASH = 2000;
const PER_CHAR = 50;
const FLASH_MAX = 8000;
let flashTimer: ReturnType<typeof setTimeout> | undefined;

function timeFlash() {
  clearTimeout(flashTimer);
  const text = app.model.flash;
  if (text === null) return;
  flashTimer = setTimeout(() => {
    app.model = expire(app.model);
    timeFlash();
  }, Math.min(FLASH + PER_CHAR * text.length, FLASH_MAX));
}

function update(next: State) {
  const before = app.model.flash;
  app.model = next;
  if (next.flash !== before) timeFlash();
}

export function flash(text: string) {
  update(say(app.model, text));
}

async function run(effect: Effect) {
  if ("GateMarks" in effect) {
    const marked = await invoke<number[]>("gate_marks", effect.GateMarks);
    update({ ...app.model, marked });
  }
}

export function dispatch(msg: Msg) {
  const step = apply(app.model, msg);
  update(step.state);
  for (const effect of step.effects) void run(effect);
}

/** Paints the palette core sent as the stylesheets' variables, and flashes what became of the switch. */
export function paint(theme: Theme) {
  for (const [name, value] of Object.entries(theme.vars)) {
    document.documentElement.style.setProperty(`--${name}`, value);
  }
  document.documentElement.style.colorScheme = theme.light ? "light" : "dark";
  app.theme = theme;
  if (theme.said) flash(theme.said);
}

export async function walkTheme() {
  paint(await invoke<Theme>("theme_next"));
}

export async function chooseTheme(name: string) {
  paint(await invoke<Theme>("theme_set", { name }));
}

export async function tryCode(code: string) {
  const found = await invoke<{ said: string; theme: Theme | null; music: boolean }>("cheat", { code });
  if (found.theme) paint({ ...found.theme, said: null });
  flash(found.said);
}

let booted = false;

// Listening before `start`, which is what starts the worker, so its first messages have somewhere to go.
// Live messages wait until the history `start` hands back is applied, and any the history already held
// are dropped by number. Once only: a remount listening twice would apply every message twice.
export async function boot() {
  if (booted) return;
  booted = true;
  let last = -1;
  let held: Sent[] | null = [];
  const take = (sent: Sent) => {
    if (sent.seq <= last) return;
    last = sent.seq;
    dispatch(sent.msg);
  };
  await listen<Sent>("earworm://msg", (e) => (held ? held.push(e.payload) : take(e.payload)));
  await listen("earworm://gone", () => (app.gone = true));
  try {
    const { theme, statuses, replay } = await invoke<Boot>("start");
    app.model = { ...app.model, statuses };
    paint(theme);
    for (const sent of [...replay, ...(held ?? [])]) take(sent);
  } catch (e) {
    app.error = String(e);
  }
  held = null;
}
