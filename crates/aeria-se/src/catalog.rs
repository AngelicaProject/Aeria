//! The macro catalog: one entry per named macro code.
//!
//! Every fact Aeria knows about a macro lives in its entry: the byte code,
//! the name and form it is written with, its arguments and their roles, the
//! semantic family that decides the translation policy, and a short
//! description for people and the translation agent. Printing, parsing,
//! validation, tagged text, and the editor all read this table, so naming a
//! new macro code is one new entry.
//!
//! Names and forms are part of the persisted macro text (see
//! `docs/architecture/strings.md`): renaming an entry changes stored text.

/// The broad semantic family of a macro. It decides how assisted translation
/// may treat the construct.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum SemanticFamily {
    /// Text transforms whose argument is user-facing text.
    TranslatableText,
    /// Formatting or presentation state. Formatting keeps its order.
    FormattingPresentation,
    /// Conditions and selections whose branches are user-facing text.
    ConditionalSelection,
    /// Values supplied at runtime, such as numbers and names.
    RuntimeContextValue,
    /// References to game data, such as item names.
    GameDataReference,
    /// Layout and textual control: line breaks, icons, hyphens.
    LayoutTextualControl,
    /// A construct whose runtime meaning is not established. It is preserved
    /// exactly and never interpreted.
    OpaqueProtected,
}

/// Where an argument is written.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Place {
    /// Inside the opening tag, separated by spaces.
    Inline,
    /// As the content of the element, between the opening and closing tags.
    Block,
}

/// What an argument means. Roles choose how a value is written (colors in
/// hexadecimal) and how it is described and previewed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Role {
    /// A condition: a comparison, or a value that is true when not zero.
    Condition,
    /// A value selected between branches.
    Selector,
    /// User-facing text.
    Text,
    /// A number shown to the player.
    Number,
    /// A game object, such as a player or a character, by its parameter.
    Object,
    /// An Excel sheet name.
    Sheet,
    /// A row ID of a sheet.
    Row,
    /// A column index of a sheet.
    Column,
    /// A 32-bit color value, written `#AARRGGBB`.
    Color,
    /// A row of the `UIColor` sheet.
    UiColor,
    /// An icon ID.
    Icon,
    /// A flag or mode switch, usually 0 or 1.
    Flag,
    /// A separator string inserted by a number format.
    Separator,
    /// A count or amount.
    Count,
    /// A time value.
    Time,
    /// A value whose meaning is not established.
    Other,
}

/// One argument of a macro.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Arg {
    /// A short name shown in descriptions and the editor.
    pub name: &'static str,
    pub role: Role,
    pub place: Place,
}

const fn inline(name: &'static str, role: Role) -> Arg {
    Arg {
        name,
        role,
        place: Place::Inline,
    }
}

const fn block(name: &'static str, role: Role) -> Arg {
    Arg {
        name,
        role,
        place: Place::Block,
    }
}

/// The argument that makes a pair macro its closing tag.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Close {
    /// The integer argument, such as `0` for `</i>`.
    Int(u32),
    /// The `$stackcolor` placeholder, which restores the previous color.
    StackColor,
}

/// How a macro is written.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Form {
    /// `<name arg arg>` with inline arguments only, or `<name>`.
    Inline,
    /// An opening tag and a closing tag written as two independent macros:
    /// `<i>` and `</i>`, or `<color #FF0000FF>` and `</color>`. The closing
    /// tag is the macro whose one argument is `close`; the opening tag
    /// omits its argument when it is `implied`.
    Pair { close: Close, implied: Option<u32> },
    /// An element whose block arguments are its content:
    /// `<if (…)>then<else>else</if>`. With `separator`, block arguments are
    /// separated by `<separator>`; with `leading`, every block argument,
    /// including the first, is preceded by it, as in `<case>`.
    Block {
        separator: Option<&'static str>,
        leading: bool,
    },
}

/// One named macro code.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MacroSpec {
    pub code: u8,
    /// The tag name in macro text.
    pub name: &'static str,
    pub family: SemanticFamily,
    pub form: Form,
    /// The arguments in byte order.
    pub args: &'static [Arg],
    /// The number of required arguments; later ones are optional.
    pub required: usize,
    /// Whether the last argument repeats any number of times.
    pub repeats: bool,
    /// A one-line description for people and the translation agent.
    pub summary: &'static str,
}

impl MacroSpec {
    /// Whether `count` arguments fit this macro.
    #[must_use]
    pub const fn accepts_count(&self, count: usize) -> bool {
        count >= self.required && (self.repeats || count <= self.args.len())
    }

