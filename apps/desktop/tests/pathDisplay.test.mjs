import assert from "node:assert/strict";
import test from "node:test";

import { displayPath, displayPathName } from "../src/pathDisplay.ts";

test("display paths remove Windows device prefixes without changing ordinary paths", () => {
  assert.equal(displayPath(String.raw`\\?\D:\aerlatest`), String.raw`D:\aerlatest`);
  assert.equal(displayPath(String.raw`\\.\D:\aerlatest`), String.raw`D:\aerlatest`);
  assert.equal(displayPath(String.raw`D:\aerlatest`), String.raw`D:\aerlatest`);
});

test("display path names use the repository folder after prefix normalization", () => {
  assert.equal(displayPathName(String.raw`\\?\D:\projects\aerlatest`), "aerlatest");
});

test("Cyrillic paths with spaces keep every character", () => {
  const path = String.raw`\\?\C:\Users\Анна Иванова\Documents\Aeria\мой перевод`;
  assert.equal(displayPath(path), String.raw`C:\Users\Анна Иванова\Documents\Aeria\мой перевод`);
  assert.equal(displayPathName(path), "мой перевод");
  assert.equal(displayPathName("/home/анна/мои проекты/перевод ru/"), "перевод ru");
});
