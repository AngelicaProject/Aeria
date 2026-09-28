//! The dialogue flow of quest scripts.
//!
//! Every quest sheet `quest/…/<ID>` has a compiled Lua script at
//! `game_script/quest/…/<ID>.luab`. Its scene functions, `OnScene00000`,
//! `OnScene00001`, …, play the quest's lines with `Talk` and similar calls,
//! offer the quest and ask the player's choices, and branch on the answers
//! and on facts about the player. This module traces each scene function
//! into a tree of lines, choices, conditions, and loops. See
//! `docs/architecture/source.md`.
//!
//! The flow is context for translators: it never affects identity,
//! permission, or persisted data.

mod cutscene;
mod flow;
mod lua;

pub use cutscene::{CutsceneError, cutscene_keys};
pub use lua::{ChunkError, Constant};

use lua::{Function, Instruction, Op};

/// The traced scenes of a quest script, in scene order.
#[derive(Clone, Debug, PartialEq)]
pub struct QuestScript {
    pub scenes: Vec<SceneFlow>,
}

/// One scene function, or another function of the script with lines.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneFlow {
    /// The number of `OnScene<number>`; `None` for another function.
    pub scene: Option<u32>,
    /// The name another function is assigned to, such as
    /// `GetBalloonTalkArgs`; `None` for a scene or an unnamed function.
    pub handler: Option<String>,
    /// The battle script it belongs to, such as `ClsRog250Btl`; `None` for
    /// the quest's own script.
    pub script: Option<String>,
    pub nodes: Vec<FlowNode>,
    /// `false` when the scene's code could not be traced: `nodes` then list
    /// its lines, choices, and markers in code order, without branches.
    pub traced: bool,
}

/// One step of a scene.
#[derive(Clone, Debug, PartialEq)]
pub enum FlowNode {
    /// A line, by its row key, such as `TEXT_…_LUCIANE_000_10`.
    Line {
        key: String,
    },
    Choice(Choice),
    /// Code that runs `then` when the condition holds and `otherwise` when
    /// it does not.
    Branch {
        condition: Guard,
        then: Vec<FlowNode>,
        otherwise: Vec<FlowNode>,
    },
    /// A body that can run again: a [`FlowNode::Repeat`] inside returns to
    /// its start.
    Loop {
        id: u32,
        body: Vec<FlowNode>,
    },
    Repeat {
        loop_id: u32,
    },
    /// A cutscene plays, named by the quest's script variable, such as
    /// `CUT_SCENE_01`, with the lines its file names (see
    /// [`GameSource::quest_script`](crate::GameSource::quest_script)).
    Cutscene {
        name: Option<String>,
        /// The `Cutscene` row the variable holds and the row's path, such
        /// as `ex4/aktkmm/aktkmm10330/aktkmm10330`, when it resolves.
        row: Option<u32>,
        path: Option<String>,
        /// Keys of the quest sheet's lines, in row order.
        lines: Vec<String>,
        /// Other dialogue sheets with lines the cutscene names, in
        /// sheet-name order.
        sheets: Vec<String>,
    },
    /// The scene ends without progress, and the player must talk again.
    Cancelled,
    Accepted,
    Completed,
}

/// What a cutscene variable resolves to.
pub(crate) struct ResolvedCutscene {
    pub row: u32,
    pub path: String,
    pub lines: Vec<String>,
    pub sheets: Vec<String>,
}

