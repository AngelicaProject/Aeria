use aeria_se::{
    ComparisonOperator, DiagnosticKind, ExpressionKind, KnownMacro, MAX_NESTING_DEPTH, MacroNode,
    OpaquePayload, PlaceholderExpression, Safety, Span, SyntaxKind, UnaryExpression, parse,
};

const LUMINA_GOLDEN: &str = include_str!("fixtures/lumina_to_macro_string.golden.txt");
const PARSER_COMPATIBILITY: &str = include_str!("fixtures/parser_compatibility.txt");

#[test]
fn lumina_to_macro_string_golden_vectors_round_trip_structurally() {
    for (label, source) in vectors(LUMINA_GOLDEN) {
        let parsed = parse(source);
        assert_eq!(
            parsed.safety(),
            if matches!(
                label,
                "unsupported_macro_payload" | "raw_payload_fallback" | "opaque_expression_fallback"
            ) {
                Safety::Opaque
            } else {
                Safety::Understood
            },
            "golden vector {label}: {source}"
        );
        assert!(
            parsed.diagnostics().is_empty(),
            "golden vector {label}: {source}"
        );
        assert!(
            !parsed.nodes().is_empty(),
            "golden vector {label}: {source}"
        );
        assert_node_spans(&parsed, source);
        assert_eq!(
            parsed.serialize(),
            source,
            "golden vector {label}: {source}"
        );

        let reparsed = parse(&parsed.serialize());
        assert_eq!(
            reparsed.nodes(),
            parsed.nodes(),
            "golden vector {label}: {source}"
        );
    }
}

#[test]
fn parser_compatibility_vectors_are_separate_from_emitter_golden_vectors() {
    for (label, source) in vectors(PARSER_COMPATIBILITY) {
        let parsed = parse(source);
        assert_eq!(parsed.safety(), Safety::Understood, "compat vector {label}");
        assert!(parsed.diagnostics().is_empty(), "compat vector {label}");
        assert_node_spans(&parsed, source);

        let SyntaxKind::Macro(macro_node) = &parsed.nodes()[0].kind else {
            panic!("compat vector {label} should start with a known macro");
        };
        assert!(matches!(
            macro_node.arguments[0].kind,
            ExpressionKind::UnsignedInteger { .. }
        ));
    }
}

#[test]
fn exposes_nested_macro_and_expression_structure_and_spans() {
    let source = r"<if([lnum1>=t_hour],<italic(1)>yes,<italic(0)>no)>";
    let parsed = parse(source);
    assert_eq!(parsed.safety(), Safety::Understood);
    assert_eq!(parsed.nodes().len(), 1);
    assert_eq!(parsed.nodes()[0].span, Span::new(0, source.len()));
    assert_eq!(parsed.slice(parsed.nodes()[0].span), Some(source));

    let SyntaxKind::Macro(if_macro) = &parsed.nodes()[0].kind else {
        panic!("expected a macro node");
    };
    assert_eq!(if_macro.name, KnownMacro::If);
    assert_eq!(if_macro.arguments.len(), 3);

    let ExpressionKind::Binary {
        operator,
        left,
        right,
    } = &if_macro.arguments[0].kind
    else {
        panic!("expected a comparison expression");
    };
    assert_eq!(*operator, ComparisonOperator::GreaterThanOrEqual);
    assert_eq!(
        parsed.slice(if_macro.arguments[0].span),
        Some("[lnum1>=t_hour]")
    );
    assert_eq!(parsed.slice(left.span), Some("lnum1"));
    assert_eq!(parsed.slice(right.span), Some("t_hour"));
    assert!(matches!(
        left.kind,
        ExpressionKind::Unary {
            operator: UnaryExpression::LocalNumber,
            ..
        }
    ));
    assert_eq!(
        right.kind,
        ExpressionKind::Placeholder(PlaceholderExpression::Hour)
    );

    let ExpressionKind::String { parts } = &if_macro.arguments[1].kind else {
        panic!("expected a nested string expression");
    };
    assert_eq!(
        parsed.slice(if_macro.arguments[1].span),
        Some("<italic(1)>yes")
    );
    assert!(matches!(
        parts[0].kind,
        SyntaxKind::Macro(MacroNode {
            name: KnownMacro::Italic,
            ..
        })
    ));
    assert_eq!(parsed.slice(parts[0].span), Some("<italic(1)>"));
}

