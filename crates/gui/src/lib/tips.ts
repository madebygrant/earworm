// One-time hints, remembered in the window's own storage. Losing it shows a hint again, which costs nothing,
// so it is not worth a file the worker would have to own.
const KEY = "earworm.tips";

function read(): string[] {
  try {
    const got: unknown = JSON.parse(localStorage.getItem(KEY) ?? "[]");
    return Array.isArray(got) ? got.filter((t) => typeof t === "string") : [];
  } catch {
    return [];
  }
}

export function seen(tip: string): boolean {
  return read().includes(tip);
}

export function markSeen(tip: string) {
  try {
    localStorage.setItem(KEY, JSON.stringify([...new Set([...read(), tip])]));
  } catch {
    // Storage refused: the hint shows again next time, and nothing else depends on it.
  }
}