/// A question to the player.
#[derive(Clone, Debug, PartialEq)]
pub struct Choice {
    /// Unique within its scene; conditions on the answer refer to it.
    pub id: u32,
    pub kind: ChoiceKind,
    /// The keys of the question's line: several when the script chooses the
    /// question earlier, as in `local q = A; if … then q = B end`.
    pub prompts: Vec<String>,
    /// The answers, with what each leads to. A menu whose answers are all
    /// spread from a list at run time, as in `Menu(q, unpack(answers))`, has
    /// none.
    pub options: Vec<ChoiceOption>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChoiceKind {
    /// Accept or decline the quest.
    QuestOffer,
    YesNo,
    Menu,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChoiceOption {
    pub label: OptionLabel,
    /// When the player can pick it; `GrayoutMenu` shows the others grayed
    /// out.
    pub available: Availability,
    pub then: Vec<FlowNode>,
}

/// When an answer can be picked.
#[derive(Clone, Debug, PartialEq)]
pub enum Availability {
    Always,
    /// Shown grayed out.
    Never,
    /// Only when the guard holds, as when a script asks one menu with the
    /// answer grayed out and the same menu without.
    When(Guard),
    /// The script passes a flag whose value is not known.
    Unknown,
}

/// How an answer is labelled in the game.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OptionLabel {
    Accept,
    Decline,
    Yes,
    No,
    /// The key of the answer's line.
    Text(String),
    /// An answer the script passes from elsewhere, such as an entry of a
    /// list it built.
    Script,
}

/// What a branch tests: one condition, or several joined as the script
/// joins them with `or` and `and`.
#[derive(Clone, Debug, PartialEq)]
pub enum Guard {
    Test(Condition),
    /// Any of them holds.
    Any(Vec<Guard>),
    /// All of them hold.
    All(Vec<Guard>),
}

impl Guard {
    /// The opposite guard: each test inverted, `Any` and `All` swapped.
    #[must_use]
    pub fn negated(self) -> Self {
        match self {
            Self::Test(condition) => Self::Test(condition.negated()),
            Self::Any(guards) => Self::All(guards.into_iter().map(Self::negated).collect()),
            Self::All(guards) => Self::Any(guards.into_iter().map(Self::negated).collect()),
        }
    }
}

/// A quest a script variable names, such as `QUEST0 = 68707`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QuestReference {
    /// The script variable, such as `QUEST0`.
    pub variable: String,
    /// The quest's `Quest` row.
    pub row: u32,
    /// The quest's name, when its row has one.
    pub name: Option<String>,
    /// The quest's sheet, when it has one.
    pub sheet: Option<String>,
}

/// Script functions whose first argument is a quest.
pub const QUEST_FUNCTIONS: [&str; 3] = [
    "IsQuestCompleted",
    "IsQuestAccepted",
    "IsQuestAcceptQualified",
];

/// A condition a branch tests.
#[derive(Clone, Debug, PartialEq)]
pub struct Condition {
    pub subject: Operand,
    pub test: Test,
}

impl Condition {
    /// The condition that holds exactly when this one does not.
    #[must_use]
    pub fn negated(self) -> Self {
        let test = match self.test {
            Test::Truthy(value) => Test::Truthy(!value),
            Test::Compare(comparison, value) => Test::Compare(comparison.negated(), value),
        };
        Self {
            subject: self.subject,
            test,
        }
    }
}

