use aeria_se::catalog::{Form, MACROS};
use aeria_se::codec::{decode, encode};
use aeria_se::{DiagnosticKind, ExprKind, SyntaxKind, Written, parse};

const GOLDEN: &str = include_str!("fixtures/macro_text.golden.txt");

fn vectors() -> impl Iterator<Item = (&'static str, Vec<u8>, &'static str)> {
    GOLDEN
        .lines()
        .filter(|line| !line.starts_with('#') && !line.trim().is_empty())
        .map(|line| {
            let mut parts = line.split('\t');
            let name = parts.next().expect("name");
            let hex = parts.next().expect("hex");
            let text = parts.next().expect("text");
            let bytes = (0..hex.len())
                .step_by(2)
                .map(|index| u8::from_str_radix(&hex[index..index + 2], 16).expect("hex"))
                .collect();
            (name, bytes, text)
        })
}

#[test]
fn golden_vectors_decode_and_encode_exactly() {
    for (name, bytes, text) in vectors() {
        assert_eq!(decode(&bytes), text, "{name}");
        assert_eq!(encode(text).as_deref(), Ok(bytes.as_slice()), "{name}");
        assert!(parse(text).diagnostics().is_empty(), "{name}");
    }
}

#[test]
fn every_catalog_entry_has_a_golden_vector() {
    let texts: Vec<&str> = vectors().map(|(_, _, text)| text).collect();
    for spec in MACROS {
        let open = format!("<{}", spec.name);
        let close = format!("</{}>", spec.name);
        let used = texts.iter().any(|text| {
            text.match_indices(&open).any(|(index, _)| {
                text[index + open.len()..]
                    .chars()
                    .next()
                    .is_some_and(|next| matches!(next, ' ' | '>'))
            })
        });
        assert!(used, "no vector opens <{}>", spec.name);
        if matches!(spec.form, Form::Pair { .. } | Form::Block { .. }) {
            assert!(
                texts.iter().any(|text| text.contains(&close)),
                "no vector closes </{}>",
                spec.name
            );
        }
    }
}

#[test]
fn spacing_inside_tags_is_free_and_prints_canonically() {
    let bytes = encode("<if  (  $n1   ==  1 )  >a</if><sheet   Item  $n1  0 >").expect("encode");
    assert_eq!(decode(&bytes), "<if ($n1 == 1)>a</if><sheet Item $n1 0>");
    assert_eq!(
        encode("<num #1F>").expect("hex"),
        encode("<num 31>").expect("decimal")
    );
    assert_eq!(
        encode("<sheet \"Item\" 1 0>").expect("quoted"),
        encode("<sheet Item 1 0>").expect("bare")
    );
}

#[test]
fn text_in_branches_stays_text_even_when_it_looks_like_a_number() {
    let text = "<if $n1>1<else>{1}</if>";
    let document = parse(text);
    let SyntaxKind::Macro(syntax) = &document.nodes()[0].kind else {
        panic!("a macro");
    };
    assert!(matches!(syntax.args[1].kind, ExprKind::Str(_)));
    assert!(matches!(syntax.args[2].kind, ExprKind::Int(1)));
    assert_eq!(decode(&encode(text).expect("encode")), text);
}

#[test]
fn block_content_spans_cover_exactly_the_branch_text() {
    let text = "x<if ($n1 == 1)>a <b>b</b><else>c</if>y";
    let document = parse(text);
    let SyntaxKind::Macro(syntax) = &document.nodes()[1].kind else {
        panic!("a macro");
    };
    assert_eq!(syntax.written, Written::Block);
    assert_eq!(document.slice(syntax.args[1].span), Some("a <b>b</b>"));
    assert_eq!(document.slice(syntax.args[2].span), Some("c"));
    assert_eq!(
        document.slice(document.nodes()[1].span),
        Some("<if ($n1 == 1)>a <b>b</b><else>c</if>")
    );
}

fn first_error(text: &str) -> (DiagnosticKind, String) {
    let document = parse(text);
    let diagnostic = document
        .diagnostics()
        .first()
        .unwrap_or_else(|| panic!("{text} has no diagnostic"));
    (diagnostic.kind, diagnostic.message.clone())
}

#[test]
fn mistakes_are_reported_with_a_way_to_fix_them() {
    let (kind, message) = first_error("<colour #FF0000FF>x</color>");
    assert_eq!(kind, DiagnosticKind::UnknownMacro);
    assert!(message.contains("did you mean <color>"), "{message}");

    let (kind, message) = first_error("<if $n1>a");
    assert_eq!(kind, DiagnosticKind::UnexpectedEof);
    assert!(message.contains("</if>"), "{message}");

    let (_, message) = first_error("a</if>");
    assert!(message.contains("does not close"), "{message}");

    let (_, message) = first_error("a<else>b");
    assert!(message.contains("not valid here"), "{message}");

    let (_, message) = first_error("1 < 2");
    assert!(message.contains("\\<"), "{message}");

    let (_, message) = first_error("{x}");
    assert!(message.contains("\\{"), "{message}");

    let (kind, _) = first_error("a\\b");
    assert_eq!(kind, DiagnosticKind::InvalidEscape);

    let (kind, message) = first_error("<sheet Item>");
    assert_eq!(kind, DiagnosticKind::InvalidArguments);
    assert!(message.contains("sheet, row, column"), "{message}");

    let (_, message) = first_error("<if $n1>a<else>b<else>c</if>");
    assert!(message.contains("branch"), "{message}");

    let (_, message) = first_error("<num $n1$n2>");
    assert!(message.contains("spaces"), "{message}");

    let (_, message) = first_error("<if $n1 == 1>a</if>");
    assert!(message.contains("value"), "{message}");

    let (_, message) = first_error("<num $level>");
    assert!(message.contains("$n1"), "{message}");

    let (kind, _) = first_error("<num 4294967296>");
    assert_eq!(kind, DiagnosticKind::InvalidValue);

    let (kind, _) = first_error("<br x>");
    assert_eq!(kind, DiagnosticKind::InvalidArguments);

    let (kind, _) = first_error("<switch $n1>x<case>a</switch>");
    assert_eq!(kind, DiagnosticKind::InvalidDelimiter);

    let (kind, message) = first_error("a<raw 02 FF>b");
    assert_eq!(kind, DiagnosticKind::RawBytes);
    assert!(message.contains("remove"), "{message}");
    assert!(encode("a<raw 02 FF>b").is_err());
}

#[test]
fn errors_recover_so_later_text_still_parses() {
    let document = parse("<nope> then <i>x</i>");
    assert_eq!(document.diagnostics().len(), 1);
    assert!(document.nodes().iter().any(|node| matches!(
        &node.kind,
        SyntaxKind::Macro(syntax) if syntax.written == Written::Close
    )));
}

#[test]
fn nesting_is_bounded() {
    let deep = "<upper>".repeat(200) + &"</upper>".repeat(200);
    let document = parse(&deep);
    assert!(
        document
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.kind == DiagnosticKind::NestingLimit)
    );
    let fine = "<upper>".repeat(60) + "x" + &"</upper>".repeat(60);
    assert!(parse(&fine).diagnostics().is_empty());
}
