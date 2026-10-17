//! Names the columns of every root sheet of a real game from a local
//! EXDSchema checkout, and prints the sheets whose schema has another number
//! of columns. Run it after a game patch or a change to the column order:
//!
//! ```text
//! git clone --depth 1 --branch ver/<version> https://github.com/xivdev/EXDSchema.git <dir>
//! AERIA_GAME_PATH="…/FINAL FANTASY XIV Online" AERIA_EXDSCHEMA_PATH=<dir> cargo test -p aeria-schema --release --test game_schema -- --ignored --nocapture
//! ```

use std::path::Path;

#[test]
#[ignore = "needs a game installation in AERIA_GAME_PATH and EXDSchema in AERIA_EXDSCHEMA_PATH"]
fn the_schema_names_the_columns_of_the_game() {
    let game = std::env::var("AERIA_GAME_PATH").expect("AERIA_GAME_PATH");
    let schemas = std::env::var("AERIA_EXDSCHEMA_PATH").expect("AERIA_EXDSCHEMA_PATH");
    let source =
        aeria_source::GameSource::open(&game, "en".parse().expect("language")).expect("game");
    let (mut named, mut other) = (0, Vec::new());
    for sheet in source.sheet_names() {
        let Ok(text) = std::fs::read_to_string(Path::new(&schemas).join(format!("{sheet}.yml")))
        else {
            continue;
        };
        let columns = source
            .sheet_columns(sheet)
            .expect("game")
            .expect("a header");
        let fields =
            aeria_schema::field_names(&text).unwrap_or_else(|error| panic!("{sheet}: {error}"));
        let count = fields.len();
        if aeria_schema::column_names(fields, &columns).is_some() {
            named += 1;
        } else {
            other.push(format!(
                "{sheet}: {count} fields, {} columns",
                columns.len()
            ));
        }
    }
    println!("{named} sheets named; another column count:");
    for sheet in &other {
        println!("  {sheet}");
    }
    assert!(named > other.len(), "the schema names few sheets");
}