    /// The argument description at `index`, following repetition.
    #[must_use]
    pub fn arg(&self, index: usize) -> Option<&'static Arg> {
        self.args
            .get(index)
            .or_else(|| self.repeats.then(|| self.args.last()).flatten())
    }

    /// Whether the argument at `index` is user-facing text that is
    /// translated in place.
    #[must_use]
    pub fn is_translatable_arg(&self, index: usize) -> bool {
        self.family != SemanticFamily::OpaqueProtected
            && self
                .arg(index)
                .is_some_and(|arg| arg.place == Place::Block && arg.role == Role::Text)
    }

    /// The closing tag name of a pair or block.
    #[must_use]
    pub fn closing_name(&self) -> &'static str {
        self.name
    }
}

use Role::{
    Color, Column, Condition, Count, Flag, Icon, Number, Object, Other, Row, Selector, Separator,
    Sheet, Text, Time, UiColor,
};
use SemanticFamily::{
    ConditionalSelection, FormattingPresentation, GameDataReference, LayoutTextualControl,
    OpaqueProtected, RuntimeContextValue, TranslatableText,
};

const INLINE: Form = Form::Inline;
const ELSE: Form = Form::Block {
    separator: Some("else"),
    leading: false,
};
const CONTENT: Form = Form::Block {
    separator: None,
    leading: false,
};

const NOUN_ARGS: &[Arg] = &[
    inline("sheet", Sheet),
    inline("form", Other),
    inline("row", Row),
    inline("count", Count),
    inline("case", Other),
    inline("extra", Other),
];

