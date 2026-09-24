import assert from "node:assert/strict";
import test from "node:test";
import { en } from "../src/i18n/en.ts";
import { ru } from "../src/i18n/ru.ts";
import { createTranslator, resolveLocale } from "../src/i18n/translate.ts";

const catalogs = { ru };

function forms(message) {
  return typeof message === "string" ? [message] : Object.values(message);
}

function placeholders(message) {
  return [...new Set(forms(message).flatMap((form) => [...form.matchAll(/\{(\w+)\}/g)].map((match) => match[1])))].sort();
}

test("every catalog has exactly the English keys and placeholders", () => {
  for (const [locale, catalog] of Object.entries(catalogs)) {
    assert.deepEqual(Object.keys(catalog).sort(), Object.keys(en).sort(), `${locale} keys`);
    for (const key of Object.keys(en)) {
      assert.deepEqual(placeholders(catalog[key]), placeholders(en[key]), `${locale} ${key} placeholders`);
      assert.equal(typeof catalog[key], typeof en[key], `${locale} ${key} plural shape`);
      for (const form of forms(catalog[key])) assert.ok(form.trim().length > 0, `${locale} ${key} is empty`);
    }
  }
});

test("plural messages cover every category of their locale", () => {
  for (const [locale, catalog] of Object.entries({ en, ...catalogs })) {
    const categories = new Intl.PluralRules(locale).resolvedOptions().pluralCategories;
    for (const [key, message] of Object.entries(catalog)) {
      if (typeof message === "string") continue;
      for (const category of categories) assert.ok(message[category], `${locale} ${key} lacks ${category}`);
    }
  }
});

test("system locale falls back to English unless a supported language is preferred", () => {
  assert.equal(resolveLocale("system", ["ru-RU", "en-US"]), "ru");
  assert.equal(resolveLocale("system", ["de-DE", "ru"]), "ru");
  assert.equal(resolveLocale("system", ["ja-JP"]), "en");
  assert.equal(resolveLocale("system", []), "en");
  assert.equal(resolveLocale("en", ["ru-RU"]), "en");
  assert.equal(resolveLocale("ru", ["en-US"]), "ru");
});

test("translator selects Russian plural forms and formats numbers", () => {
  const t = createTranslator("ru");
  assert.equal(t("launcher.recentCount", { count: 1 }), "1 локальный проект");
  assert.equal(t("launcher.recentCount", { count: 3 }), "3 локальных проекта");
  assert.equal(t("launcher.recentCount", { count: 11 }), "11 локальных проектов");
  assert.equal(t("launcher.recentCount", { count: 21 }), "21 локальный проект");
  assert.equal(t("status.rowsLoaded", { count: 12500 }), "Загружено 12 500 записей");
  assert.equal(t("common.cellLocation", { sheet: "Item", row: "12500", subrow: "0", column: "3" }), "Item 12500:0 · кол. 3");
});

test("English translator keeps existing wording", () => {
  const t = createTranslator("en");
  assert.equal(t("launcher.recentCount", { count: 1 }), "1 local project");
  assert.equal(t("status.rowsLoaded", { count: 1200 }), "1,200 rows loaded");
  assert.equal(t("launcher.noMatch", { query: "abc" }), "No projects match “abc”.");
});
