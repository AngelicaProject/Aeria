use aeria_se::{
    KnownMacro, SemanticFamily, SemanticValidity, StructureCompatibility, StructureDifferenceKind,
    TextRangeKind, analyze, compare_macro_strings, parse,
};

fn has_difference(source: &str, target: &str, kind: StructureDifferenceKind) -> bool {
    compare_macro_strings(source, target)
        .differences
        .iter()
        .any(|difference| difference.kind == kind)
}

#[test]
fn every_lumina_macro_has_an_exhaustive_semantic_classification() {
    assert_eq!(KnownMacro::ALL.len(), 57);
    for macro_name in KnownMacro::ALL {
        assert!(matches!(
            macro_name.semantic_family(),
            SemanticFamily::TranslatableText
                | SemanticFamily::FormattingPresentation
                | SemanticFamily::ConditionalSelection
                | SemanticFamily::RuntimeContextValue
                | SemanticFamily::GameDataReference
                | SemanticFamily::LayoutTextualControl
                | SemanticFamily::OpaqueProtected
        ));
    }

    for macro_name in [
        KnownMacro::Key,
        KnownMacro::Scale,
        KnownMacro::Edge,
        KnownMacro::Shadow,
        KnownMacro::Icon2,
        KnownMacro::Time,
        KnownMacro::Split,
        KnownMacro::Fixed,
    ] {
        assert_eq!(
            macro_name.semantic_family(),
            SemanticFamily::OpaqueProtected
        );
    }
}

#[test]
fn classifies_representative_lumina_families() {
    assert_eq!(
        KnownMacro::If.semantic_family(),
        SemanticFamily::ConditionalSelection
    );
    assert_eq!(
        KnownMacro::Switch.semantic_family(),
        SemanticFamily::ConditionalSelection
    );
    assert_eq!(
        KnownMacro::Bold.semantic_family(),
        SemanticFamily::FormattingPresentation
    );
    assert_eq!(
        KnownMacro::Sheet.semantic_family(),
        SemanticFamily::GameDataReference
    );
    assert_eq!(
        KnownMacro::SwitchPlatform.semantic_family(),
        SemanticFamily::GameDataReference
    );
    assert_eq!(
        KnownMacro::PcName.semantic_family(),
        SemanticFamily::RuntimeContextValue
    );
    assert_eq!(
        KnownMacro::NewLine.semantic_family(),
        SemanticFamily::LayoutTextualControl
    );
    assert_eq!(
        KnownMacro::String.semantic_family(),
        SemanticFamily::TranslatableText
    );
}

#[test]
fn extracts_plain_and_nested_translatable_text_deterministically() {
    let source = r"before <if([1==2],<bold(1)>yes\,there,<italic(1)>no)> after";
    let parsed = parse(source);
    let analysis = analyze(&parsed);
    let ranges = analysis.text_ranges();

    assert_eq!(
        ranges
            .iter()
            .map(|range| parsed.slice(range.span).unwrap())
            .collect::<Vec<_>>(),
        vec!["before ", "yes", r"\,", "there", "no", " after"]
    );
    assert_eq!(
        ranges.iter().map(|range| range.kind).collect::<Vec<_>>(),
        vec![
            TextRangeKind::Text,
            TextRangeKind::Text,
            TextRangeKind::Escape,
            TextRangeKind::Text,
            TextRangeKind::Text,
            TextRangeKind::Text,
        ]
    );
}

#[test]
fn validation_distinguishes_understood_opaque_and_unsafe_documents() {
    assert_eq!(
        analyze(&parse("plain text")).validation().status(),
        SemanticValidity::ValidAndUnderstood
    );
    assert_eq!(
        analyze(&parse("<futuremacro(1)>text"))
            .validation()
            .status(),
        SemanticValidity::ValidWithOpaque
    );
    assert_eq!(
        analyze(&parse("<fixed(1)>text")).validation().status(),
        SemanticValidity::ValidWithOpaque
    );

    let malformed = analyze(&parse("<if(1,2,3>"));
    assert_eq!(
        malformed.validation().status(),
        SemanticValidity::InvalidUnsafe
    );
    assert!(!malformed.validation().diagnostics().is_empty());
    assert_eq!(malformed.validation().diagnostics()[0].span.start(), 9);
}

#[test]
fn text_extraction_excludes_game_reference_and_opaque_payload_text() {
    let parsed = parse("<sheet(Items,1,2,parameter)><futuremacro(hidden)>visible");
    let analysis = analyze(&parsed);
    assert_eq!(
        analysis
            .text_ranges()
            .iter()
            .map(|range| parsed.slice(range.span).unwrap())
            .collect::<Vec<_>>(),
        vec!["visible"]
    );
}

#[test]
fn opaque_fallbacks_are_valid_protected_structure() {
    let analysis = analyze(&parse(
        "<payload:35><if(<expr: 0xD0 is unsupported>,yes,no)>",
    ));
    assert_eq!(
        analysis.validation().status(),
        SemanticValidity::ValidWithOpaque
    );
    assert_eq!(analysis.structure().nodes().len(), 2);
    assert_eq!(
        compare_macro_strings("<payload:35>", "<payload:36>").compatibility,
        StructureCompatibility::Incompatible
    );
    assert!(has_difference(
        "<if(<expr: 0xD0 is unsupported>,yes,no)>",
        "<if(<expr: 0xD1 is unsupported>,yes,no)>",
        StructureDifferenceKind::ChangedOpaqueConstruct
    ));
}