/// Every named macro, by code.
pub const MACROS: &[MacroSpec] = &[
    MacroSpec {
        code: 0x06,
        name: "reset-time",
        family: RuntimeContextValue,
        form: INLINE,
        args: &[inline("hour", Time), inline("weekday", Time)],
        required: 1,
        repeats: false,
        summary: "sets the time placeholders to the next daily or weekly reset",
    },
    MacroSpec {
        code: 0x07,
        name: "set-time",
        family: RuntimeContextValue,
        form: INLINE,
        args: &[inline("time", Time)],
        required: 1,
        repeats: false,
        summary: "sets the time placeholders ($hour, $day, …) to a Unix time",
    },
    MacroSpec {
        code: 0x08,
        name: "if",
        family: ConditionalSelection,
        form: ELSE,
        args: &[
            inline("condition", Condition),
            block("then", Text),
            block("else", Text),
        ],
        required: 3,
        repeats: false,
        summary: "shows the first branch when the condition holds, otherwise the branch after <else>",
    },
    MacroSpec {
        code: 0x09,
        name: "switch",
        family: ConditionalSelection,
        form: Form::Block {
            separator: Some("case"),
            leading: true,
        },
        args: &[inline("value", Selector), block("case", Text)],
        required: 2,
        repeats: true,
        summary: "shows the case whose position equals the value: the first <case> for 1, the second for 2, …",
    },
    MacroSpec {
        code: 0x0A,
        name: "player-name",
        family: RuntimeContextValue,
        form: INLINE,
        args: &[inline("player", Object)],
        required: 1,
        repeats: false,
        summary: "the name of a player character",
    },
    MacroSpec {
        code: 0x0B,
        name: "if-gender",
        family: ConditionalSelection,
        form: ELSE,
        args: &[
            inline("player", Object),
            block("male", Text),
            block("female", Text),
        ],
        required: 3,
        repeats: false,
        summary: "shows the first branch for a male player character, the branch after <else> for a female one",
    },
    MacroSpec {
        code: 0x0C,
        name: "if-name",
        family: ConditionalSelection,
        form: ELSE,
        args: &[
            inline("player", Object),
            inline("name", Other),
            block("then", Text),
            block("else", Text),
        ],
        required: 4,
        repeats: false,
        summary: "shows the first branch when the player character has the given name",
    },
    MacroSpec {
        code: 0x0D,
        name: "josa",
        family: ConditionalSelection,
        form: ELSE,
        args: &[
            inline("word", Other),
            block("final consonant", Text),
            block("final vowel", Text),
        ],
        required: 3,
        repeats: false,
        summary: "a Korean particle chosen by how the preceding word ends",
    },
    MacroSpec {
        code: 0x0E,
        name: "josa-ro",
        family: ConditionalSelection,
        form: ELSE,
        args: &[
            inline("word", Other),
            block("final consonant", Text),
            block("final vowel", Text),
        ],
        required: 3,
        repeats: false,
        summary: "the Korean particle 으로/로 chosen by how the preceding word ends",
    },
    MacroSpec {
        code: 0x0F,
        name: "if-self",
        family: ConditionalSelection,
        form: ELSE,
        args: &[
            inline("player", Object),
            block("self", Text),
            block("other", Text),
        ],
        required: 3,
        repeats: false,
        summary: "shows the first branch when the character is the player reading the text, otherwise the branch after <else>",
    },
    MacroSpec {
        code: 0x10,
        name: "br",
        family: LayoutTextualControl,
        form: INLINE,
        args: &[],
        required: 0,
        repeats: false,
        summary: "a line break",
    },
    MacroSpec {
        code: 0x11,
        name: "wait",
        family: LayoutTextualControl,
        form: INLINE,
        args: &[inline("duration", Number)],
        required: 1,
        repeats: false,
        summary: "pauses text display",
    },
    MacroSpec {
        code: 0x12,
        name: "icon",
        family: LayoutTextualControl,
        form: INLINE,
        args: &[inline("icon", Icon)],
        required: 1,
        repeats: false,
        summary: "an inline icon of the game font",
    },
    MacroSpec {
        code: 0x13,
        name: "color",
        family: FormattingPresentation,
        form: Form::Pair {
            close: Close::StackColor,
            implied: None,
        },
        args: &[inline("color", Color)],
        required: 1,
        repeats: false,
        summary: "text color; </color> restores the previous color",
    },
    MacroSpec {
        code: 0x14,
        name: "edge-color",
        family: FormattingPresentation,
        form: Form::Pair {
            close: Close::StackColor,
            implied: None,
        },
        args: &[inline("color", Color)],
        required: 1,
        repeats: false,
        summary: "text outline color; </edge-color> restores the previous color",
    },
    MacroSpec {
        code: 0x15,
        name: "shadow-color",
        family: FormattingPresentation,
        form: Form::Pair {
            close: Close::StackColor,
            implied: None,
        },
        args: &[inline("color", Color)],
        required: 1,
        repeats: false,
        summary: "text shadow color; </shadow-color> restores the previous color",
    },
    MacroSpec {
        code: 0x16,
        name: "shy",
        family: LayoutTextualControl,
        form: INLINE,
        args: &[],
        required: 0,
        repeats: false,
        summary: "a soft hyphen: a place where a long word may break",
    },
    MacroSpec {
        code: 0x17,
        name: "key",
        family: OpaqueProtected,
        form: INLINE,
        args: &[inline("value", Other)],
        required: 0,
        repeats: false,
        summary: "a key prompt; its runtime meaning is not established",
    },
    MacroSpec {
        code: 0x18,
        name: "scale",
        family: OpaqueProtected,
        form: INLINE,
        args: &[inline("scale", Other)],
        required: 1,
        repeats: false,
        summary: "text scale; its runtime meaning is not established",
    },
    MacroSpec {
        code: 0x19,
        name: "b",
        family: FormattingPresentation,
        form: Form::Pair {
            close: Close::Int(0),
            implied: Some(1),
        },
        args: &[inline("on", Flag)],
        required: 1,
        repeats: false,
        summary: "bold text",
    },
    MacroSpec {
        code: 0x1A,
        name: "i",
        family: FormattingPresentation,
        form: Form::Pair {
            close: Close::Int(0),
            implied: Some(1),
        },
        args: &[inline("on", Flag)],
        required: 1,
        repeats: false,
        summary: "italic text",
    },
    MacroSpec {
        code: 0x1B,
        name: "edge",
        family: OpaqueProtected,
        form: INLINE,
        args: &[inline("on", Flag)],
        required: 1,
        repeats: false,
        summary: "turns the text outline on (1) or off (0)",
    },
    MacroSpec {
        code: 0x1C,
        name: "shadow",
        family: OpaqueProtected,
        form: INLINE,
        args: &[inline("on", Flag)],
        required: 1,
        repeats: false,
        summary: "turns the text shadow on (1) or off (0)",
    },
    MacroSpec {
        code: 0x1D,
        name: "nbsp",
        family: LayoutTextualControl,
        form: INLINE,
        args: &[],
        required: 0,
        repeats: false,
        summary: "a non-breaking space",
    },
    MacroSpec {
        code: 0x1E,
        name: "icon2",
        family: OpaqueProtected,
        form: INLINE,
        args: &[inline("icon", Icon)],
        required: 1,
        repeats: false,
        summary: "an inline icon that depends on the input device",
    },
    MacroSpec {
        code: 0x1F,
        name: "hyphen",
        family: LayoutTextualControl,
        form: INLINE,
        args: &[],
        required: 0,
        repeats: false,
        summary: "a hyphen that does not break the line",
    },
    MacroSpec {
        code: 0x20,
        name: "num",
        family: RuntimeContextValue,
        form: INLINE,
        args: &[inline("value", Number)],
        required: 1,
        repeats: false,
        summary: "a number",
    },
    MacroSpec {
        code: 0x21,
        name: "hex",
        family: RuntimeContextValue,
        form: INLINE,
        args: &[inline("value", Number)],
        required: 1,
        repeats: false,
        summary: "a number in hexadecimal",
    },
    MacroSpec {
        code: 0x22,
        name: "kilo",
        family: RuntimeContextValue,
        form: INLINE,
        args: &[inline("value", Number), inline("separator", Separator)],
        required: 2,
        repeats: false,
        summary: "a number with a thousands separator",
    },
    MacroSpec {
        code: 0x23,
        name: "byte",
        family: RuntimeContextValue,
        form: INLINE,
        args: &[inline("value", Number)],
        required: 1,
        repeats: false,
        summary: "a size in bytes with a unit",
    },
    MacroSpec {
        code: 0x24,
        name: "num2",
        family: RuntimeContextValue,
        form: INLINE,
        args: &[inline("value", Number)],
        required: 1,
        repeats: false,
        summary: "a number with at least two digits, such as 05",
    },
    MacroSpec {
        code: 0x25,
        name: "time",
        family: OpaqueProtected,
        form: INLINE,
        args: &[inline("value", Time)],
        required: 1,
        repeats: false,
        summary: "a formatted time; its runtime meaning is not established",
    },
    MacroSpec {
        code: 0x26,
        name: "float",
        family: RuntimeContextValue,
        form: INLINE,
        args: &[
            inline("value", Number),
            inline("radix", Number),
            inline("separator", Separator),
            inline("digits", Number),
        ],
        required: 3,
        repeats: false,
        summary: "a decimal number",
    },
    MacroSpec {
        code: 0x27,
        name: "link",
        family: LayoutTextualControl,
        form: CONTENT,
        args: &[
            inline("kind", Other),
            inline("a", Other),
            inline("b", Other),
            inline("c", Other),
            block("text", Text),
        ],
        required: 5,
        repeats: false,
        summary: "a link to a game object",
    },
    MacroSpec {
        code: 0x28,
        name: "sheet",
        family: GameDataReference,
        form: INLINE,
        args: &[
            inline("sheet", Sheet),
            inline("row", Row),
            inline("column", Column),
            inline("parameter", Other),
        ],
        required: 2,
        repeats: true,
        summary: "a value from a game data sheet, such as a name",
    },
    MacroSpec {
        code: 0x29,
        name: "string",
        family: TranslatableText,
        form: INLINE,
        args: &[inline("value", Text)],
        required: 1,
        repeats: false,
        summary: "a text value passed to the string",
    },
    MacroSpec {
        code: 0x2A,
        name: "upper",
        family: TranslatableText,
        form: CONTENT,
        args: &[block("text", Text)],
        required: 1,
        repeats: false,
        summary: "the content in upper case",
    },
    MacroSpec {
        code: 0x2B,
        name: "capitalize",
        family: TranslatableText,
        form: CONTENT,
        args: &[block("text", Text)],
        required: 1,
        repeats: false,
        summary: "the content with its first letter in upper case",
    },
    MacroSpec {
        code: 0x2C,
        name: "split",
        family: OpaqueProtected,
        form: CONTENT,
        args: &[
            block("text", Text),
            inline("separator", Separator),
            inline("part", Number),
        ],
        required: 3,
        repeats: false,
        summary: "one part of the content split at a separator, such as a first name",
    },
    MacroSpec {
        code: 0x2D,
        name: "title-case",
        family: TranslatableText,
        form: CONTENT,
        args: &[block("text", Text)],
        required: 1,
        repeats: false,
        summary: "the content with every word capitalized",
    },
    MacroSpec {
        code: 0x2E,
        name: "fixed",
        family: OpaqueProtected,
        form: INLINE,
        args: &[inline("kind", Other), inline("value", Other)],
        required: 1,
        repeats: true,
        summary: "fixed game text; its runtime meaning is not established",
    },
    MacroSpec {
        code: 0x2F,
        name: "lower",
        family: TranslatableText,
        form: CONTENT,
        args: &[block("text", Text)],
        required: 1,
        repeats: false,
        summary: "the content in lower case",
    },
    MacroSpec {
        code: 0x30,
        name: "noun-ja",
        family: GameDataReference,
        form: INLINE,
        args: NOUN_ARGS,
        required: 3,
        repeats: false,
        summary: "a Japanese noun from a game data sheet",
    },
    MacroSpec {
        code: 0x31,
        name: "noun-en",
        family: GameDataReference,
        form: INLINE,
        args: NOUN_ARGS,
        required: 3,
        repeats: false,
        summary: "an English noun from a game data sheet, with article and number",
    },
    MacroSpec {
        code: 0x32,
        name: "noun-de",
        family: GameDataReference,
        form: INLINE,
        args: NOUN_ARGS,
        required: 3,
        repeats: false,
        summary: "a German noun from a game data sheet, with article, number, and case",
    },
    MacroSpec {
        code: 0x33,
        name: "noun-fr",
        family: GameDataReference,
        form: INLINE,
        args: NOUN_ARGS,
        required: 3,
        repeats: false,
        summary: "a French noun from a game data sheet, with article and number",
    },
    MacroSpec {
        code: 0x34,
        name: "noun-zh",
        family: GameDataReference,
        form: INLINE,
        args: NOUN_ARGS,
        required: 3,
        repeats: false,
        summary: "a Chinese noun from a game data sheet",
    },
    MacroSpec {
        code: 0x40,
        name: "lower-first",
        family: TranslatableText,
        form: CONTENT,
        args: &[block("text", Text)],
        required: 1,
        repeats: false,
        summary: "the content with its first letter in lower case",
    },
    MacroSpec {
        code: 0x41,
        name: "sheet-sub",
        family: GameDataReference,
        form: INLINE,
        args: &[
            inline("sheet", Sheet),
            inline("row", Row),
            inline("subrow", Row),
            inline("column", Column),
            inline("a", Other),
            inline("b", Other),
        ],
        required: 3,
        repeats: true,
        summary: "a value from a subrow of a game data sheet",
    },
    MacroSpec {
        code: 0x42,
        name: "platform",
        family: GameDataReference,
        form: INLINE,
        args: &[inline("text", Other)],
        required: 1,
        repeats: false,
        summary: "text that depends on the platform, such as a controller name",
    },
    MacroSpec {
        code: 0x48,
        name: "ui-color",
        family: FormattingPresentation,
        form: Form::Pair {
            close: Close::Int(0),
            implied: None,
        },
        args: &[inline("color", UiColor)],
        required: 1,
        repeats: false,
        summary: "text color from the UIColor sheet; </ui-color> restores the previous color",
    },
    MacroSpec {
        code: 0x49,
        name: "ui-edge-color",
        family: FormattingPresentation,
        form: Form::Pair {
            close: Close::Int(0),
            implied: None,
        },
        args: &[inline("color", UiColor)],
        required: 1,
        repeats: false,
        summary: "text outline color from the UIColor sheet; </ui-edge-color> restores the previous color",
    },
    MacroSpec {
        code: 0x4A,
        name: "ruby",
        family: LayoutTextualControl,
        form: Form::Block {
            separator: Some("rt"),
            leading: false,
        },
        args: &[block("base", Text), block("reading", Text)],
        required: 2,
        repeats: false,
        summary: "text with a reading above it; the reading follows <rt>",
    },
    MacroSpec {
        code: 0x50,
        name: "digit",
        family: RuntimeContextValue,
        form: INLINE,
        args: &[inline("value", Number), inline("digits", Number)],
        required: 2,
        repeats: false,
        summary: "a number padded with zeros to a number of digits",
    },
    MacroSpec {
        code: 0x51,
        name: "ordinal",
        family: RuntimeContextValue,
        form: INLINE,
        args: &[inline("value", Number)],
        required: 1,
        repeats: false,
        summary: "an ordinal number, such as 1st",
    },
    MacroSpec {
        code: 0x60,
        name: "sound",
        family: LayoutTextualControl,
        form: INLINE,
        args: &[inline("kind", Other), inline("sound", Other)],
        required: 2,
        repeats: false,
        summary: "plays a sound effect",
    },
    MacroSpec {
        code: 0x61,
        name: "level-pos",
        family: GameDataReference,
        form: INLINE,
        args: &[inline("position", Row)],
        required: 1,
        repeats: false,
        summary: "a map position from the Level sheet",
    },
];

