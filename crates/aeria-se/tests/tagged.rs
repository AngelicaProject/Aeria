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
        project(r"Hi <player-name $n1>, 1 \< 2 & <color #FF0000FF>go</color>").expect("projection");
    assert_eq!(
        tagged.text,
        r#"Hi <x id="1"/>, 1 &lt; 2 &amp; <x id="2"/>go<x id="3"/>"#
    );
    assert_eq!(tagged.tags.len(), 3);
    assert!(tagged.tags[0].repeatable);
    assert!(tagged.tags[0].legend().contains("runtime value"));
    assert!(
        tagged.tags[0]
            .legend()
            .contains("player = number parameter 1")
    );
    assert!(tagged.tags[1].legend().contains("keep the order"));
    assert!(tagged.tags[1].legend().contains("color = #FF0000FF"));
    assert!(tagged.tags[2].legend().contains("end of: text color"));
    assert!(!tagged.tags[1].repeatable);
}

#[test]
fn legends_describe_game_references_with_their_values() {
    let tagged = project("<sheet Item $n1 0> and <i>x</i>").expect("projection");
    let legend = tagged.tags[0].legend();
    assert!(legend.contains("sheet = Item"), "{legend}");
    assert!(legend.contains("row = number parameter 1"), "{legend}");
    assert!(legend.contains("game data reference"), "{legend}");
    assert!(tagged.tags[1].legend().contains("italic text"));
    assert!(!tagged.tags[1].legend().contains("on ="));
}

#[test]
fn branches_become_groups_and_rebuild_with_escapes() {
    let source = "<if ($n1 == 1)>He is ready<else>She is ready</if>.";
    let tagged = project(source).expect("projection");
    assert_eq!(
        tagged.text,
        r#"<g id="1"><b>He is ready</b><b>She is ready</b></g>."#
    );
    let target = rebuild(
        source,
        r#"<g id="1"><b>Он готов, да (точно) {1}</b><b>Она &lt;готова&gt; всех</b></g>."#,
    )
    .expect("rebuilt");
    assert_eq!(
        target,
        r"<if ($n1 == 1)>Он готов, да (точно) \{1}<else>Она \<готова> всех</if>."
    );
    assert!(parse(&target).is_well_formed());
    check_assisted_structure(source, &target).expect("policy");
}

#[test]
fn identical_tagged_text_rebuilds_the_source() {
    for source in [
        "plain",
        r"Hi <player-name $n1>, 1 \< 2",
        "<if ($n1 == 1)>yes<else>no</if> and <sheet Item 5 0>",
        "<capitalize>outer <i>text</i> tail</capitalize>",
        "<code:35> kept",
        "<switch $weekday><case>Sunday<case>{2}</switch>",
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
    let source = "<player-name $n1> found <sheet Item 5 0>.";
    let target = rebuild(
        source,
        r#"Найден предмет <x id="2"/>, <x id="1"/>! Молодец, <x id="1"/>."#,
    )
    .expect("reordered and repeated");
    assert_eq!(
        target,
        "Найден предмет <sheet Item 5 0>, <player-name $n1>! Молодец, <player-name $n1>."
    );
}

#[test]
fn missing_repeated_unknown_and_misplaced_tags_are_refused() {
    let source = "<sheet Item 5 0> and <if ($n1 == 1)>a <player-name $n1><else>b</if>";
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
    let source = "<color #FF0000FF>red</color> text";
    assert!(messages(rebuild(source, r#"<x id="2"/>красный<x id="1"/> текст"#)).contains("order"));
    assert!(rebuild(source, r#"текст <x id="1"/>красный<x id="2"/>"#).is_ok());
}

#[test]
fn the_structure_check_catches_changes_the_tags_cannot_express() {
    let source = "<sheet Item 5 0> x";
    let errors =
        check_assisted_structure(source, "<sheet Item 6 0> x").expect_err("changed reference");
    assert!(errors.iter().any(|error| error.message.contains("missing")));
    assert!(check_assisted_structure(source, "<sheet Quest 5 0> x").is_err());
    assert!(check_assisted_structure(source, "x <sheet Item 5 0>").is_ok());
    assert!(check_assisted_structure(source, "<sheet Item 5 0><sheet Item 5 0> x").is_err());
    let errors = check_assisted_structure(source, "<sheet Item 5 0").expect_err("malformed");
    assert!(errors.len() > 1, "the parse diagnostics are included");

    // Text in a branch stays text, even when it reads as a number.
    let conditional = "<if ($n1 == 1)>one<else>many</if>";
    assert!(rebuild(conditional, r#"<g id="1"><b>1</b><b>много</b></g>"#).is_ok());
}

#[test]
fn malformed_sources_have_no_projection() {
    assert!(project("<if $n1>").is_err());
    assert!(project("a<raw 00>").is_err());
}

#[test]
fn every_golden_vector_round_trips_through_tags() {
    let mut checked = 0;
    for line in include_str!("fixtures/macro_text.golden.txt").lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let source = line.rsplit('\t').next().expect("text");
        let tagged = project(source).expect("projection");
        let rebuilt =
            rebuild(source, &tagged.text).unwrap_or_else(|errors| panic!("{source}: {errors:?}"));
        assert_eq!(rebuilt, source);
        check_assisted_structure(source, &rebuilt).expect("policy");
        checked += 1;
    }
    assert!(checked > 40, "only {checked} vectors were checked");
}
