import assert from "node:assert/strict";
import test from "node:test";
import { columnLabel } from "../src/columnNames.ts";
import { createTranslator } from "../src/i18n/translate.ts";

const en = createTranslator("en");
const ru = createTranslator("ru");
const placeName = { schemaVersion: "2026.09.01.0000.0000", names: ["Name", "Adjective", "NameNoArticle"] };

test("a column the schema names shows its name and the schema version", () => {
  assert.deepEqual(columnLabel(ru, 2, placeName), {
    label: "кол. 2 · NameNoArticle",
    hint: "Имя столбца из EXDSchema 2026.09.01.0000.0000",
  });
  assert.equal(columnLabel(en, 0, placeName).label, "col 0 · Name");
});

test("without a name a column shows its index alone", () => {
  assert.deepEqual(columnLabel(ru, 2, null), { label: "кол. 2", hint: undefined });
  assert.deepEqual(columnLabel(ru, 2, undefined), { label: "кол. 2", hint: undefined });
  assert.deepEqual(columnLabel(en, 7, placeName), { label: "col 7", hint: undefined });
});