/// The entry of a macro code.
#[must_use]
pub fn by_code(code: u8) -> Option<&'static MacroSpec> {
    MACROS.iter().find(|spec| spec.code == code)
}

/// The entry of a tag name.
#[must_use]
pub fn by_name(name: &str) -> Option<&'static MacroSpec> {
    MACROS.iter().find(|spec| spec.name == name)
}

/// A nullary expression: a value the game supplies.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NullarySpec {
    pub code: u8,
    pub name: &'static str,
    pub summary: &'static str,
}

/// Every named nullary expression.
pub const NULLARY: &[NullarySpec] = &[
    NullarySpec {
        code: 0xD8,
        name: "msec",
        summary: "milliseconds of the set time",
    },
    NullarySpec {
        code: 0xD9,
        name: "sec",
        summary: "seconds of the set time",
    },
    NullarySpec {
        code: 0xDA,
        name: "min",
        summary: "minutes of the set time",
    },
    NullarySpec {
        code: 0xDB,
        name: "hour",
        summary: "hour of the set time, 0–23",
    },
    NullarySpec {
        code: 0xDC,
        name: "day",
        summary: "day of the month of the set time",
    },
    NullarySpec {
        code: 0xDD,
        name: "weekday",
        summary: "day of the week of the set time, 1 for Sunday",
    },
    NullarySpec {
        code: 0xDE,
        name: "month",
        summary: "month of the set time",
    },
    NullarySpec {
        code: 0xDF,
        name: "year",
        summary: "year of the set time",
    },
    NullarySpec {
        code: 0xEC,
        name: "stackcolor",
        summary: "the previous color",
    },
];

