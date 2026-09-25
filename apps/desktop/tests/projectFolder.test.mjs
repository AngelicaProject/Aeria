import assert from "node:assert/strict";
import test from "node:test";
import { projectFolder } from "../src/projectFolder.ts";

test("new project folders join the parent with its own separator", () => {
  assert.equal(projectFolder("C:\\Users\\ada\\Documents\\Aeria", "ffxiv-ru"), "C:\\Users\\ada\\Documents\\Aeria\\ffxiv-ru");
  assert.equal(projectFolder("D:\\Projects\\", "  ffxiv ru  "), "D:\\Projects\\ffxiv ru");
  assert.equal(projectFolder("/home/ada/Documents/Aeria/", "ffxiv-ru"), "/home/ada/Documents/Aeria/ffxiv-ru");
});

test("names that are not a single usable folder are rejected", () => {
  for (const name of ["", "   ", ".", "..", "a/b", "a\\b", "a:b", "a?", "name.", "con<", "tab\tname"]) {
    assert.equal(projectFolder("C:\\Projects", name), null, JSON.stringify(name));
  }
});

test("Cyrillic names and parents with spaces are kept as typed", () => {
  assert.equal(
    projectFolder(String.raw`C:\Users\Анна Иванова\Documents\Aeria`, "Русский перевод"),
    String.raw`C:\Users\Анна Иванова\Documents\Aeria\Русский перевод`,
  );
  assert.equal(projectFolder("/home/анна/проекты", "перевод ё"), "/home/анна/проекты/перевод ё");
  assert.equal(projectFolder(String.raw`C:\Проекты`, "перевод."), null);
});