#[test]
fn classifies_lumina_numeric_and_native_expression_forms() {
    let source = "<num(0x1234_5678)><string(lnum1)><sec(t_min)>";
    let parsed = parse(source);
    assert_node_spans(&parsed, source);

    let SyntaxKind::Macro(num_macro) = &parsed.nodes()[0].kind else {
        panic!("expected a num macro");
    };
    assert_eq!(
        &num_macro.arguments[0].kind,
        &ExpressionKind::UnsignedInteger { value: 0x1234_5678 }
    );
    assert_eq!(
        parsed.slice(num_macro.arguments[0].span),
        Some("0x1234_5678")
    );

    let SyntaxKind::Macro(string_macro) = &parsed.nodes()[1].kind else {
        panic!("expected a string macro");
    };
    assert!(matches!(
        &string_macro.arguments[0].kind,
        ExpressionKind::Unary {
            operator: UnaryExpression::LocalNumber,
            operand
        } if operand.kind == ExpressionKind::UnsignedInteger { value: 1 }
    ));

    let SyntaxKind::Macro(sec_macro) = &parsed.nodes()[2].kind else {
        panic!("expected a sec macro");
    };
    assert_eq!(
        &sec_macro.arguments[0].kind,
        &ExpressionKind::Placeholder(PlaceholderExpression::Minute)
    );
}

#[test]
fn preserves_escape_nodes_and_their_source_spans() {
    let source = r"literal \<angle> and \\slash";
    let parsed = parse(source);
    assert_eq!(parsed.safety(), Safety::Understood);
    assert_eq!(parsed.nodes().len(), 5);
    assert!(matches!(
        parsed.nodes()[1].kind,
        SyntaxKind::Escape { character: '<' }
    ));
    assert!(matches!(
        parsed.nodes()[3].kind,
        SyntaxKind::Escape { character: '\\' }
    ));
    assert_eq!(parsed.slice(parsed.nodes()[0].span), Some("literal "));
    assert_eq!(parsed.slice(parsed.nodes()[1].span), Some(r"\<"));
    assert_eq!(parsed.slice(parsed.nodes()[3].span), Some(r"\\"));
    assert_eq!(parsed.serialize(), source);
}

#[test]
fn preserves_wider_string_expression_escapes() {
    let source = r"<string(foo\,bar\<baz\>\[x\]\(y\)\\)>";
    let parsed = parse(source);
    assert_eq!(parsed.safety(), Safety::Understood);
    let SyntaxKind::Macro(macro_node) = &parsed.nodes()[0].kind else {
        panic!("expected a macro node");
    };
    let ExpressionKind::String { parts } = &macro_node.arguments[0].kind else {
        panic!("expected a string expression");
    };
    assert_eq!(
        parts
            .iter()
            .filter(|node| matches!(node.kind, SyntaxKind::Escape { .. }))
            .count(),
        8
    );
    assert_eq!(
        parsed.slice(macro_node.arguments[0].span),
        Some(&source[8..source.len() - 2])
    );
    assert_eq!(parsed.serialize(), source);
}

#[test]
fn preserves_unknown_named_macros_as_opaque_without_semantic_diagnostics() {
    let cases = [
        ("<futuremacro>", "futuremacro", 0),
        ("<FutureMacro(1,text)>", "FutureMacro", 2),
        ("<outer(<inner(1)>)>", "outer", 1),
    ];

    for (source, expected_name, expected_arguments) in cases {
        let parsed = parse(source);
        assert_eq!(parsed.safety(), Safety::Opaque, "source: {source}");
        assert!(parsed.diagnostics().is_empty(), "source: {source}");
        assert_eq!(parsed.serialize(), source);

        let node = &parsed.nodes()[0];
        assert_eq!(node.span, Span::new(0, source.len()));
        assert_eq!(parsed.slice(node.span), Some(source));
        let SyntaxKind::Opaque(OpaquePayload::NamedMacro { name, arguments }) = &node.kind else {
            panic!("expected an opaque named macro: {source}");
        };
        assert_eq!(name, expected_name);
        assert_eq!(arguments.len(), expected_arguments);
        if expected_arguments == 2 {
            assert_eq!(parsed.slice(arguments[0].span), Some("1"));
            assert_eq!(parsed.slice(arguments[1].span), Some("text"));
        }
    }

    let nested = parse(cases[2].0);
    let SyntaxKind::Opaque(OpaquePayload::NamedMacro { arguments, .. }) = &nested.nodes()[0].kind
    else {
        panic!("expected an opaque outer macro");
    };
    let ExpressionKind::String { parts } = &arguments[0].kind else {
        panic!("expected the nested macro argument to be a string expression");
    };
    let SyntaxKind::Opaque(OpaquePayload::NamedMacro {
        name,
        arguments: inner_arguments,
    }) = &parts[0].kind
    else {
        panic!("expected an opaque inner macro");
    };
    assert_eq!(name, "inner");
    assert_eq!(inner_arguments.len(), 1);
    assert_eq!(nested.slice(arguments[0].span), Some("<inner(1)>"));
    assert_eq!(nested.slice(parts[0].span), Some("<inner(1)>"));
    assert_eq!(nested.slice(inner_arguments[0].span), Some("1"));
}