/// A parameter expression: a value passed to the string.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ParameterSpec {
    pub code: u8,
    /// The prefix after `$`, followed by the parameter number.
    pub prefix: &'static str,
    pub summary: &'static str,
}

/// Every parameter kind. Longer prefixes come first so `$gn` is not read
/// as `$g`.
pub const PARAMETERS: &[ParameterSpec] = &[
    ParameterSpec {
        code: 0xE9,
        prefix: "gn",
        summary: "global number",
    },
    ParameterSpec {
        code: 0xEB,
        prefix: "gs",
        summary: "global text",
    },
    ParameterSpec {
        code: 0xE8,
        prefix: "n",
        summary: "number parameter",
    },
    ParameterSpec {
        code: 0xEA,
        prefix: "s",
        summary: "text parameter",
    },
];

/// A global parameter whose meaning the game strings establish.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GlobalSpec {
    /// `gn` or `gs`.
    pub prefix: &'static str,
    pub index: u32,
    /// A stable name, such as `class-job`.
    pub name: &'static str,
    /// The sheet whose row the value is, when it is one.
    pub sheet: Option<&'static str>,
    pub summary: &'static str,
}

/// Global parameters with an established meaning. Each entry rests on how
/// game strings use it: what it is compared with and what the branches say.
/// Other globals are shown by their code.
pub const GLOBALS: &[GlobalSpec] = &[
    // "<if ($gs1 == $gs3)>your<else>…'s" and "<split " " 1><string $gs1></split>, it's you!"
    GlobalSpec {
        prefix: "gs",
        index: 1,
        name: "player-name",
        sheet: None,
        summary: "the name of the player character",
    },
    // Messages about other people, such as the party and battle log, name
    // them with $gs2 and $gs3 and compare each with $gs1: "<if ($gs1 == $gs2)>
    // you<else><if $gn7><noun-en ObjStr 2 $gn7 1 1><else>{$gs2}</if></if>
    // invites you to a party" (about 1,000 strings, $gs3 about 650).
    GlobalSpec {
        prefix: "gs",
        index: 2,
        name: "other-name",
        sheet: None,
        summary: "the name of the person or thing a message is about, such as who joins the party; \
                  it is the player character when it equals $gs1",
    },
    GlobalSpec {
        prefix: "gs",
        index: 3,
        name: "second-other-name",
        sheet: None,
        summary: "the name of a second person or thing a message is about; it is the player \
                  character when it equals $gs1",
    },
    // "A <if $gn4>woman<else>man</if> your size"
    GlobalSpec {
        prefix: "gn",
        index: 4,
        name: "player-female",
        sheet: None,
        summary: "1 when the player character is female, 0 when male",
    },
    // "<if $gn7><if \"<sheet BNpcName $gn7 6>\">her<else>his</if><else><if
    // $gn5>her<else>his</if></if>": for a player named by $gs2 (116 strings);
    // $gn6 does the same for $gs3.
    GlobalSpec {
        prefix: "gn",
        index: 5,
        name: "other-female",
        sheet: None,
        summary: "1 when the player named by $gs2 is female, 0 when male",
    },
    GlobalSpec {
        prefix: "gn",
        index: 6,
        name: "second-other-female",
        sheet: None,
        summary: "1 when the player named by $gs3 is female, 0 when male",
    },
    // "<if $gn7><noun-en ObjStr 2 $gn7 1 1><else>{$gs2}</if>": set when $gs2
    // is not a player (about 1,000 strings); $gn8 does the same for $gs3.
    GlobalSpec {
        prefix: "gn",
        index: 7,
        name: "other-object",
        sheet: Some("ObjStr"),
        summary: "the ObjStr row of what $gs2 names when it is not a player, such as a character \
                  or an object; 0 for a player",
    },
    GlobalSpec {
        prefix: "gn",
        index: 8,
        name: "second-other-object",
        sheet: Some("ObjStr"),
        summary: "the ObjStr row of what $gs3 names when it is not a player, such as a character \
                  or an object; 0 for a player",
    },
    // "<if ($gn52 > 0)>Storm<else><if ($gn53 > 0)>Serpent<else>Flame"
    // and "<sheet GCRankLimsaMaleText $gn52 8>"
    GlobalSpec {
        prefix: "gn",
        index: 52,
        name: "rank-maelstrom",
        sheet: None,
        summary: "the player's rank in the Maelstrom, 0 when not a member",
    },
    GlobalSpec {
        prefix: "gn",
        index: 53,
        name: "rank-twin-adder",
        sheet: None,
        summary: "the player's rank in the Order of the Twin Adder, 0 when not a member",
    },
    GlobalSpec {
        prefix: "gn",
        index: 54,
        name: "rank-immortal-flames",
        sheet: None,
        summary: "the player's rank in the Immortal Flames, 0 when not a member",
    },
    // Compared with ClassJob rows: "<if ($gn68 == 17)>… class is changed to botanist"
    GlobalSpec {
        prefix: "gn",
        index: 68,
        name: "class-job",
        sheet: Some("ClassJob"),
        summary: "the player's current class or job, a ClassJob row",
    },
    // "A <if ($gn71 == 3)>… man your size─the beasts would swallow you whole": Race 3 is Lalafell
    GlobalSpec {
        prefix: "gn",
        index: 71,
        name: "race",
        sheet: Some("Race"),
        summary: "the player character's race, a Race row",
    },
    // Compared with trait levels in action descriptions: "<if ($gn72 >= 94)>{220}"
    GlobalSpec {
        prefix: "gn",
        index: 72,
        name: "level",
        sheet: None,
        summary: "the level of the player's current class or job",
    },
];

