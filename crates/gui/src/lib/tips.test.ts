// @vitest-environment jsdom
import { beforeEach, expect, it } from "vitest";
import { markSeen, seen } from "./tips";

beforeEach(() => localStorage.clear());

it("remembers a tip once it is seen, and only that tip", () => {
  expect(seen("first-run")).toBe(false);
  markSeen("first-run");
  markSeen("first-run");
  expect(seen("first-run")).toBe(true);
  expect(seen("another")).toBe(false);
  expect(JSON.parse(localStorage.getItem("earworm.tips")!)).toEqual(["first-run"]);
});

it("treats storage it cannot read as nothing seen, rather than failing", () => {
  localStorage.setItem("earworm.tips", "{not json");
  expect(seen("first-run")).toBe(false);
  markSeen("first-run");
  expect(seen("first-run")).toBe(true);
});
