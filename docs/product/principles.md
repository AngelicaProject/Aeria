# Product principles

1. **Preserve before guessing.** Translation integrity across game updates is more important than aggressive automation.
2. **Use existing collaboration infrastructure.** Aeria builds on Git and existing repository hosting instead of replacing them.
3. **One project, one translation.** The product is optimized for depth and scale within one target language, not a matrix of hundreds of locales.
4. **Structured text is data.** Game macros and runtime constructs are part of the source semantics and must be parsed, visualized, validated, and preserved deliberately.
5. **AI produces drafts, people approve translations.** Automation should remove bulk work without taking review authority away from people.
6. **No Aeria service is required.** A user can work from a local project and local source data without an Aeria account or hosted backend.
7. **Simple and advanced workflows share the same project state.** Friendly operations and power-user Git tooling work on the same repository.
8. **Failures are visible.** Unknown constructs, ambiguous migrations, and invalid AI responses become explicit work instead of silent corruption.
9. **Formats are contracts.** Public workspace and export formats are versioned and migrated without data loss.
10. **Documentation stays current.** Architecture and development documentation evolve with the code so future work begins from current truth.
