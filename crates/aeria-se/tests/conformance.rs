use aeria_se::{
    ComparisonOperator, DiagnosticKind, ExpressionKind, KnownMacro, MAX_NESTING_DEPTH, MacroNode,
    OpaquePayload, PlaceholderExpression, Safety, SyntaxKind, UnaryExpression, parse,
};

const LUMINA_CORPUS: &str = include_str!("fixtures/lumina_macro_strings.txt");

#[test]
fn lumina_corpus_round_trips_without_diagnostics() {
    for (line_number, source) in LUMINA_CORPUS.lines().enumerate() {
        let parsed = parse(source);
        let expected_safety = if source.contains("<payload:") || source.contains("<expr:") {
            Safety::Opaque
        } else {
            Safety::Understood
        };
        assert_eq!(
            parsed.safety(),
            expected_safety,
            "fixture line {line_number}: {source}"
        );
        assert!(
            parsed.diagnostics().is_empty(),
            "fixture line {line_number}: {source}"
        );
        assert_eq!(
            parsed.serialize(),
            source,
            "fixture line {line_number}: {source}"
        );
        let reparsed = parse(&parsed.serialize());
        assert_eq!(
            reparsed.nodes(),
            parsed.nodes(),
            "fixture line {line_number}: {source}"
        );
    }
}

#[test]
fn exposes_nested_macro_and_expression_structure() {
    let source = r"<if([lnum1>=t_hour],<italic(1)>yes,<italic(0)>no)>";
    let parsed = parse(source);
    assert_eq!(parsed.safety(), Safety::Understood);
    assert_eq!(parsed.nodes().len(), 1);

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
    assert!(matches!(
        parts[0].kind,
        SyntaxKind::Macro(MacroNode {
            name: KnownMacro::Italic,
            ..
        })
    ));
}

#[test]
fn classifies_lumina_numeric_and_native_expression_forms() {
    let parsed = parse("<num(0x1234_5678)><string(lnum1)><sec(t_min)>");
    let SyntaxKind::Macro(num_macro) = &parsed.nodes()[0].kind else {
        panic!("expected a num macro");
    };
    assert_eq!(
        &num_macro.arguments[0].kind,
        &ExpressionKind::UnsignedInteger { value: 0x1234_5678 }
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
    assert_eq!(parsed.slice(parsed.nodes()[1].span), Some(r"\<"));
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
    assert_eq!(parsed.serialize(), source);
}

#[test]
fn classifies_opaque_fallbacks_without_diagnostics() {
    let parsed = parse("<payload:CF(1,<expr: 01 02>)><payload: 00 FF>");
    assert_eq!(parsed.safety(), Safety::Opaque);
    assert!(parsed.diagnostics().is_empty());
    assert_eq!(
        parsed.serialize(),
        "<payload:CF(1,<expr: 01 02>)><payload: 00 FF>"
    );

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
        "<bad_payload>",
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
        parse("<bad_payload>").diagnostics()[0].kind,
        DiagnosticKind::UnknownMacro
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
