use aeria_se::{ConstructRule, authoring_reference, check_assisted_structure, constructs};

fn refused(source: &str, target: &str) -> String {
    check_assisted_structure(source, target)
        .expect_err("the translation is refused")
        .into_iter()
        .map(|error| error.message)
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn constructs_explain_what_each_macro_does_and_what_may_change() {
    let found = constructs(r"Hi <player-name $n1>, <if $gn4>lass<else>lad</if>! <i>Go</i><br>")
        .expect("constructs");
    let legends: Vec<String> = found.iter().map(aeria_se::Construct::legend).collect();
    assert_eq!(found[0].spelling, "<player-name $n1>");
    assert_eq!(found[0].rule, ConstructRule::Keep);
    assert!(legends[0].contains("keep it"), "{}", legends[0]);
    let condition = found
        .iter()
        .find(|construct| construct.rule == ConstructRule::Condition)
        .expect("condition");
    assert!(
        condition.legend().contains("player character is female"),
        "a known global reads as its meaning: {}",
        condition.legend()
    );
    assert!(condition.legend().contains("may be reworded"));
    assert!(found.iter().any(
        |construct| construct.spelling == "<i>" && construct.rule == ConstructRule::Formatting
    ));
    assert!(
        found
            .iter()
            .any(|construct| construct.spelling == "<br>" && construct.rule == ConstructRule::Free)
    );
    assert!(
        constructs("<sheet Item 5 0").is_err(),
        "a malformed source has none"
    );
}

#[test]
fn an_idiom_is_explained_by_what_it_means() {
    let found = constructs(r#"You! <split " " 1><string $gs1></split>?"#).expect("constructs");
    assert!(
        found[0]
            .legend()
            .contains("the first name of the player character"),
        "{}",
        found[0].legend()
    );
}

#[test]
fn game_data_stays_and_may_move_or_repeat() {
    let source = "You get <num $n1> <sheet Item $n2 0>.";
    assert!(check_assisted_structure(source, "<sheet Item $n2 0>: <num $n1>.").is_ok());
    assert!(check_assisted_structure(source, "<num $n1> <sheet Item $n2 0>, <num $n1>!").is_ok());
    assert!(
        refused(source, "Вы получили <sheet Item $n2 0>.")
            .contains("<num $n1> of the source is missing")
    );
    assert!(
        refused(source, "<num $n1> <sheet Item $n2 0> <sheet Quest $n2 0>")
            .contains("may not add game data")
    );
    assert!(
        refused(source, "<num $n1> <sheet Item $n3 0>").contains("may not add game data"),
        "a changed argument is other data"
    );
}

#[test]
fn a_translation_may_restructure_conditions_its_language_needs() {
    let source = "Don't just stand there, <if $gn4>lass<else>lad</if>. Congratulate me!";
    assert!(
        check_assisted_structure(
            source,
            "Чего <if $gn4>застыла<else>застыл</if>, <if $gn4>девица<else>парень</if>? Похвали меня!"
        )
        .is_ok(),
        "a gender condition the language needs is added"
    );
    assert!(
        check_assisted_structure(source, "Чего стоишь столбом? Похвали меня!").is_ok(),
        "one it does not need is dropped"
    );
    assert!(
        check_assisted_structure(
            "Hello, <player-name $n1>.",
            "<if ($gn68 == 20)>Здравствуй, монах<else>Здравствуй</if>, <player-name $n1>."
        )
        .is_ok(),
        "a known global may be tested"
    );
    assert!(
        check_assisted_structure(
            "<num $n1> left.",
            "<if ($n1 == 1)>Осталась <num $n1><else>Осталось <num $n1></if>."
        )
        .is_ok(),
        "a value the source uses may be tested"
    );
    assert!(
        refused("Hello.", "<if ($n2 == 1)>Привет<else>Здравствуйте</if>.")
            .contains("tests $n2, which the source does not use")
    );
    assert!(
        refused("Hello.", "<if $gn4><player-name $n1><else>друг</if>.")
            .contains("may not add game data"),
        "branches of a new condition add no game data"
    );
}

#[test]
fn game_data_inside_a_dropped_condition_stays() {
    let source = "<if $gn4><sheet Item $n1 0> for her<else><sheet Item $n1 0> for him</if>";
    assert!(check_assisted_structure(source, "<sheet Item $n1 0> для тебя").is_ok());
    assert!(refused(source, "Для тебя").contains("<sheet Item $n1 0> of the source is missing"));
}

#[test]
fn formatting_may_change_but_what_it_opens_it_closes() {
    let source = "<i>Heavens</i> and <b>earth</b>";
    // As the official localizations do: moved, dropped, added, repeated.
    assert!(check_assisted_structure(source, "<b>Земля</b> и <i>небеса</i>").is_ok());
    assert!(check_assisted_structure(source, "Небеса и земля").is_ok());
    assert!(check_assisted_structure(source, "<i>Небеса</i> и <b>земля</b> <i>!</i>").is_ok());
    assert!(check_assisted_structure("Heavens and earth", "<i>Небеса</i> и земля").is_ok());
    let colored = "Use <ui-color 500><ui-edge-color 501>Fast Blade</ui-edge-color></ui-color>.";
    assert!(check_assisted_structure(colored, "Используйте «Быстрый клинок».").is_ok());

    // Nothing may stay open past the string, or close before it opens.
    assert!(refused(source, "<i>Небеса и земля").contains("needs its </i>"));
    assert!(refused(colored, "Используйте <ui-color 500>Быстрый клинок.").contains("</ui-color>"));
    assert!(refused(source, "Небеса</i> и <i>земля").contains("comes before the <i>"));
    assert!(refused(source, "<i>Небеса</i></i> и земля").contains("without its <i>"));
    // What a source forgot to close may stay open or be closed, never more.
    let open = "<i>Danger";
    assert!(check_assisted_structure(open, "<i>Опасно").is_ok());
    assert!(check_assisted_structure(open, "<i>Опасно</i>").is_ok());
    assert!(check_assisted_structure(open, "Опасно").is_ok());
    assert!(refused(open, "<i><i>Опасно").contains("needs its </i>"));
    // A stray closing tag of the source may be dropped, not added to.
    let stray = "I </i>loathe</i> lemons!";
    assert!(check_assisted_structure(stray, "Я <i>ненавижу</i> лимоны!").is_ok());
    assert!(check_assisted_structure(stray, "Я ненавижу</i> лимоны!").is_ok());
    assert!(refused(stray, "Я </i>ненавижу</i></i> лимоны!").contains("without its <i>"));
}

#[test]
fn layout_and_text_transforms_are_free() {
    assert!(check_assisted_structure("One.<br>Two.", "Раз. Два.").is_ok());
    assert!(check_assisted_structure("One. Two.", "Раз.<br>Два.").is_ok());
    assert!(check_assisted_structure("<capitalize>well</capitalize>", "Ну").is_ok());
    assert!(
        refused("You, <string $gs1>!", "Ты!").contains("<string $gs1> of the source is missing"),
        "a transform that shows a value is game data"
    );
}

#[test]
fn a_letter_case_transform_is_explained_as_what_capitalizes_game_data() {
    // The game stores "paladin" and shows "Paladin" through the transform;
    // a translation that drops it shows the name in lower case.
    let found = constructs("<title-case><sheet ClassJob $n1 0></title-case>").expect("constructs");
    assert!(found[0].spelling.starts_with("<title-case>"));
    assert_eq!(found[0].rule, ConstructRule::LetterCase);
    let legend = found[0].legend();
    assert!(
        legend.contains("keep a case transform around game data"),
        "{legend}"
    );
    assert!(
        legend.contains("<capitalize> in place of <title-case>"),
        "{legend}"
    );
    assert_eq!(found[1].rule, ConstructRule::Keep);
}

#[test]
fn a_translation_starts_with_a_speaker_name_exactly_when_the_source_does() {
    assert!(check_assisted_structure("(-???-)Hello.", "(-Некто-)Привет.").is_ok());
    assert!(refused("(-???-)Hello.", "Привет.").contains("must start with a speaker name"));
    assert!(refused("(-???-)Hello.", "Привет (-???-).").contains("must start with a speaker name"));
    assert!(refused("Hello.", "(-Некто-)Привет.").contains("the source has none"));
    assert!(refused("(-???-)Hello.", "(--)Привет.").contains("is empty"));
    assert!(check_assisted_structure("Damage (-<num $n1>%)", "Урон (-<num $n1>%)").is_ok());
}

#[test]
fn malformed_text_is_refused_with_its_diagnostics() {
    let errors = refused("<sheet Item 5 0> x", "<sheet Item 5 0");
    assert!(errors.contains("not well-formed"), "{errors}");
}

#[test]
fn every_golden_vector_satisfies_the_policy_as_it_is() {
    let mut checked = 0;
    for line in include_str!("fixtures/macro_text.golden.txt").lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let source = line.rsplit('\t').next().expect("text");
        constructs(source).unwrap_or_else(|error| panic!("{source}: {error:?}"));
        check_assisted_structure(source, source)
            .unwrap_or_else(|errors| panic!("{source}: {errors:?}"));
        checked += 1;
    }
    assert!(checked > 40, "only {checked} vectors were checked");
}

#[test]
fn the_reference_names_the_known_globals() {
    let reference = authoring_reference();
    assert!(reference.contains("$gn4 is"), "{reference}");
    assert!(reference.contains("<if (left op right)>"), "{reference}");
}

#[test]
fn a_message_about_another_player_may_agree_with_their_gender() {
    let source = "<capitalize><if ($gs1 == $gs2)>you<else><if $gn7><noun-en ObjStr 2 $gn7 1 1><else>{$gs2}</if></if></capitalize> <if ($gs1 == $gs2)>have<else>has</if> left the party.";
    assert!(
        check_assisted_structure(
            source,
            r#"<capitalize><if ($gs1 == $gs2)>Вы покинули<else><if $gn7><noun-en ObjStr 2 $gn7 1 1><else>{$gs2}</if> <if $gn7><if "<sheet BNpcName $gn7 6>">покинула<else>покинул</if><else><if $gn5>покинула<else>покинул</if></if></if></capitalize> группу."#
        )
        .is_ok(),
        "the gender of the player named by $gs2 may be tested"
    );
    let constructs = constructs(source).expect("constructs");
    let legend = constructs
        .iter()
        .map(aeria_se::Construct::legend)
        .find(|legend| legend.starts_with("<if $gn7>"))
        .expect("the legend of the condition on $gn7");
    assert!(legend.contains("ObjStr row"), "{legend}");
}

#[test]
fn the_player_character_may_be_named_where_the_source_does_not() {
    let source = "Well done!";
    for target in [
        "Отлично, <string $gs1>!",
        r#"Отлично, <split " " 1><string $gs1></split>!"#,
        r#"Отлично, <split " " 2><string $gs1></split>!"#,
        "Отличная работа для <sheet ClassJob $gn68 0>!",
        "Отлично для <sheet Race $gn71 0>!",
    ] {
        assert!(check_assisted_structure(source, target).is_ok(), "{target}");
    }
    assert!(refused(source, "Отлично, <string $gs2>!").contains("is not in the source"));
    assert!(refused(source, "Отлично, <num $n1>!").contains("is not in the source"));
    assert!(
        refused(source, "Отлично, <sheet ClassJob 19 0>!").contains("is not in the source"),
        "a constant row is not a player insertion"
    );
    assert!(
        authoring_reference().contains(r#"<split " " 1><string $gs1></split> is the first name"#)
    );
}

#[test]
fn whether_a_character_of_a_message_is_female_may_be_tested() {
    let source = "<if $gn7><noun-en ObjStr 2 $gn7 1 1><else>{$gs2}</if> left.";
    assert!(
        check_assisted_structure(
            source,
            r#"<if $gn7><noun-en ObjStr 2 $gn7 1 1><else>{$gs2}</if> <if "<sheet BNpcName $gn7 6>">ушла<else>ушёл</if>."#
        )
        .is_ok()
    );
    for other in [
        "<sheet BNpcName $gn7 0>",
        "<sheet BNpcName 12 6>",
        "<sheet ENpcResident $gn7 6>",
    ] {
        assert!(
            refused(source, &format!("{other} ушёл.")).contains("is not in the source"),
            "{other}"
        );
    }
    assert!(authoring_reference().contains(r#"<if "<sheet BNpcName $gn8 6>">"#));
}