/// The established meaning of a global parameter.
#[must_use]
pub fn global(prefix: &str, index: u32) -> Option<&'static GlobalSpec> {
    GLOBALS
        .iter()
        .find(|spec| spec.prefix == prefix && spec.index == index)
}

/// A construct of several macros whose meaning game strings establish,
/// written exactly as `text` in macro text. It is still those macros: it only
/// reads as one value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IdiomSpec {
    /// A stable name, such as `player-first-name`.
    pub name: &'static str,
    /// The exact macro text, as it prints.
    pub text: &'static str,
    pub summary: &'static str,
}

/// Every idiom, most frequent first.
pub const IDIOMS: &[IdiomSpec] = &[
    // `$gs1` is the player's name, "First Last"; `<split>` takes one part of
    // it: "<split " " 1><string $gs1></split>, it's you!" (13,280 times).
    IdiomSpec {
        name: "player-first-name",
        text: r#"<split " " 1><string $gs1></split>"#,
        summary: "the first name of the player character",
    },
    // "Ah, <split " " 2><string $gs1></split>" as a family name (850 times).
    IdiomSpec {
        name: "player-last-name",
        text: r#"<split " " 2><string $gs1></split>"#,
        summary: "the last name of the player character",
    },
];

/// What the game reads about the people of a message, by the globals the
/// message sets, which a translation may test although its source does not:
/// `<if $gn7><if "<sheet BNpcName $gn7 6>">her<else>his</if><else>…` in
/// about 120 strings tells whether the character `$gs2` names is female.
pub const PERSON_READS: &[IdiomSpec] = &[
    IdiomSpec {
        name: "other-character-female",
        text: "<sheet BNpcName $gn7 6>",
        summary: "1 when the character $gs2 names ($gn7 set) is female",
    },
    IdiomSpec {
        name: "second-other-character-female",
        text: "<sheet BNpcName $gn8 6>",
        summary: "1 when the character $gs3 names ($gn8 set) is female",
    },
];