#[test]
fn unknown_named_macro_with_malformed_arguments_remains_malformed() {
    let source = "<futuremacro(1,2>";
    let parsed = parse(source);
    assert_eq!(parsed.safety(), Safety::Malformed);
    assert!(!parsed.diagnostics().is_empty());
    assert_eq!(
        parsed.diagnostics()[0].kind,
        DiagnosticKind::InvalidDelimiter
    );
    assert_eq!(parsed.serialize(), source);
}

#[test]
fn classifies_opaque_fallbacks_without_diagnostics() {
    let source = "<payload:CF(1,<expr: 01 02>)><payload: 00 FF>";
    let parsed = parse(source);
    assert_eq!(parsed.safety(), Safety::Opaque);
    assert!(parsed.diagnostics().is_empty());
    assert_eq!(parsed.serialize(), source);
    assert_node_spans(&parsed, source);

    let SyntaxKind::Opaque(OpaquePayload::Macro { code, arguments }) = &parsed.nodes()[0].kind
    else {
        panic!("expected an opaque macro payload");
    };
    assert_eq!(*code, 0xCF);
    assert!(matches!(arguments[1].kind, ExpressionKind::Opaque { .. }));
    assert!(matches!(
        parsed.nodes()[1].kind,
        SyntaxKind::Opaque(OpaquePayload::Raw { .. })
    ));
}

#[test]
fn malformed_input_is_recoverable_and_lossless() {
    for source in [
        "<>",
        "<bad_payload(1,2>",
        "<if(1,2,3>",
        "<if(1,2,3",
        "<if([1=2],yes,no)>",
        "<payload:ZZ>",
        "<if(1,\\)",
        "trailing\\",
    ] {
        let parsed = parse(source);
        assert_eq!(parsed.safety(), Safety::Malformed, "source: {source}");
        assert!(!parsed.diagnostics().is_empty(), "source: {source}");
        assert_eq!(parsed.serialize(), source, "source: {source}");
        assert!(parsed.diagnostics().iter().all(|diagnostic| {
            diagnostic.span.start() <= diagnostic.span.end()
                && diagnostic.span.end() <= source.len()
        }));
    }
    assert_eq!(
        parse("<>").diagnostics()[0].kind,
        DiagnosticKind::InvalidDelimiter
    );
}

#[test]
fn deep_nesting_hits_a_bounded_diagnostic() {
    let mut source = String::new();
    for _ in 0..(MAX_NESTING_DEPTH + 8) {
        source.push_str("<string(");
    }
    source.push('x');
    for _ in 0..(MAX_NESTING_DEPTH + 8) {
        source.push_str(")>");
    }

    let parsed = parse(&source);
    assert_eq!(parsed.safety(), Safety::Malformed);
    assert!(
        parsed
            .diagnostics()
            .iter()
            .any(|diagnostic| { diagnostic.kind == DiagnosticKind::NestingLimit })
    );
    assert_eq!(parsed.serialize(), source);
}

fn vectors(corpus: &str) -> impl Iterator<Item = (&str, &str)> {
    corpus.lines().filter_map(|line| {
        let line = line.trim_end_matches('\r');
        if line.is_empty() || line.starts_with('#') {
            return None;
        }
        Some(
            line.split_once('\t')
                .expect("fixture line must contain a tab"),
        )
    })
}

fn assert_node_spans(parsed: &aeria_se::MacroString, source: &str) {
    for node in parsed.nodes() {
        assert!(node.span.start() <= node.span.end());
        assert!(node.span.end() <= source.len());
        assert_eq!(
            parsed.slice(node.span),
            Some(&source[node.span.start()..node.span.end()])
        );
    }
}

#[test]
fn golden_lumina_output_encodes_and_decodes_back_to_itself() {
    // Fallback forms are printed by Lumina but cannot be parsed back.
    const UNPARSEABLE: &[&str] = &[
        "unsupported_macro_payload",
        "raw_payload_fallback",
        "opaque_expression_fallback",
    ];
    let golden = include_str!("fixtures/lumina_to_macro_string.golden.txt");
    for line in golden.lines().filter(|line| !line.starts_with('#')) {
        let (name, text) = line.split_once('\t').expect("vector");
        let encoded = aeria_se::codec::encode(text);
        if UNPARSEABLE.contains(&name) {
            assert!(encoded.is_err(), "{name}");
            continue;
        }
        let bytes = encoded.unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(aeria_se::codec::decode(&bytes), text, "{name}");
        assert_eq!(aeria_se::codec::encode_checked(text), Ok(bytes), "{name}");
    }
}
