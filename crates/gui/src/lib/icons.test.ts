import { describe, expect, it } from "vitest";
import sprite from "../icons.svg?raw";
import { ICONS } from "./icons";

describe("icons", () => {
  it("names exactly the symbols the sprite draws", () => {
    const ids = [...sprite.matchAll(/<symbol id="([^"]+)"/g)].map((m) => m[1]);
    expect([...ICONS].sort()).toEqual(ids.sort());
  });
});
