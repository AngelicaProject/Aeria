use aeria_se::{check_assisted_structure, parse, project, rebuild};

fn messages(result: Result<String, Vec<aeria_se::TaggedError>>) -> String {
    result
        .expect_err("the translation is refused")
        .into_iter()
        .map(|error| error.message)
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn projection_tags_constructs_and_escapes_prose() {
    let tagged =
        project(r"Hi <pcname(lnum1)>, 1 \< 2 & <color(5)>go<color(0)>").expect("projection");
    assert_eq!(
        tagged.text,
        r#"Hi <x id="1"/>, 1 &lt; 2 &amp; <x id="2"/>go<x id="3"/>"#
    );
    assert_eq!(tagged.tags.len(), 3);
    assert!(tagged.tags[0].repeatable);
    assert!(tagged.tags[0].legend().contains("runtime value"));
    assert!(tagged.tags[1].legend().contains("keep the order"));
    assert!(!tagged.tags[1].repeatable);
}

#[test]
fn branches_become_groups_and_rebuild_with_argument_escapes() {
    let source = "<if([lnum1==1],He is ready,She is ready)>.";
    let tagged = project(source).expect("projection");
    assert_eq!(
        tagged.text,
        r#"<g id="1"><b>He is ready</b><b>She is ready</b></g>."#
    );
    let target = rebuild(
        source,
        r#"<g id="1"><b>Он готов, да (точно)</b><b>Она [готова] &gt; всех</b></g>."#,
    )
    .expect("rebuilt");
    assert_eq!(
        target,
        r"<if([lnum1==1],Он готов\, да \(точно\),Она \[готова\] \> всех)>."
    );
    assert!(parse(&target).is_well_formed());
    check_assisted_structure(source, &target).expect("policy");
}

#[test]
fn identical_tagged_text_rebuilds_the_source() {
    for source in [
        "plain",
        r"Hi <pcname(lnum1)>, 1 \< 2",
        "<if([lnum1==1],yes,no)> and <sheet(Item,5,0)>",
        "<string(outer <italic(1)>text<italic(0)> tail)>",
        "<payload:35> kept",
    ] {
        let tagged = project(source).expect("projection");
        assert_eq!(
            rebuild(source, &tagged.text).expect("rebuilt"),
            source,
            "{source}"
        );
    }
}

#[test]
fn moving_and_repeating_runtime_values_are_allowed() {
    let source = "<pcname(lnum1)> found <sheet(Item,5,0)>.";
    let target = rebuild(
        source,
        r#"Найден предмет <x id="2"/>, <x id="1"/>! Молодец, <x id="1"/>."#,
    )
    .expect("reordered and repeated");
    assert_eq!(
        target,
        "Найден предмет <sheet(Item,5,0)>, <pcname(lnum1)>! Молодец, <pcname(lnum1)>."
    );
}

#[test]
fn missing_repeated_unknown_and_misplaced_tags_are_refused() {
    let source = "<sheet(Item,5,0)> and <if([lnum1==1],a <pcname(lnum1)>,b)>";
    assert!(
        messages(rebuild(
            source,
            r#"<g id="2"><b>a <x id="3"/></b><b>b</b></g>"#
        ))
        .contains("tag 1")
    );
    assert!(
        messages(rebuild(
            source,
            r#"<x id="1"/><x id="1"/><g id="2"><b>a <x id="3"/></b><b>b</b></g>"#
        ))
        .contains("only once")
    );
    assert!(
        messages(rebuild(
            source,
            r#"<x id="1"/><x id="9"/><g id="2"><b>a <x id="3"/></b><b>b</b></g>"#
        ))
        .contains("does not exist")
    );
    assert!(
        messages(rebuild(
            source,
            r#"<x id="1"/><x id="3"/><g id="2"><b>a</b><b>b</b></g>"#
        ))
        .contains("belongs in branch 1 of tag 2")
    );
    assert!(
        messages(rebuild(
            source,
            r#"<x id="1"/><g id="2"><b>a <x id="3"/></b></g>"#
        ))
        .contains("exactly 2")
    );
    assert!(messages(rebuild(source, r#"<x id="1"/> <b>"#)).contains("<b>"));
    assert!(messages(rebuild(source, "1 < 2")).contains("&lt;"));
}

#[test]
fn formatting_keeps_its_order() {
    let source = "<color(5)>red<color(0)> text";
    assert!(messages(rebuild(source, r#"<x id="2"/>красный<x id="1"/> текст"#)).contains("order"));
    assert!(rebuild(source, r#"текст <x id="1"/>красный<x id="2"/>"#).is_ok());
}

#[test]
fn the_structure_check_catches_changes_the_tags_cannot_express() {
    let source = "<sheet(Item,5,0)> x";
    let errors =
        check_assisted_structure(source, "<sheet(Item,6,0)> x").expect_err("changed reference");
    assert!(errors.iter().any(|error| error.message.contains("missing")));
    assert!(check_assisted_structure(source, "x <sheet(Item,5,0)>").is_ok());
    assert!(check_assisted_structure(source, "<sheet(Item,5,0)><sheet(Item,5,0)> x").is_err());
    assert!(check_assisted_structure(source, "<sheet(Item,5,0)").is_err());

    // A branch whose translation reads as a number would change the construct.
    let conditional = "<if([lnum1==1],one,many)>";
    assert!(rebuild(conditional, r#"<g id="1"><b>1</b><b>много</b></g>"#).is_err());
}

#[test]
fn malformed_sources_have_no_projection() {
    assert!(project("<if(").is_err());
}

#[test]
fn every_well_formed_conformance_vector_round_trips_through_tags() {
    let fixtures = [
        include_str!("fixtures/lumina_to_macro_string.golden.txt"),
        include_str!("fixtures/parser_compatibility.txt"),
    ];
    let mut checked = 0;
    for line in fixtures.iter().flat_map(|fixture| fixture.lines()) {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let source = line.split_once('\t').map_or(line, |(_, source)| source);
        if !parse(source).is_well_formed() {
            continue;
        }
        let tagged = project(source).expect("projection");
        let rebuilt =
            rebuild(source, &tagged.text).unwrap_or_else(|errors| panic!("{source}: {errors:?}"));
        check_assisted_structure(source, &rebuilt).expect("policy");
        checked += 1;
    }
    assert!(checked > 10, "only {checked} vectors were checked");
}
