import assert from "node:assert/strict";
import test from "node:test";
import { isBrowserCommand, shortcutKey } from "../src/shortcuts.ts";

const press = (code, key, modifiers = {}) => ({ code, key, ctrlKey: false, metaKey: false, altKey: false, shiftKey: false, ...modifiers });

test("shortcut keys do not depend on the keyboard layout", () => {
  assert.equal(shortcutKey(press("KeyP", "з")), "p");
  assert.equal(shortcutKey(press("KeyP", "P")), "p");
  assert.equal(shortcutKey(press("Comma", "б")), ",");
  assert.equal(shortcutKey(press("NumpadEnter", "Enter")), "enter");
  assert.equal(shortcutKey(press("ArrowDown", "ArrowDown")), "arrowdown");
});

test("browser commands are cancelled and text editing is kept", () => {
  const ctrl = { ctrlKey: true };
  assert.equal(isBrowserCommand(press("KeyP", "з", ctrl), false), true);
  assert.equal(isBrowserCommand(press("KeyR", "к", ctrl), false), true);
  assert.equal(isBrowserCommand(press("KeyF", "а", ctrl), false), true);
  assert.equal(isBrowserCommand(press("F5", "F5"), false), true);
  assert.equal(isBrowserCommand(press("ArrowLeft", "ArrowLeft", { altKey: true }), false), true);
  for (const code of ["KeyA", "KeyC", "KeyV", "KeyX", "KeyY", "KeyZ"]) assert.equal(isBrowserCommand(press(code, "", ctrl), false), false, code);
  assert.equal(isBrowserCommand(press("KeyZ", "Z", { ctrlKey: true, shiftKey: true }), false), false);
  assert.equal(isBrowserCommand(press("KeyP", "p"), false), false);
  assert.equal(isBrowserCommand(press("Enter", "Enter", ctrl), false), false);
  assert.equal(isBrowserCommand(press("ArrowLeft", "ArrowLeft", ctrl), false), false);
});

test("development builds keep the developer tools keys", () => {
  const inspect = press("KeyI", "ш", { ctrlKey: true, shiftKey: true });
  assert.equal(isBrowserCommand(inspect, true), false);
  assert.equal(isBrowserCommand(inspect, false), true);
  assert.equal(isBrowserCommand(press("F12", "F12"), true), false);
});
