# Structured game strings

Game strings are not plain prose. They may contain nested macros, conditions, formatting, runtime values, game references, and other engine-readable constructs.

Aeria must never treat these constructs as disposable decoration.

## Source representation

HXS provides an encodeable macro-string representation and raw source bytes. `aeria-se` parses the macro representation into a lossless syntax representation suitable for editing and validation.

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

AI should receive structured translatable content and typed/protected placeholders wherever practical. The default operation translates text nodes and reconstructs the syntax tree in Rust. Structural edits may later be exposed as explicit validated operations rather than unrestricted mutation of raw syntax.