#[test]
fn text_only_translation_changes_remain_compatible() {
    let comparison = compare_macro_strings(
        "<bold(1)>Hello <if([1==2],<italic(1)>yes,no)>",
        "<bold(1)>Bonjour <if([1==2],<italic(1)>oui,non)>",
    );
    assert_eq!(comparison.compatibility, StructureCompatibility::Compatible);
    assert!(comparison.differences.is_empty());
}

#[test]
fn pure_text_replacement_remains_compatible() {
    let comparison = compare_macro_strings("Hello", "Bonjour");
    assert_eq!(comparison.compatibility, StructureCompatibility::Compatible);
    assert!(comparison.differences.is_empty());
}

#[test]
fn formatting_macros_cannot_cross_a_text_slot() {
    for (source, target) in [
        ("<bold(1)>Hello", "Hello<bold(1)>"),
        ("<bold(1)>Hello<bold(0)>", "Hello<bold(1)><bold(0)>"),
    ] {
        let comparison = compare_macro_strings(source, target);
        assert_eq!(
            comparison.compatibility,
            StructureCompatibility::Incompatible,
            "unexpected comparison for {source:?} -> {target:?}: {:?}",
            comparison.differences
        );
        assert!(
            comparison.differences.iter().any(|difference| {
                matches!(
                    difference.kind,
                    StructureDifferenceKind::ReorderedProtectedNodes
                        | StructureDifferenceKind::DifferentProtectedKind
                )
            }),
            "expected text-slot movement difference for {source:?} -> {target:?}"
        );
    }
}

#[test]
fn text_slots_are_preserved_inside_branches_and_string_expressions() {
    for (source, target) in [
        (
            "<if([1==2],<bold(1)>yes<bold(0)>,fallback)>",
            "<if([1==2],yes<bold(1)><bold(0)>,fallback)>",
        ),
        (
            "<string(<bold(1)>yes<bold(0)>)>",
            "<string(yes<bold(1)><bold(0)>)>",
        ),
    ] {
        let comparison = compare_macro_strings(source, target);
        assert_eq!(
            comparison.compatibility,
            StructureCompatibility::Incompatible,
            "unexpected comparison for {source:?} -> {target:?}: {:?}",
            comparison.differences
        );
        assert!(!comparison.differences.is_empty());
    }
}

#[test]
fn comparison_reports_missing_and_reordered_protected_nodes() {
    let missing = compare_macro_strings("<bold(1)>hello", "hello");
    assert_eq!(missing.compatibility, StructureCompatibility::Incompatible);
    assert!(
        missing
            .differences
            .iter()
            .any(|difference| difference.kind == StructureDifferenceKind::MissingProtectedNode)
    );

    let reordered = compare_macro_strings("<bold(1)><italic(1)>hello", "<italic(1)><bold(1)>hello");
    assert!(
        reordered
            .differences
            .iter()
            .any(|difference| difference.kind == StructureDifferenceKind::ReorderedProtectedNodes)
    );
}

#[test]
fn comparison_reports_runtime_game_and_opaque_changes() {
    assert!(has_difference(
        "<if([lnum1>=t_hour],yes,no)>",
        "<if([lnum2>=t_hour],yes,no)>",
        StructureDifferenceKind::ChangedRuntimeExpression
    ));
    assert!(has_difference(
        "<sheet(Items,1,2,foo)>",
        "<sheet(Quests,1,2,foo)>",
        StructureDifferenceKind::ChangedGameReference
    ));
    assert!(has_difference(
        "<sheet(Item,1,2,foo)>",
        "<sheet(Item,2,2,foo)>",
        StructureDifferenceKind::ChangedGameReference
    ));
    assert!(has_difference(
        "<futuremacro(1)>",
        "<futuremacro(2)>",
        StructureDifferenceKind::ChangedOpaqueConstruct
    ));
    assert!(has_difference(
        "<if(foo,yes,no)>",
        "<if(bar,yes,no)>",
        StructureDifferenceKind::ChangedExpressionStructure
    ));
}

#[test]
fn malformed_documents_cannot_be_compared_safely() {
    let comparison = compare_macro_strings("<if(1,2,3>", "<if(1,2,3)");
    assert_eq!(
        comparison.compatibility,
        StructureCompatibility::CannotSafelyCompare
    );
    assert!(
        comparison
            .differences
            .iter()
            .any(|difference| difference.kind == StructureDifferenceKind::UnsafeInput)
    );
}

#[test]
fn formatting_only_documents_have_no_letters_in_user_facing_text() {
    for source in [
        "",
        "...",
        "…",
        ": ",
        "：",
        "0",
        "&",
        "<nbsp>",
        r"<kilo(lnum1,\,)>",
        "<if([lnum1>=t_hour],<italic(1)>1,<italic(0)>2)>",
    ] {
        assert!(
            aeria_se::parse(source).is_formatting_only(),
            "{source:?} should be formatting only"
        );
    }
    for source in [
        "Hello",
        "é",
        "未使用",
        "<if([lnum1>=t_hour],<italic(1)>yes,<italic(0)>no)>",
        "<if(1,2,3",
    ] {
        assert!(
            !aeria_se::parse(source).is_formatting_only(),
            "{source:?} should not be formatting only"
        );
    }
}
