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
