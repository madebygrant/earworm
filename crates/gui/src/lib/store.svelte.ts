// The page's one store: the reducer's state plus the IPC around it. Messages come in on one event and go
// through `apply`; the effects it returns are core questions, answered here and folded back in.
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { AlbumRow, Msg, Reply, SettingRow, Word } from "./protocol";
import { apply, expire, hasRoom, initial, say, type Effect, type State } from "./reducer";

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

export interface Setup {
  text: string;
  format: string;
  rows: SettingRow[];
  brief: [string, string][];
  dir: string;
  key: boolean;
}

interface Boot {
  theme: Theme;
  statuses: Word[];
  setup: Setup;
  replay: Sent[];
  music: boolean;
}

export const app = $state({
  model: initial() as State,
  theme: null as Theme | null,
  // Why the config would not build, shown where somebody launched from the Finder will see it.
  error: null as string | null,
  // The worker has dropped its sender.
  gone: false,
  setup: null as Setup | null,
  // The YouTube Music unlock. Off, the artist page lists and never offers a download, and says nothing of it.
  music: false,
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

function tell(text: string) {
  if (hasRoom(app.model)) flash(text);
  else setTimeout(() => tell(text), 1000);
}

// One at a time and in order, each reading the state as it is when it runs: two run side by side from
// snapshots took the new releases and the pages read from before each other and overwrote one another.
let effects = Promise.resolve();
function run(effect: Effect) {
  effects = effects.then(() => runNow(effect)).catch(() => {});
}

async function runNow(effect: Effect) {
  if ("GateMarks" in effect) {
    const marked = await invoke<number[]>("gate_marks", effect.GateMarks);
    update({ ...app.model, marked });
  } else if ("Unread" in effect) {
    const newReleases = await invoke<[string, number, string][]>("unread", { found: effect.Unread.found, read: app.model.pagesRead });
    update({ ...app.model, newReleases });
  } else if ("MarkNew" in effect) {
    const { name } = effect.MarkNew;
    const asked = { ...effect.MarkNew, fresh: app.model.newReleases, read: app.model.pagesRead };
    const [rows, newReleases, pagesRead] = await invoke<[AlbumRow[], [string, number, string][], string[]]>("mark_new", asked);
    // Only onto the page it was asked for: a quick Esc to another artist has moved on.
    const page = app.model.page.name === name ? { ...app.model.page, rows } : app.model.page;
    update({ ...app.model, page, newReleases, pagesRead });
  }
}

// What the page's own state does on a message: a new run starts with no filter, or one left over from the
// last playlist hides most of the next and reads as a broken run. Set by the app shell.
export const hooks = { restart: () => {} };

export function dispatch(msg: Msg) {
  if (msg === "Restart") hooks.restart();
  const step = apply(app.model, msg);
  update(step.state);
  for (const effect of step.effects) void run(effect);
}

// Paints the palette core sent as the stylesheets' variables, and flashes what became of the switch.
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

export async function tryCode(code: string) {
  const found = await invoke<{ said: string; theme: Theme | null; music: boolean }>("cheat", { code });
  if (found.theme) paint({ ...found.theme, said: null });
  app.music = found.music;
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
    const { theme, statuses, setup, replay, music } = await invoke<Boot>("start");
    app.music = music;
    app.model = { ...app.model, statuses, settings: { text: setup.text, format: setup.format, rows: setup.rows, brief: setup.brief } };
    app.setup = setup;
    paint(theme);
    for (const sent of [...replay, ...(held ?? [])]) take(sent);
    // The shell answers once a session and `null` when the check is off, so this waits for room in the
    // queue rather than being dropped from it at a busy start.
    invoke<string | null>("latest").then((news) => news && tell(news)).catch(() => {});
  } catch (e) {
    app.error = String(e);
  }
  held = null;
}

// Answers the question on screen. Taken off the screen first, so a second key cannot answer it twice.
export async function reply(r: Reply) {
  const open = app.model.prompt;
  if (!open) return;
  update({ ...app.model, prompt: null });
  await invoke("answer", { id: open.id, reply: r });
}

// Answers the pick gate with the marks and nothing else: never the cursor, which would turn "I unmarked
// everything" into "download the row I happen to be on".
export async function answerPick(picked: number[]) {
  const id = app.model.picking;
  if (id === null) return;
  update({ ...app.model, picking: null, marked: [], follow: true });
  await invoke("answer", { id, reply: { Picked: picked } });
}
