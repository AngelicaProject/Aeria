import assert from "node:assert/strict";
import test from "node:test";
import { commonTargetLanguages, isTargetLanguage, languageName, suggestedTargetLanguage } from "../src/targetLanguages.ts";

test("target languages are BCP 47 tags other than und, as Rust accepts them", () => {
  for (const tag of ["ru", "uk", "pt-BR", "zh-Hant", "es-419", ...commonTargetLanguages]) assert.ok(isTargetLanguage(tag), tag);
  for (const tag of ["", "r", "russian", "ru_RU", "ru-", "-ru", "ru-toolongsubtag", "und", "UND"]) assert.ok(!isTargetLanguage(tag), tag);
});

test("a language reads as its own name with its tag", () => {
  assert.match(languageName("ru"), /^русский \(ru\)$/i);
  assert.equal(languageName("qaa-x"), "qaa-x");
});

test("a new project suggests the interface language unless the game is in it", () => {
  assert.equal(suggestedTargetLanguage("ru", "en"), "ru");
  assert.equal(suggestedTargetLanguage("en", "en"), null);
  assert.equal(suggestedTargetLanguage("pt-BR", "ja"), "pt");
});
