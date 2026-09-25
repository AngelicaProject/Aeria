# Structured game strings

Game strings are not plain prose. They may contain nested macros, conditions, formatting, runtime values, game references, and other engine-readable constructs.

Aeria must never treat these constructs as disposable decoration.

## Source representation

HXS provides an encodeable macro-string representation and raw source bytes. `aeria-se` parses the macro representation into a lossless syntax representation suitable for editing and validation.

## Syntax-layer contract

The first `aeria-se` slice is an owned concrete syntax tree (CST), not the
future semantic editing or preview AST. Root and nested string-expression
nodes retain byte spans into the original source. The tree exposes ordinary
text, escapes, known Lumina macro names, ordered expressions, unsigned integer
values, nested strings, native placeholders, unary expressions, comparison
expressions, opaque named macros, and recovery nodes for malformed input.

The parser follows the encodeable representation emitted by the Lumina 7.7.0
`ToMacroString()` implementation used by Harmonia Atlas. It has no runtime
dependency on Lumina or .NET. The native macro and expression name tables are
owned by `aeria-se` and must be reviewed with the corresponding upstream
source and conformance corpus when Atlas changes its Lumina version. The
checked-in golden vectors in `crates/aeria-se/tests/fixtures/lumina_to_macro_string.golden.txt`
are fixed output from synthetic Lumina 7.7.0 `ReadOnlySeString` values; the
separate `parser_compatibility.txt` corpus covers accepted spellings that the
emitter does not produce.

Serialization is source-preserving:

```text
parse(source).serialize() == source
```

This invariant applies to understood syntax and to Lumina fallback forms that
are opaque but losslessly preservable. Opaque payloads, expression fallbacks,
and syntactically valid named macros that are unknown to the Lumina 7.7.0
table are exposed as protected nodes and do not receive guessed semantic
meaning. Unknown names alone do not produce diagnostics; invalid delimiters or
arguments still do. The document retains the original source even for
malformed input, but malformed documents are unsafe for editing/export and
carry structured diagnostics with byte spans, kinds, and messages.

`serialize()` currently returns the owned source buffer. Mutation-aware
structural serialization is future work; this slice does not claim to rewrite
syntax that it cannot model.

The aggregate safety state distinguishes understood, opaque, and malformed
documents. Recovery nodes keep malformed delimiters, escapes, and tails
visible to inspection. Parsing is bounded by a nesting limit of 128 parser
calls; exceeding it produces a diagnostic and preserves the remaining source
as malformed rather than recursing without limit.

## Safety model

A parsed construct is one of three broad safety classes:

- **Known**: Aeria understands the construct and can expose typed editing/preview behavior.
- **Opaque but preservable**: Aeria does not understand the semantics, but can preserve the construct losslessly. The surrounding text remains editable and the opaque node is protected.
- **Unparseable/unsafe**: Aeria cannot prove preservation. Editing/export for that unit is blocked with a diagnostic rather than risking silent corruption.

Unknown future constructs should not make an entire project unusable when lossless preservation is possible.

## Editor modes

The product should grow three complementary views:

- Visual editor with structured macro tokens/controls.
- Raw macro editor for power users, always parsed and validated before save.
- Semantic preview that progressively approaches in-game fidelity.

The initial preview prioritizes meaning: nesting, colors, branches, runtime values, and selectable conditional outcomes. Pixel-perfect game rendering is an incremental goal.

## AI boundary

AI receives structured translatable content with protected placeholders and never writes raw macro syntax. Structural edits beyond the policy below may later be exposed as explicit validated operations rather than unrestricted mutation of raw syntax.

### Tagged text

`aeria_se::project` turns a well-formed source string into tagged text. Prose
is plain text with `&`, `<`, and `>` written as `&amp;`, `&lt;`, and `&gt;`.
Each root or nested protected construct becomes a numbered tag in source
order:

- `<x id="N"/>` for a construct without translatable content, including
  opaque constructs;
- `<g id="N"><b>…</b>…</g>` for a known macro whose user-facing string
  arguments are translated in place, one `<b>` per such argument, for example
  the branches of `<if(…)>`, `<switch(…)>`, or `<ifpcgender(…)>`, or the text
  of `<string(…)>`.

Each tag has a legend entry with its exact source spelling, its semantic
family, its branch count, and whether it may repeat. A malformed source has
no projection and is not offered for assisted translation.

`aeria_se::rebuild(source, tagged)` parses a tagged translation, checks it,
and rebuilds the target by copying each construct's exact source spelling
and splicing translated branch text into the source arguments. Prose is
escaped for its context: `\` and `<` everywhere, and also `,`, `(`, `)`,
`[`, `]`, and `>` inside arguments.

### Assisted structure policy

A tagged translation and, independently, the rebuilt syntax tree
(`aeria_se::check_assisted_structure`) must satisfy:

- every source construct is kept; no construct kind is droppable yet;
- a construct stays in its container, the top level or one branch of one
  construct;
- constructs may move within their container, except that formatting
  constructs keep their relative order, so start and end pairs cannot cross;
- only runtime values without branches, such as the player's name, may
  repeat;
- constructs with branches keep their number of branches;
- nothing else may be added. A changed game reference, parameter, argument,
  or opaque construct is a different construct and is rejected, as is a
  branch whose translation would parse as a number or runtime value.

The tree check compares constructs by their protected structure without
spans or branch prose, then compares the branches of matching constructs
recursively. Refusals are returned as messages written for the model, so it
can correct its translation. This policy, not strict structure comparison,
is the acceptance rule for assisted translation.

## Semantic analysis

`aeria-se` derives a semantic projection from the lossless CST. The CST remains the source-preserving syntax layer: semantic nodes retain CST spans and do not replace the original representation or execute expressions. Known Lumina 7.7.0 macros receive only the broad classification supported by the upstream contract; entries whose semantics are not established are intentionally classified as opaque protected constructs. Unknown named macros and fallback payloads remain opaque protected constructs as well.

Intrinsic validity is separate from source/target structure compatibility. A malformed CST is always invalid and blocks semantic editing/export. A well-formed document containing opaque constructs is valid with protected data. Strict structure comparison is a conservative safety mechanism for assisted or AI translation: it compares the ordered protected macro, expression, runtime, game-reference, and opaque structure while ignoring ordinary translatable prose. User-facing text and escapes are retained in that projection as normalized text slots, so prose may change while a protected macro cannot silently move across a text boundary. The same slots are used inside user-facing string expressions and conditional branches. Numeric identifiers in known game-data reference arguments are compared as game references alongside sheet-name and other protected lookup inputs. It does not make identical structure a general validity requirement. Manual structural editing may intentionally add, remove, or modify macros later through explicit validated operations.
