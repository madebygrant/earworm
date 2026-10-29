// Dev builds opened in a browser rather than the app: replays a session recorded by the debug app
// (`EARWORM_SESSION`) with IPC mocked, so each screen can be drawn and captured where the app's own
// window cannot be. `?file=` names the session, `?upto=` how many of its messages to apply, and `quit`,
// `logs`, `mode` and `tokens` set the page's own state. Never in a build.
import { mockConvertFileSrc, mockIPC, mockWindows } from "@tauri-apps/api/mocks";

const q = new URLSearchParams(location.search);
const session = await fetch(`/@fs${q.get("file")}`).then((r) => r.json());
// `?tokens` keeps the stylesheet's own graphite, for comparing against the mockups' pictures.
if (q.has("tokens")) session.theme = { ...session.theme, vars: {} };
// The history holds every question, answered or not; one is still open only if it is the last thing applied.
const upto: unknown[] = session.msgs.slice(0, Number(q.get("upto") ?? session.msgs.length));
const asks = (m: unknown) => typeof m === "object" && m !== null && ("Ask" in m || "Pick" in m);
const msgs = upto.filter((m, i) => !asks(m) || i === upto.length - 1);

mockWindows("main");
mockConvertFileSrc("macos");
mockIPC(
  (cmd, args) => {
    const a = args as Record<string, unknown>;
    switch (cmd) {
      case "start":
        return { theme: session.theme, statuses: session.statuses, setup: session.setup, replay: msgs.map((msg, seq) => ({ seq, msg })) };
      case "tools":
        return session.tools;
      // Identity orders: the session is already in the worker's order, and the preview narrows nothing.
      case "rows":
        return (a.tracks as unknown[]).map((_, i) => i);
      case "shelf_rows":
        return (a.library as unknown[]).map((_, i) => i);
      // Close enough to core's for a picture: everything still to fetch.
      case "gate_marks":
        return (a.tracks as { index: number; status: string }[]).filter((t) => t.status === "Pending").map((t) => t.index);
      // Core's tally, near enough for a picture: settled statuses only, in the order core sends.
      case "counts": {
        const words = session.statuses as { status: string; tally: number | null }[];
        const tracks = a.tracks as { status: string }[];
        return words
          .filter((w) => w.tally !== null && tracks.some((t) => t.status === w.status))
          .sort((x, y) => x.tally! - y.tally!)
          .map((w) => [w.status, tracks.filter((t) => t.status === w.status).length]);
      }
      default:
        return null;
    }
  },
  { shouldMockEvents: true },
);

export const previewState = {
  quit: q.has("quit"),
  logs: q.has("logs"),
  mode: q.get("mode") as "tiles" | "list" | null,
};