/// The idiom `text` is written as, if any.
#[must_use]
pub fn idiom(text: &str) -> Option<&'static IdiomSpec> {
    IDIOMS.iter().find(|spec| spec.text == text)
}

/// How an insertion is written around the selection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InsertionForm {
    /// `parts[0]` replaces the selection.
    Insert,
    /// The selection goes between `parts[0]` and `parts[1]`.
    Wrap,
    /// The selection goes into both branches: `parts[0]`, selection,
    /// `parts[1]`, selection, `parts[2]`.
    Branches,
}

/// A macro a person can insert while translating, in the form game strings
/// use for it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InsertionSpec {
    /// A stable name, such as `player-full-name`.
    pub name: &'static str,
    /// The menu group: `player`, `choice`, or `format`.
    pub group: &'static str,
    pub form: InsertionForm,
    /// The macro text around the selection; `{row}` is a row of `rows`.
    pub parts: &'static [&'static str],
    /// The sheet whose rows the insertion is offered for, such as `Race`.
    pub rows: Option<&'static str>,
    pub summary: &'static str,
}

/// Every insertion, in menu order. Each is a form the game's dialogue uses:
/// `<string $gs1>` in 13,553 strings, the first name in 12,368, the gender
/// choice in 6,505, the class or job in 504, the last name in 742, the race
/// choice by `<switch $gn71>` in 109 and by `($gn71 == …)`, and class
/// choices by `($gn68 == …)`.
pub const INSERTIONS: &[InsertionSpec] = &[
    InsertionSpec {
        name: "player-full-name",
        group: "player",
        form: InsertionForm::Insert,
        parts: &["<string $gs1>"],
        rows: None,
        summary: "the full name of the player character",
    },
    InsertionSpec {
        name: "player-first-name",
        group: "player",
        form: InsertionForm::Insert,
        parts: &[r#"<split " " 1><string $gs1></split>"#],
        rows: None,
        summary: "the first name of the player character",
    },
    InsertionSpec {
        name: "player-last-name",
        group: "player",
        form: InsertionForm::Insert,
        parts: &[r#"<split " " 2><string $gs1></split>"#],
        rows: None,
        summary: "the last name of the player character",
    },
    InsertionSpec {
        name: "player-class-job",
        group: "player",
        form: InsertionForm::Insert,
        parts: &["<sheet ClassJob $gn68 0>"],
        rows: None,
        summary: "the name of the player's current class or job",
    },
    InsertionSpec {
        name: "player-race",
        group: "player",
        form: InsertionForm::Insert,
        parts: &["<sheet Race $gn71 0>"],
        rows: None,
        summary: "the name of the player character's race",
    },
    InsertionSpec {
        name: "gender-choice",
        group: "choice",
        form: InsertionForm::Branches,
        parts: &["<if $gn4>", "<else>", "</if>"],
        rows: None,
        summary: "a female form, then a male form, by the player character's gender",
    },
    InsertionSpec {
        name: "race-choice",
        group: "choice",
        form: InsertionForm::Branches,
        parts: &["<if ($gn71 == {row})>", "<else>", "</if>"],
        rows: Some("Race"),
        summary: "one form for a race of the player character, another otherwise",
    },
    InsertionSpec {
        name: "class-job-choice",
        group: "choice",
        form: InsertionForm::Branches,
        parts: &["<if ($gn68 == {row})>", "<else>", "</if>"],
        rows: Some("ClassJob"),
        summary: "one form for a class or job of the player, another otherwise",
    },
    InsertionSpec {
        name: "italic",
        group: "format",
        form: InsertionForm::Wrap,
        parts: &["<i>", "</i>"],
        rows: None,
        summary: "italic text, as game strings write titles",
    },
    InsertionSpec {
        name: "capitalize",
        group: "format",
        form: InsertionForm::Wrap,
        parts: &["<capitalize>", "</capitalize>"],
        rows: None,
        summary: "the first letter of the text capitalized",
    },
    InsertionSpec {
        name: "nbsp",
        group: "format",
        form: InsertionForm::Insert,
        parts: &["<nbsp>"],
        rows: None,
        summary: "a space the line never breaks at",
    },
];

/// A comparison operator.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ComparisonSpec {
    pub code: u8,
    pub operator: &'static str,
}

/// Every comparison. Two-character operators come first.
pub const COMPARISONS: &[ComparisonSpec] = &[
    ComparisonSpec {
        code: 0xE0,
        operator: ">=",
    },
    ComparisonSpec {
        code: 0xE2,
        operator: "<=",
    },
    ComparisonSpec {
        code: 0xE4,
        operator: "==",
    },
    ComparisonSpec {
        code: 0xE5,
        operator: "!=",
    },
    ComparisonSpec {
        code: 0xE1,
        operator: ">",
    },
    ComparisonSpec {
        code: 0xE3,
        operator: "<",
    },
];

/// Words that end a block or separate its content and are therefore not
/// macro names.
pub const STRUCTURE_WORDS: &[&str] = &["else", "case", "rt"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_insertion_is_well_formed_and_reads_known_globals() {
        for spec in INSERTIONS {
            let text = spec.parts.join("text").replace("{row}", "1");
            let document = crate::parse(&text);
            assert!(document.is_well_formed(), "{}: {text}", spec.name);
            let printed = crate::print(&document.to_nodes().expect("well formed"));
            assert_eq!(printed, text, "{} is written as it prints", spec.name);
            for parameter in ["$gs1", "$gn4", "$gn68", "$gn71"] {
                if text.contains(parameter) {
                    let (prefix, index) = parameter[1..].split_at(2);
                    assert!(
                        global(prefix, index.parse().expect("index")).is_some(),
                        "{parameter} has an established meaning"
                    );
                }
            }
        }
    }

    #[test]
    fn every_idiom_is_canonical_macro_text() {
        for spec in IDIOMS {
            let document = crate::parse(spec.text);
            assert!(document.is_well_formed(), "{}", spec.text);
            let printed = crate::print(&document.to_nodes().expect("well formed"));
            assert_eq!(printed, spec.text, "an idiom is written as it prints");
            assert_eq!(idiom(spec.text), Some(spec));
        }
    }

    #[test]
    fn codes_and_names_are_unique_and_well_formed() {
        for (index, spec) in MACROS.iter().enumerate() {
            assert!(
                MACROS[..index].iter().all(|other| other.code != spec.code),
                "duplicate code {:#04x}",
                spec.code
            );
            assert!(
                MACROS[..index].iter().all(|other| other.name != spec.name),
                "duplicate name {}",
                spec.name
            );
            assert!(
                spec.name
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'),
                "{}",
                spec.name
            );
            assert!(!STRUCTURE_WORDS.contains(&spec.name), "{}", spec.name);
            assert!(
                spec.required <= spec.args.len() || spec.repeats,
                "{}",
                spec.name
            );
        }
    }

    #[test]
    fn forms_match_their_arguments() {
        for spec in MACROS {
            let blocks = spec
                .args
                .iter()
                .filter(|arg| arg.place == Place::Block)
                .count();
            match spec.form {
                Form::Inline => assert_eq!(blocks, 0, "{}", spec.name),
                Form::Pair { .. } => {
                    assert_eq!(spec.args.len(), 1, "{}", spec.name);
                    assert_eq!(spec.required, 1, "{}", spec.name);
                }
                Form::Block { separator, .. } => {
                    assert!(blocks >= 1, "{}", spec.name);
                    assert!(blocks == 1 || separator.is_some(), "{}", spec.name);
                    if spec.repeats {
                        assert_eq!(spec.args.last().map(|arg| arg.place), Some(Place::Block));
                    }
                }
            }
        }
    }
}
