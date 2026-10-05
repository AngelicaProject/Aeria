# Product principles

1. **Preserve before guessing.** Translation integrity across game updates is more important than aggressive automation.
2. **Use existing collaboration infrastructure.** Aeria builds on Git and existing repository hosting instead of replacing them.
3. **One project, one translation.** The product is optimized for depth and scale within one target language, not a matrix of hundreds of locales.
4. **Structured text is data.** Game macros and runtime constructs are part of the source semantics and must be parsed, visualized, validated, and preserved deliberately.
5. **Machines translate, people decide.** Machine translation is a tool a person runs: it fills untranslated strings, and strings whose source changed only when asked, and every translation it writes passes the same checks as a person's. It never replaces a translation a person reviewed, and every change stays visible and reversible through the project's history.
6. **No Aeria service is required.** A user can work from a local project and local source data without an Aeria account or hosted backend.
7. **Simple and advanced workflows share the same project state.** Friendly operations and power-user Git tooling work on the same repository.
8. **Failures are visible.** Unknown constructs, changed sources, and translations the checks refuse become explicit work instead of silent corruption.
9. **Formats are contracts.** Project and export formats are versioned and migrated without data loss.
10. **Documentation stays current.** Architecture and development documentation evolve with the code so future work begins from current truth.
11. **A person sees what the model is told.** What a machine translation request says about a string (its names and terms, where each comes from, its speaker, its length, its macros and what a translation may do with them) the editor's string guide shows the person translating that string, read by the same code. A person can always tell where a machine translation took a name, a term, or a form from, and judge the string on the same grounds.
