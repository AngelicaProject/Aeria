use aeria_se::{SemanticValidity, parse};

#[test]
fn validity_separates_understood_opaque_and_invalid_text() {
    for (text, expected) in [
        ("plain <i>text</i>", SemanticValidity::ValidAndUnderstood),
        (
            "<if ($n1 == 1)>a<else>b</if>",
            SemanticValidity::ValidAndUnderstood,
        ),
        ("<code:35 1>", SemanticValidity::ValidWithOpaque),
        (
            "<split \" \" 1><string $gs1></split>",
            SemanticValidity::ValidWithOpaque,
        ),
        ("<if $n1><key>a</if>", SemanticValidity::ValidWithOpaque),
        ("<if $n1>", SemanticValidity::InvalidUnsafe),
        ("a<raw 00>", SemanticValidity::InvalidUnsafe),
    ] {
        assert_eq!(
            parse(text).semantic_validation().status(),
            expected,
            "{text}"
        );
    }
}

#[test]
fn plain_text_is_what_players_read() {
    assert_eq!(
        parse("Obtain <sheet Item $n1 0> from <if ($n1 == 1)>him<else>her</if>.").plain_text(),
        "Obtain  from him her ."
    );
    assert_eq!(parse("a \\< b<br>c").plain_text(), "a < b c");
    assert_eq!(parse("<i>Omnilex</i>").plain_text(), "Omnilex");
    assert_eq!(
        parse("<split \" \" 1><string $gs1></split> x").plain_text(),
        "x"
    );
}

#[test]
fn formatting_only_text_has_no_letters_in_what_players_read() {
    for text in [
        "",
        "<num $n1>/<num $n2>",
        "<icon 62> 100%",
        "<if $n1>1<else>2</if>",
    ] {
        assert!(parse(text).is_formatting_only(), "{text}");
    }
    for text in ["a", "<if $n1>yes<else>no</if>", "<i>x</i>", "<if $n1>"] {
        assert!(!parse(text).is_formatting_only(), "{text}");
    }
}