impl Comparison {
    /// The comparison that holds exactly when this one does not.
    #[must_use]
    pub const fn negated(self) -> Self {
        match self {
            Self::Eq => Self::Ne,
            Self::Ne => Self::Eq,
            Self::Lt => Self::Ge,
            Self::Ge => Self::Lt,
            Self::Le => Self::Gt,
            Self::Gt => Self::Le,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Test {
    /// The subject is truthy (`true`) or not (`false`).
    Truthy(bool),
    Compare(Comparison, Operand),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Comparison {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

/// A value a condition tests.
#[derive(Clone, Debug, PartialEq)]
pub enum Operand {
    /// The answer to the choice with this id.
    Answer(u32),
    /// The result of a call, such as `GetSex` or `IsQuestCompleted`, with
    /// the arguments that are known.
    Call {
        function: Option<String>,
        arguments: Vec<Operand>,
    },
    Constant(Constant),
    /// A field, usually a constant of the quest such as `SEQ_1`.
    Field(String),
    /// A quest named by one of the quest's script variables (see
    /// [`GameSource::quest_script`](crate::GameSource::quest_script)).
    Quest(QuestReference),
    Global(String),
    /// One of several values, depending on the path taken to the test, such
    /// as either call of `A() or B()`.
    OneOf(Vec<Operand>),
    /// A value the script computes in a way tracing does not follow.
    Unknown,
}

impl QuestScript {
    /// Reads a compiled quest script and traces its scene functions. Scenes
    /// without lines, choices, or markers are left out.
    ///
    /// # Errors
    ///
    /// Returns an error when the data is not a supported compiled script.
    /// The script variables its scenes pass to `PlayCutScene`, each once.
    #[must_use]
    pub fn cutscene_names(&self) -> Vec<String> {
        let mut names = Vec::new();
        for scene in &self.scenes {
            visit(&scene.nodes, &mut |node| {
                if let FlowNode::Cutscene {
                    name: Some(name), ..
                } = node
                    && !names.contains(name)
                {
                    names.push(name.clone());
                }
            });
        }
        names
    }

    /// Fills each cutscene's row, path, lines, and sheets from `resolved`,
    /// by variable name.
    pub(crate) fn set_cutscenes(
        &mut self,
        resolved: &std::collections::HashMap<String, ResolvedCutscene>,
    ) {
        for scene in &mut self.scenes {
            visit_mut(&mut scene.nodes, &mut |node| {
                if let FlowNode::Cutscene {
                    name: Some(name),
                    row,
                    path,
                    lines,
                    sheets,
                } = node
                    && let Some(found) = resolved.get(name)
                {
                    *row = Some(found.row);
                    *path = Some(found.path.clone());
                    lines.clone_from(&found.lines);
                    sheets.clone_from(&found.sheets);
                }
            });
        }
    }

    /// The `Cutscene` rows its scenes play, with the index of the scene
    /// that plays each, once per scene.
    pub(crate) fn cutscene_rows(&self) -> Vec<(usize, u32)> {
        let mut rows = Vec::new();
        for (index, scene) in self.scenes.iter().enumerate() {
            visit(&scene.nodes, &mut |node| {
                if let FlowNode::Cutscene { row: Some(row), .. } = node
                    && !rows.contains(&(index, *row))
                {
                    rows.push((index, *row));
                }
            });
        }
        rows
    }

    /// The script variables its conditions pass as the quest of a quest
    /// function, such as `QUEST0` in `IsQuestCompleted(QUEST0)`, each once.
    #[must_use]
    pub fn quest_variables(&self) -> Vec<String> {
        let mut names = Vec::new();
        for scene in &self.scenes {
            visit(&scene.nodes, &mut |node| {
                if let FlowNode::Branch { condition, .. } = node {
                    guard_operands(condition, &mut |operand| {
                        if let Some(name) = quest_argument(operand)
                            && !names.contains(name)
                        {
                            names.push(name.clone());
                        }
                    });
                }
            });
        }
        names
    }

    /// Replaces the quest variables of quest functions with the quests in
    /// `resolved`, by variable name.
    pub fn set_quests(&mut self, resolved: &std::collections::HashMap<String, QuestReference>) {
        for scene in &mut self.scenes {
            visit_mut(&mut scene.nodes, &mut |node| {
                if let FlowNode::Branch { condition, .. } = node {
                    guard_operands_mut(condition, &mut |operand| {
                        if let Operand::Call {
                            function: Some(function),
                            arguments,
                        } = operand
                            && QUEST_FUNCTIONS.contains(&function.as_str())
                            && let Some(Operand::Field(name)) = arguments.first()
                            && let Some(quest) = resolved.get(name)
                        {
                            arguments[0] = Operand::Quest(quest.clone());
                        }
                    });
                }
            });
        }
    }

    /// Reads a compiled quest script; see [`QuestScript`].
    ///
    /// # Errors
    ///
    /// Returns an error when the data is not a supported compiled script.
    pub fn read(data: &[u8]) -> Result<Self, ChunkError> {
        let main = lua::read_chunk(data)?;
        let mut functions = Vec::new();
        collect_functions(&main, &mut functions);
        // Scenes in scene order, then the other functions in code order.
        let mut scenes: Vec<(u32, &Function)> = Vec::new();
        let mut handlers: Vec<(Option<String>, &Function)> = Vec::new();
        for (name, function) in functions {
            match name.as_deref().and_then(scene_number) {
                Some(scene) => scenes.push((scene, function)),
                None => handlers.push((name, function)),
            }
        }
        scenes.sort_by_key(|(scene, _)| *scene);
        scenes.dedup_by_key(|(scene, _)| *scene);
        let trace = |scene: Option<u32>, handler: Option<String>, function: &Function| {
            let (nodes, traced) = match flow::trace(function) {
                Ok(nodes) => (nodes, true),
                Err(flow::Untraceable) => (flow::listing(function), false),
            };
            (!nodes.is_empty()).then_some(SceneFlow {
                scene,
                handler,
                script: None,
                nodes,
                traced,
            })
        };
        Ok(Self {
            scenes: scenes
                .into_iter()
                .filter_map(|(scene, function)| trace(Some(scene), None, function))
                .chain(
                    handlers
                        .into_iter()
                        .filter_map(|(name, function)| trace(None, name, function)),
                )
                .collect(),
        })
    }
}

/// The quest variable a quest function is called with, such as `QUEST0` in
/// `IsQuestCompleted(QUEST0)`.
fn quest_argument(operand: &Operand) -> Option<&String> {
    match operand {
        Operand::Call {
            function: Some(function),
            arguments,
        } if QUEST_FUNCTIONS.contains(&function.as_str()) => match arguments.first() {
            Some(Operand::Field(name)) => Some(name),
            _ => None,
        },
        _ => None,
    }
}

/// Every operand a guard tests, outermost first, including one-of values.
fn guard_operands(guard: &Guard, action: &mut impl FnMut(&Operand)) {
    fn operand(value: &Operand, action: &mut impl FnMut(&Operand)) {
        action(value);
        if let Operand::OneOf(values) = value {
            for inner in values {
                operand(inner, action);
            }
        }
    }
    match guard {
        Guard::Test(condition) => {
            operand(&condition.subject, action);
            if let Test::Compare(_, value) = &condition.test {
                operand(value, action);
            }
        }
        Guard::Any(guards) | Guard::All(guards) => {
            for inner in guards {
                guard_operands(inner, action);
            }
        }
    }
}

fn guard_operands_mut(guard: &mut Guard, action: &mut impl FnMut(&mut Operand)) {
    fn operand(value: &mut Operand, action: &mut impl FnMut(&mut Operand)) {
        action(value);
        if let Operand::OneOf(values) = value {
            for inner in values {
                operand(inner, action);
            }
        }
    }
    match guard {
        Guard::Test(condition) => {
            operand(&mut condition.subject, action);
            if let Test::Compare(_, value) = &mut condition.test {
                operand(value, action);
            }
        }
        Guard::Any(guards) | Guard::All(guards) => {
            for inner in guards {
                guard_operands_mut(inner, action);
            }
        }
    }
}

fn children(node: &FlowNode) -> Vec<&Vec<FlowNode>> {
    match node {
        FlowNode::Choice(choice) => choice.options.iter().map(|option| &option.then).collect(),
        FlowNode::Branch {
            then, otherwise, ..
        } => vec![then, otherwise],
        FlowNode::Loop { body, .. } => vec![body],
        _ => Vec::new(),
    }
}

fn visit(nodes: &[FlowNode], action: &mut impl FnMut(&FlowNode)) {
    for node in nodes {
        action(node);
        for child in children(node) {
            visit(child, action);
        }
    }
}

fn visit_mut(nodes: &mut [FlowNode], action: &mut impl FnMut(&mut FlowNode)) {
    for node in nodes {
        action(node);
        match node {
            FlowNode::Choice(choice) => {
                for option in &mut choice.options {
                    visit_mut(&mut option.then, action);
                }
            }
            FlowNode::Branch {
                then, otherwise, ..
            } => {
                visit_mut(then, action);
                visit_mut(otherwise, action);
            }
            FlowNode::Loop { body, .. } => visit_mut(body, action),
            _ => {}
        }
    }
}

/// The number of `OnScene<number>`.
fn scene_number(name: &str) -> Option<u32> {
    name.strip_prefix("OnScene")
        .filter(|number| !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit()))
        .and_then(|number| number.parse().ok())
}

/// Every function below `function`, with the name it is assigned to:
/// `<table>.<name> = function … end` or a global `<name> = function … end`.
/// When a name is assigned several times, the last assignment is the one the
/// game calls, so the functions it replaces are left out. A function
/// assigned otherwise, such as to a local, has no name.
fn collect_functions<'a>(
    function: &'a Function,
    functions: &mut Vec<(Option<String>, &'a Function)>,
) {
    let mut closures = std::collections::HashMap::new();
    // Each name's function, and every function a name was ever given.
    let mut assigned: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    let mut named: std::collections::HashSet<u32> = std::collections::HashSet::new();
    for word in &function.code {
        let Some(instruction) = Instruction::decode(*word) else {
            continue;
        };
        let target = match instruction.op {
            Op::Closure => {
                closures.insert(instruction.a, instruction.bx);
                None
            }
            Op::SetTable
                if instruction.b >= lua::CONSTANT_BIT && instruction.c < lua::CONSTANT_BIT =>
            {
                function
                    .string(instruction.b - lua::CONSTANT_BIT)
                    .zip(closures.get(&instruction.c))
            }
            Op::SetGlobal => function
                .string(instruction.bx)
                .zip(closures.get(&instruction.a)),
            _ => None,
        };
        if let Some((name, index)) = target {
            assigned.insert(name.to_owned(), *index);
            named.insert(*index);
        }
    }
    let current: std::collections::HashMap<u32, String> = assigned
        .into_iter()
        .map(|(name, index)| (index, name))
        .collect();
    for (index, child) in function.functions.iter().enumerate() {
        let Ok(index) = u32::try_from(index) else {
            continue;
        };
        let name = current.get(&index).cloned();
        if name.is_none() && named.contains(&index) {
            // Replaced by a later assignment: never called.
            continue;
        }
        functions.push((name, child));
        collect_functions(child, functions);
    }
}

#[cfg(test)]
mod tests;
