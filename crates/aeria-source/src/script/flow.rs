//! Traces the control flow of one compiled scene function.
//!
//! The function's instructions form a graph. Each conditional jump becomes a
//! branch whose two sides are traced up to the jump's immediate
//! post-dominator, where both sides meet again; each backward jump marks a
//! loop, traced up to the first post-dominator outside the loop's body, and
//! a jump back to a loop being traced is a repeat. Register contents are
//! followed along each path well enough to name called functions, the text
//! keys passed to them, and the values conditions compare.
//!
//! Nothing is guessed: code that does not fit this shape, or is too large,
//! is reported as untraceable and listed in instruction order instead.

use std::collections::{HashMap, HashSet};

use super::lua::{CONSTANT_BIT, Constant, Function, Instruction, Op};
use super::{
    Availability, Choice, ChoiceKind, ChoiceOption, Comparison, Condition, FlowNode, Guard,
    Operand, OptionLabel, Test,
};

/// Most instructions visited while tracing one function, counting revisits.
const MAX_STEPS: usize = 20_000;
/// Most nested branches and loops.
const MAX_DEPTH: usize = 200;

/// Calls that ask the player to choose, and whose result the script tests.
const QUEST_OFFER: &str = "QuestOffer";
const YES_NO: [&str; 2] = ["YesNo", "YesNoQuestBattle"];
const MENU: &str = "Menu";
/// A menu whose every answer is followed by a flag that grays it out.
const GRAYOUT_MENU: &str = "GrayoutMenu";
const MENU_FLAG_ENABLE: &str = "MENU_FLAG_ENABLE";
const MENU_FLAG_DISABLE: &str = "MENU_FLAG_DISABLE";

/// The code does not have a shape this module traces.
#[derive(Debug)]
pub struct Untraceable;

/// What a register holds, as far as tracing knows.
#[derive(Clone, Debug, PartialEq)]
enum Value {
    Unknown,
    Constant(Constant),
    /// A field of a table, such as `TEXT_…` or `SEQ_0` of the quest.
    Field(String),
    Global(String),
    /// A method looked up by `SELF`.
    Method(String),
    /// The result of a call, by its index.
    Call(usize),
    /// Different values on the paths that meet here, such as either call
    /// of `A() or B()`.
    OneOf(Vec<Value>),
    /// A boolean that is `true` exactly when the guard holds, such as
    /// `local ready = a and b` compiled to `true` and `false` on each path.
    /// The flag says whether it may be `nil` rather than `false` where the
    /// guard fails, as a local assigned on only some paths is.
    Truth(Box<Guard>, bool),
}

/// Most tests a computed boolean may combine before it counts as unknown.
const MAX_TRUTH_TESTS: usize = 64;

/// What a register holds where the two sides of a branch on `guard` meet,
/// when it is a boolean on both: `taken` where the guard holds, `other`
/// where it does not.
fn truth(guard: &Guard, taken: &Value, other: &Value) -> Option<Value> {
    // `nil` counts as `false`, remembering that it is not `false`.
    let read = |value: &Value| match value {
        Value::Constant(Constant::Boolean(true)) => Some((Bool::True, false)),
        Value::Constant(Constant::Boolean(false)) => Some((Bool::False, false)),
        Value::Constant(Constant::Nil) => Some((Bool::False, true)),
        Value::Truth(inner, nil) => Some((Bool::Holds((**inner).clone()), *nil)),
        _ => None,
    };
    let ((taken, taken_nil), (other, other_nil)) = (read(taken)?, read(other)?);
    let Bool::Holds(result) = select(guard, taken, other) else {
        return None;
    };
    let result = flatten(result);
    (tests(&result) <= MAX_TRUTH_TESTS)
        .then(|| Value::Truth(Box::new(result), taken_nil || other_nil))
}

/// A boolean as far as tracing knows it.
enum Bool {
    True,
    False,
    Holds(Guard),
}

/// The boolean that is `taken` where `guard` holds and `other` where it
/// does not. Tests both sides share are taken out, as the compiler repeats
/// the rest of an `or` or `and` chain on both sides of each test in it:
/// `(h and a and x) or (not h and a and y)` is `a and (h and x or not h and
/// y)`, and `(h and (a or x)) or (not h and (a or y))` is `a or (h and x) or
/// (not h and y)`.
fn select(guard: &Guard, taken: Bool, other: Bool) -> Bool {
    let holds = || guard.clone();
    let fails = || guard.clone().negated();
    match (taken, other) {
        (Bool::True, Bool::True) => Bool::True,
        (Bool::False, Bool::False) => Bool::False,
        (Bool::True, Bool::False) => Bool::Holds(holds()),
        (Bool::False, Bool::True) => Bool::Holds(fails()),
        (Bool::Holds(inner), Bool::False) => Bool::Holds(all(holds(), inner)),
        (Bool::Holds(inner), Bool::True) => Bool::Holds(any(fails(), inner)),
        (Bool::True, Bool::Holds(inner)) => Bool::Holds(any(holds(), inner)),
        (Bool::False, Bool::Holds(inner)) => Bool::Holds(all(fails(), inner)),
        (Bool::Holds(left), Bool::Holds(right)) => {
            if left == right {
                return Bool::Holds(left);
            }
            let (common, left_rest, right_rest) = shared(conjuncts(left), conjuncts(right));
            if !common.is_empty() {
                let rest = select(
                    guard,
                    grouped(left_rest, Guard::All, Bool::True),
                    grouped(right_rest, Guard::All, Bool::True),
                );
                return match rest {
                    Bool::False => Bool::False,
                    Bool::True => grouped(common, Guard::All, Bool::True),
                    Bool::Holds(rest) => Bool::Holds(all(rest, Guard::All(common))),
                };
            }
            let (left, right) = (group(left_rest, Guard::All), group(right_rest, Guard::All));
            let (common, left_rest, right_rest) = shared(disjuncts(left), disjuncts(right));
            if !common.is_empty() {
                let rest = select(
                    guard,
                    grouped(left_rest, Guard::Any, Bool::False),
                    grouped(right_rest, Guard::Any, Bool::False),
                );
                return match rest {
                    Bool::True => Bool::True,
                    Bool::False => grouped(common, Guard::Any, Bool::False),
                    Bool::Holds(rest) => Bool::Holds(any(rest, Guard::Any(common))),
                };
            }
            let (left, right) = (group(left_rest, Guard::Any), group(right_rest, Guard::Any));
            Bool::Holds(Guard::Any(vec![all(holds(), left), all(fails(), right)]))
        }
    }
}

fn conjuncts(guard: Guard) -> Vec<Guard> {
    match guard {
        Guard::All(guards) => guards,
        other => vec![other],
    }
}

fn disjuncts(guard: Guard) -> Vec<Guard> {
    match guard {
        Guard::Any(guards) => guards,
        other => vec![other],
    }
}

/// The guards in both lists, then the rest of each.
fn shared(left: Vec<Guard>, right: Vec<Guard>) -> (Vec<Guard>, Vec<Guard>, Vec<Guard>) {
    let common: Vec<Guard> = left
        .iter()
        .filter(|guard| right.contains(guard))
        .cloned()
        .collect();
    let rest = |guards: Vec<Guard>| -> Vec<Guard> {
        guards
            .into_iter()
            .filter(|guard| !common.contains(guard))
            .collect()
    };
    let (left, right) = (rest(left), rest(right));
    (common, left, right)
}

/// One guard, or several joined; not empty.
fn group(mut guards: Vec<Guard>, join: fn(Vec<Guard>) -> Guard) -> Guard {
    if guards.len() == 1 {
        guards.remove(0)
    } else {
        join(guards)
    }
}

/// `group`, or `empty` when there are no guards.
fn grouped(guards: Vec<Guard>, join: fn(Vec<Guard>) -> Guard, empty: Bool) -> Bool {
    if guards.is_empty() {
        empty
    } else {
        Bool::Holds(group(guards, join))
    }
}

/// How many tests a guard combines.
fn tests(guard: &Guard) -> usize {
    match guard {
        Guard::Test(_) => 1,
        Guard::Any(guards) | Guard::All(guards) => guards.iter().map(tests).sum(),
    }
}

/// Merges nested `Any` into `Any` and `All` into `All`.
fn flatten(guard: Guard) -> Guard {
    match guard {
        Guard::Any(guards) => Guard::Any(
            guards
                .into_iter()
                .map(flatten)
                .flat_map(|inner| match inner {
                    Guard::Any(nested) => nested,
                    other => vec![other],
                })
                .collect(),
        ),
        Guard::All(guards) => Guard::All(
            guards
                .into_iter()
                .map(flatten)
                .flat_map(|inner| match inner {
                    Guard::All(nested) => nested,
                    other => vec![other],
                })
                .collect(),
        ),
        test @ Guard::Test(_) => test,
    }
}

/// Most values a register can hold on the paths that meet.
const MAX_ALTERNATIVES: usize = 4;

impl Value {
    /// What a register holds where two paths meet. Values computed from
    /// constants on each path, such as `true` or `false` from a condition,
    /// stay unknown.
    fn merge(self, other: Self) -> Self {
        if self == other {
            return self;
        }
        let mut values = Vec::new();
        for value in [self, other] {
            match value {
                Self::Unknown | Self::Truth(..) => return Self::Unknown,
                Self::OneOf(inner) => values.extend(inner),
                value => values.push(value),
            }
        }
        let mut unique: Vec<Self> = Vec::new();
        for value in values {
            if !unique.contains(&value) {
                unique.push(value);
            }
        }
        if unique.len() > MAX_ALTERNATIVES
            || unique
                .iter()
                .all(|value| matches!(value, Self::Constant(_)))
        {
            return Self::Unknown;
        }
        Self::OneOf(unique)
    }
}

type Registers = HashMap<u32, Value>;

struct CallInfo {
    /// Where the call is: a choice's id, the same on every path through it.
    pc: usize,
    function: Option<String>,
    arguments: Vec<Value>,
    /// The values each option of a choice returns.
    choice: Option<Vec<Constant>>,
}

/// Traces a scene function into flow nodes; `Err` when its code does not
/// have a shape this module traces.
pub fn trace(function: &Function) -> Result<Vec<FlowNode>, Untraceable> {
    let mut tracer = Tracer::new(function)?;
    let registers = (function.parameters..function.stack)
        .map(|register| (u32::from(register), Value::Constant(Constant::Nil)))
        .collect();
    let (nodes, _) = tracer.walk(0, None, registers, &[], 0, false)?;
    Ok(tracer.finish(nodes))
}

/// The function's text, choices, and markers in instruction order, without
/// branches, for code that cannot be traced.
pub fn listing(function: &Function) -> Vec<FlowNode> {
    let mut tracer = Tracer {
        function,
        code: function
            .code
            .iter()
            .map(|word| Instruction::decode(*word))
            .collect::<Option<Vec<_>>>()
            .unwrap_or_default(),
        successors: Vec::new(),
        post_dominators: Vec::new(),
        headers: HashSet::new(),
        loop_exits: HashMap::new(),
        steps: 0,
        calls: Vec::new(),
        open: None,
    };
    let mut registers = Registers::new();
    let mut nodes = Vec::new();
    for pc in 0..tracer.code.len() {
        let instruction = tracer.code[pc];
        tracer.effect(pc, instruction, &mut registers, &mut nodes);
    }
    tracer.finish(nodes)
}

struct Tracer<'a> {
    function: &'a Function,
    code: Vec<Instruction>,
    /// Where each instruction continues, through chains of jumps. A test
    /// continues at the target of its jump when it holds, and after the
    /// jump otherwise; a loop instruction at its body, then at its exit.
    successors: Vec<Vec<usize>>,
    post_dominators: Vec<Option<usize>>,
    headers: HashSet<usize>,
    loop_exits: HashMap<usize, Option<usize>>,
    steps: usize,
    calls: Vec<CallInfo>,
    /// The register of the last call or `...` whose results are left open
    /// (`C = 0`, `B = 0`), which a following call with `B = 0` passes as
    /// its last arguments.
    open: Option<u32>,
}

impl<'a> Tracer<'a> {
    fn new(function: &'a Function) -> Result<Self, Untraceable> {
        let code = function
            .code
            .iter()
            .map(|word| Instruction::decode(*word))
            .collect::<Option<Vec<_>>>()
            .ok_or(Untraceable)?;
        let successors = successors(&code)?;
        let post_dominators = post_dominators(&successors);
        let mut predecessors = vec![Vec::new(); code.len()];
        let mut back_edges: HashMap<usize, Vec<usize>> = HashMap::new();
        for (pc, targets) in successors.iter().enumerate() {
            for &target in targets {
                predecessors[target].push(pc);
                if target <= pc {
                    back_edges.entry(target).or_default().push(pc);
                }
            }
        }
        let mut loop_exits = HashMap::new();
        for (&header, sources) in &back_edges {
            // The natural loop: what reaches a back edge without passing
            // through the header.
            // Compiled loops are contiguous, from the header to the last
            // jump back. The range matters for `for`: `FORPREP` enters at the
            // `FORLOOP` at the end, so walking back from it would leave the
            // loop through the code before it.
            let last = sources.iter().copied().max().unwrap_or(header);
            let mut body = HashSet::from([header]);
            let mut pending = sources.clone();
            while let Some(pc) = pending.pop() {
                if (header..=last).contains(&pc) && body.insert(pc) {
                    pending.extend(&predecessors[pc]);
                }
            }
            let mut exit = post_dominators[header];
            while let Some(pc) = exit.filter(|pc| body.contains(pc)) {
                exit = post_dominators[pc];
            }
            loop_exits.insert(header, exit);
        }
        Ok(Self {
            function,
            code,
            successors,
            post_dominators,
            headers: back_edges.into_keys().collect(),
            loop_exits,
            steps: 0,
            calls: Vec::new(),
            open: None,
        })
    }

    #[allow(clippy::too_many_lines)]
    fn walk(
        &mut self,
        start: usize,
        stop: Option<usize>,
        mut registers: Registers,
        active: &[usize],
        depth: usize,
        loop_body: bool,
    ) -> Result<(Vec<FlowNode>, Option<Registers>), Untraceable> {
        if depth > MAX_DEPTH {
            return Err(Untraceable);
        }
        let mut nodes = Vec::new();
        let mut first = loop_body;
        let mut pc = start;
        loop {
            if Some(pc) == stop {
                return Ok((nodes, Some(registers)));
            }
            self.steps += 1;
            if self.steps > MAX_STEPS {
                return Err(Untraceable);
            }
            if active.contains(&pc) && !first {
                nodes.push(FlowNode::Repeat {
                    loop_id: loop_id(pc),
                });
                return Ok((nodes, None));
            }
            if self.headers.contains(&pc) && !active.contains(&pc) {
                let exit = self.loop_exits[&pc];
                let mut inner = active.to_vec();
                inner.push(pc);
                let (body, exit_registers) =
                    self.walk(pc, exit, registers.clone(), &inner, depth + 1, true)?;
                nodes.push(FlowNode::Loop {
                    id: loop_id(pc),
                    body,
                });
                registers = exit_registers.unwrap_or_default();
                match exit {
                    Some(exit) => {
                        pc = exit;
                        first = false;
                        continue;
                    }
                    None => return Ok((nodes, None)),
                }
            }
            first = false;
            let instruction = self.code[pc];
            match instruction.op {
                Op::Eq | Op::Lt | Op::Le | Op::Test | Op::TestSet => {
                    let merge = self.post_dominators[pc];
                    let Chain {
                        condition,
                        taken,
                        fallthrough,
                        taken_registers,
                        fallthrough_registers,
                    } = self.chain(pc, &registers, merge.or(stop), active);
                    let (then, then_registers) =
                        self.walk(taken, merge, taken_registers, active, depth + 1, false)?;
                    let (otherwise, otherwise_registers) = self.walk(
                        fallthrough,
                        merge,
                        fallthrough_registers,
                        active,
                        depth + 1,
                        false,
                    )?;
                    registers = match (then_registers, otherwise_registers) {
                        (Some(left), Some(mut right)) => left
                            .into_iter()
                            .filter_map(|(register, value)| {
                                let other = right.remove(&register)?;
                                let merged = truth(&condition, &value, &other)
                                    .unwrap_or_else(|| value.merge(other));
                                (merged != Value::Unknown).then_some((register, merged))
                            })
                            .collect(),
                        (Some(one), None) | (None, Some(one)) => one,
                        (None, None) => Registers::new(),
                    };
                    nodes.push(FlowNode::Branch {
                        condition,
                        then,
                        otherwise,
                    });
                    match merge {
                        Some(merge) => pc = merge,
                        None => return Ok((nodes, None)),
                    }
                }
                Op::ForLoop | Op::TForLoop => {
                    // Each round writes the loop's registers: `for` its counter
                    // and variable, `for … in` its control and variables.
                    let Instruction { a, c, .. } = instruction;
                    let written = if instruction.op == Op::ForLoop {
                        a..=a + 3
                    } else {
                        a + 2..=a + 2 + c
                    };
                    for register in written {
                        registers.remove(&register);
                    }
                    let back = self.successors[pc][0];
                    if active.contains(&back) {
                        // The end of one iteration of the loop being traced.
                        return Ok((nodes, Some(registers)));
                    }
                    pc = back;
                }
                Op::Return | Op::TailCall => {
                    self.effect(pc, instruction, &mut registers, &mut nodes);
                    return Ok((nodes, None));
                }
                _ => {
                    self.effect(pc, instruction, &mut registers, &mut nodes);
                    pc = self.successors[pc][0];
                }
            }
        }
    }

    /// The condition of the test at `pc` with the tests of `and` and `or`
    /// that follow it, and where the code goes when it holds and when not.
    /// `if a and b then X else Y` compiles to a test of `a` that leads to a
    /// test of `b` or to `Y`, and the test of `b` leads to `X` or the same
    /// `Y`; `or` is the same with the sides swapped. Joined here, `Y` is
    /// traced once: an `elseif` ladder of such conditions would otherwise
    /// trace the rest of the ladder twice at each rung.
    ///
    /// Code between the tests only computes the next test's values; a test
    /// of a choice's answer is not joined, so its branches can still move
    /// into the choice's options.
    fn chain(
        &mut self,
        pc: usize,
        registers: &Registers,
        stop: Option<usize>,
        active: &[usize],
    ) -> Chain {
        let mut chain = Chain {
            condition: self.guard(self.code[pc], registers),
            taken: self.successors[pc][0],
            fallthrough: self.successors[pc][1],
            taken_registers: registers.clone(),
            fallthrough_registers: registers.clone(),
        };
        if self.code[pc].op == Op::TestSet || tests_answer(&chain.condition) {
            return chain;
        }
        loop {
            if let Some((next, inner, registers)) =
                self.next_test(chain.taken, &chain.taken_registers, stop, active)
            {
                let (holds, fails) = (self.successors[next][0], self.successors[next][1]);
                if fails == chain.fallthrough || holds == chain.fallthrough {
                    let (inner, taken) = if fails == chain.fallthrough {
                        (inner, holds)
                    } else {
                        (inner.negated(), fails)
                    };
                    chain.condition = all(chain.condition, inner);
                    chain.taken = taken;
                    // Both tests fail to the same code, with what each path left.
                    chain.fallthrough_registers = merged(&chain.fallthrough_registers, &registers);
                    chain.taken_registers = registers;
                    continue;
                }
            }
            if let Some((next, inner, registers)) = self.next_test(
                chain.fallthrough,
                &chain.fallthrough_registers,
                stop,
                active,
            ) {
                let (holds, fails) = (self.successors[next][0], self.successors[next][1]);
                if holds == chain.taken || fails == chain.taken {
                    let (inner, fallthrough) = if holds == chain.taken {
                        (inner, fails)
                    } else {
                        (inner.negated(), holds)
                    };
                    chain.condition = any(chain.condition, inner);
                    chain.fallthrough = fallthrough;
                    // Both tests hold to the same code, as in `x = A() or B()`.
                    chain.taken_registers = merged(&chain.taken_registers, &registers);
                    chain.fallthrough_registers = registers;
                    continue;
                }
            }
            return chain;
        }
    }

    /// The test that code starting at `start` reaches after computing its
    /// values, with its condition and the registers there; `None` when the
    /// code does anything else first, such as show text, or leaves the
    /// region being traced.
    fn next_test(
        &mut self,
        start: usize,
        registers: &Registers,
        stop: Option<usize>,
        active: &[usize],
    ) -> Option<(usize, Guard, Registers)> {
        let open = self.open;
        let mut registers = registers.clone();
        let mut nodes = Vec::new();
        let mut pc = start;
        for _ in 0..MAX_CONDITION_CODE {
            if Some(pc) == stop || self.headers.contains(&pc) || active.contains(&pc) {
                break;
            }
            let instruction = self.code[pc];
            match instruction.op {
                Op::Eq | Op::Lt | Op::Le | Op::Test => {
                    let condition = self.guard(instruction, &registers);
                    if tests_answer(&condition) {
                        break;
                    }
                    return Some((pc, condition, registers));
                }
                Op::Move
                | Op::LoadK
                | Op::LoadBool
                | Op::LoadNil
                | Op::GetUpval
                | Op::GetGlobal
                | Op::GetTable
                | Op::Method
                | Op::Call => {
                    if instruction.op == Op::LoadBool && instruction.c != 0 {
                        break;
                    }
                    self.steps += 1;
                    self.effect(pc, instruction, &mut registers, &mut nodes);
                    if !nodes.is_empty() {
                        break;
                    }
                    pc = self.successors[pc][0];
                }
                _ => break,
            }
        }
        self.open = open;
        None
    }

    /// What a non-branching instruction does to the registers, and the flow
    /// nodes a call adds.
    fn effect(
        &mut self,
        pc: usize,
        instruction: Instruction,
        registers: &mut Registers,
        nodes: &mut Vec<FlowNode>,
    ) {
        let Instruction { a, b, c, bx, .. } = instruction;
        match instruction.op {
            Op::Method => {
                let method = self
                    .key_string(c, registers)
                    .map_or(Value::Unknown, Value::Method);
                let object = registers.get(&b).cloned().unwrap_or(Value::Unknown);
                registers.insert(a + 1, object);
                registers.insert(a, method);
            }
            Op::GetTable => {
                let field = self
                    .key_string(c, registers)
                    .map_or(Value::Unknown, Value::Field);
                registers.insert(a, field);
            }
            Op::GetGlobal => {
                let global = self
                    .function
                    .string(bx)
                    .map_or(Value::Unknown, |name| Value::Global(name.to_owned()));
                registers.insert(a, global);
            }
            Op::LoadK => {
                let constant = self
                    .function
                    .constant(bx)
                    .cloned()
                    .map_or(Value::Unknown, Value::Constant);
                registers.insert(a, constant);
            }
            Op::LoadBool => {
                registers.insert(a, Value::Constant(Constant::Boolean(b != 0)));
            }
            Op::LoadNil => {
                for register in a..=b {
                    registers.insert(register, Value::Constant(Constant::Nil));
                }
            }
            Op::Move => {
                let value = registers.get(&b).cloned().unwrap_or(Value::Unknown);
                registers.insert(a, value);
            }
            Op::Call | Op::TailCall => self.call(pc, instruction, registers, nodes),
            // Text a function returns or puts in a table is text it shows,
            // such as a balloon's line or a menu's labels.
            Op::Return | Op::SetList => {
                let count = if b == 0 { 0 } else { b - 1 };
                let first = if instruction.op == Op::Return {
                    a
                } else {
                    a + 1
                };
                let count = if instruction.op == Op::Return {
                    count
                } else {
                    b
                };
                for register in first..first + count {
                    if let Some(Value::Field(name)) = registers.get(&register)
                        && is_text_key(name)
                    {
                        nodes.push(FlowNode::Line { key: name.clone() });
                    }
                }
            }
            Op::SetTable => {
                let value = if c >= CONSTANT_BIT {
                    None
                } else {
                    registers.get(&c)
                };
                if let Some(Value::Field(name)) = value
                    && is_text_key(name)
                {
                    nodes.push(FlowNode::Line { key: name.clone() });
                }
            }
            Op::Jmp
            | Op::ForPrep
            | Op::SetGlobal
            | Op::SetUpval
            | Op::Close
            | Op::Eq
            | Op::Lt
            | Op::Le
            | Op::Test => {}
            _ => {
                if instruction.op == Op::VarArg && b == 0 {
                    self.open = Some(a);
                }
                registers.insert(a, Value::Unknown);
            }
        }
    }

    #[allow(clippy::too_many_lines)]
    fn call(
        &mut self,
        pc: usize,
        instruction: Instruction,
        registers: &mut Registers,
        nodes: &mut Vec<FlowNode>,
    ) {
        let Instruction { a, b, c, .. } = instruction;
        let callee = registers.get(&a).cloned().unwrap_or(Value::Unknown);
        let function = match &callee {
            Value::Method(name) | Value::Global(name) | Value::Field(name) => Some(name.clone()),
            _ => None,
        };
        // `B = 0` passes the registers up to the open results of the call
        // or `...` before it, which count as one argument.
        let count = if b == 0 {
            self.open
                .take()
                .filter(|top| *top > a)
                .map_or(1, |top| top - a + 1)
        } else {
            b
        };
        let mut arguments: Vec<Value> = (1..count)
            .map(|index| {
                registers
                    .get(&(a + index))
                    .cloned()
                    .unwrap_or(Value::Unknown)
            })
            .collect();
        if matches!(callee, Value::Method(_)) && !arguments.is_empty() {
            arguments.remove(0);
        }
        let text = |value: &Value| match value {
            Value::Field(name) if is_text_key(name) => Some(name.clone()),
            _ => None,
        };
        // Every text a value can be: one field, or the fields of a value
        // chosen earlier from several.
        let texts = |value: &Value| -> Vec<String> {
            match value {
                Value::OneOf(values) => values.iter().filter_map(text).collect(),
                other => text(other).into_iter().collect(),
            }
        };
        let prompts = arguments.first().map(texts).unwrap_or_default();
        let index = self.calls.len();
        let mut choice = None;
        match function.as_deref() {
            Some(QUEST_OFFER) => {
                choice = Some(vec![Constant::Boolean(true), Constant::Boolean(false)]);
                nodes.push(FlowNode::Choice(Choice {
                    id: choice_id(pc),
                    kind: ChoiceKind::QuestOffer,
                    prompts: Vec::new(),
                    options: vec![option(OptionLabel::Accept), option(OptionLabel::Decline)],
                }));
            }
            Some(name) if YES_NO.contains(&name) => {
                choice = Some(vec![Constant::Boolean(true), Constant::Boolean(false)]);
                let label = |position: usize, fallback: OptionLabel| {
                    arguments
                        .get(position)
                        .and_then(text)
                        .map_or(fallback, OptionLabel::Text)
                };
                nodes.push(FlowNode::Choice(Choice {
                    id: choice_id(pc),
                    kind: ChoiceKind::YesNo,
                    prompts,
                    options: vec![
                        option(label(1, OptionLabel::Yes)),
                        option(label(2, OptionLabel::No)),
                    ],
                }));
            }
            Some(name @ (MENU | GRAYOUT_MENU)) => {
                // Answers spread from a list (`B = 0`) cannot be counted.
                let answers: Vec<&Value> = if b == 0 {
                    Vec::new()
                } else {
                    arguments.iter().skip(1).collect()
                };
                let label =
                    |answer: &Value| text(answer).map_or(OptionLabel::Script, OptionLabel::Text);
                let options: Vec<ChoiceOption> = if name == GRAYOUT_MENU {
                    answers
                        .chunks(2)
                        .map(|pair| ChoiceOption {
                            label: label(pair[0]),
                            available: match pair.get(1) {
                                Some(Value::Field(flag)) if flag == MENU_FLAG_ENABLE => {
                                    Availability::Always
                                }
                                Some(Value::Field(flag)) if flag == MENU_FLAG_DISABLE => {
                                    Availability::Never
                                }
                                _ => Availability::Unknown,
                            },
                            then: Vec::new(),
                        })
                        .collect()
                } else {
                    answers
                        .into_iter()
                        .map(|answer| option(label(answer)))
                        .collect()
                };
                choice = Some(
                    (1..=options.len())
                        .map(|number| {
                            Constant::Number(f64::from(u32::try_from(number).unwrap_or(u32::MAX)))
                        })
                        .collect(),
                );
                nodes.push(FlowNode::Choice(Choice {
                    id: choice_id(pc),
                    kind: ChoiceKind::Menu,
                    prompts,
                    options,
                }));
            }
            Some("PlayCutScene") => nodes.push(FlowNode::Cutscene {
                name: arguments.iter().find_map(|value| match value {
                    Value::Field(name) => Some(name.clone()),
                    _ => None,
                }),
                row: None,
                path: None,
                lines: Vec::new(),
                sheets: Vec::new(),
            }),
            Some("CancelEventScene") => nodes.push(FlowNode::Cancelled),
            Some("QuestAccepted") => nodes.push(FlowNode::Accepted),
            Some("QuestCompleted") => nodes.push(FlowNode::Completed),
            // An argument chosen earlier from several texts shows one of
            // them; each is listed.
            _ => nodes.extend(
                arguments
                    .iter()
                    .flat_map(texts)
                    .map(|key| FlowNode::Line { key }),
            ),
        }
        self.calls.push(CallInfo {
            pc,
            function,
            arguments,
            choice,
        });
        // `C = 0` keeps every result; only the first can be followed.
        if c == 0 {
            self.open = Some(a);
        }
        let results = if c == 0 { 1 } else { c - 1 };
        for register in a..a + results {
            registers.insert(register, Value::Call(index));
        }
    }

    /// What a test instruction tests. A test of a computed boolean is the
    /// guard it was computed from.
    fn guard(&self, instruction: Instruction, registers: &Registers) -> Guard {
        let Instruction { a, b, c, .. } = instruction;
        let register = |index: u32| registers.get(&index);
        let truth = |value: Option<&Value>| match value {
            Some(Value::Truth(guard, nil)) => Some(((**guard).clone(), *nil)),
            _ => None,
        };
        let constant = |index: u32| {
            (index >= CONSTANT_BIT)
                .then(|| self.function.constant(index - CONSTANT_BIT))
                .flatten()
        };
        let oriented = |guard: Guard, holds: bool| if holds { guard } else { guard.negated() };
        match instruction.op {
            Op::Test => {
                if let Some((guard, _)) = truth(register(a)) {
                    return oriented(guard, c != 0);
                }
            }
            Op::TestSet => {
                if let Some((guard, _)) = truth(register(b)) {
                    return oriented(guard, c != 0);
                }
            }
            Op::Eq => {
                // `x == true` holds with `x`; `x == false` against it, unless
                // `x` may be `nil`; `A = 0` inverts the comparison.
                let sides = [(b, c), (c, b)];
                for (value, other) in sides {
                    if let (Some((guard, nil)), Some(Constant::Boolean(expected))) = (
                        truth(if value < CONSTANT_BIT {
                            register(value)
                        } else {
                            None
                        }),
                        constant(other),
                    ) && (*expected || !nil)
                    {
                        return oriented(guard, *expected == (a != 0));
                    }
                }
            }
            _ => {}
        }
        Guard::Test(self.condition(instruction, registers))
    }

    fn condition(&self, instruction: Instruction, registers: &Registers) -> Condition {
        let Instruction { a, b, c, .. } = instruction;
        let register = |index: u32| registers.get(&index).cloned().unwrap_or(Value::Unknown);
        let rk = |index: u32| {
            if index >= CONSTANT_BIT {
                self.function
                    .constant(index - CONSTANT_BIT)
                    .cloned()
                    .map_or(Value::Unknown, Value::Constant)
            } else {
                register(index)
            }
        };
        // `JMP` follows a test when the test's outcome equals `A` (or `C`
        // for `TEST`); that side is `then`.
        match instruction.op {
            Op::Test => Condition {
                subject: self.operand(&register(a)),
                test: Test::Truthy(c != 0),
            },
            Op::TestSet => Condition {
                subject: self.operand(&register(b)),
                test: Test::Truthy(c != 0),
            },
            op => {
                let comparison = match (op, a != 0) {
                    (Op::Eq, true) => Comparison::Eq,
                    (Op::Eq, false) => Comparison::Ne,
                    (Op::Lt, true) => Comparison::Lt,
                    (Op::Lt, false) => Comparison::Ge,
                    (Op::Le, true) => Comparison::Le,
                    _ => Comparison::Gt,
                };
                let (left, right) = (rk(b), rk(c));
                // Put the value being tested first: `3 <= x` reads `x >= 3`.
                if matches!(left, Value::Constant(_)) && !matches!(right, Value::Constant(_)) {
                    Condition {
                        subject: self.operand(&right),
                        test: Test::Compare(comparison.flipped(), self.operand(&left)),
                    }
                } else {
                    Condition {
                        subject: self.operand(&left),
                        test: Test::Compare(comparison, self.operand(&right)),
                    }
                }
            }
        }
    }

    fn operand(&self, value: &Value) -> Operand {
        match value {
            Value::Constant(constant) => Operand::Constant(constant.clone()),
            Value::Field(name) => Operand::Field(name.clone()),
            Value::Global(name) => Operand::Global(name.clone()),
            Value::Call(index) => {
                let call = &self.calls[*index];
                if call.choice.is_some() {
                    Operand::Answer(choice_id(call.pc))
                } else {
                    Operand::Call {
                        function: call.function.clone(),
                        arguments: call
                            .arguments
                            .iter()
                            .map(|argument| self.operand(argument))
                            .filter(|operand| *operand != Operand::Unknown)
                            .collect(),
                    }
                }
            }
            Value::OneOf(values) => {
                Operand::OneOf(values.iter().map(|value| self.operand(value)).collect())
            }
            Value::Unknown | Value::Method(_) | Value::Truth(..) => Operand::Unknown,
        }
    }

    /// The string key of `GETTABLE` or `SELF`: a constant, or a register
    /// holding a string constant, as the compiler writes keys whose constant
    /// index does not fit the instruction (functions with more than 256
    /// constants).
    fn key_string(&self, index: u32, registers: &Registers) -> Option<String> {
        if index >= CONSTANT_BIT {
            return self.constant_string(index);
        }
        match registers.get(&index) {
            Some(Value::Constant(Constant::String(text))) => Some(text.clone()),
            _ => None,
        }
    }

    fn constant_string(&self, index: u32) -> Option<String> {
        (index >= CONSTANT_BIT)
            .then(|| self.function.string(index - CONSTANT_BIT))
            .flatten()
            .map(str::to_owned)
    }

    /// Drops branches and loops without text, merges the same menu asked
    /// on both sides of a branch, folds the branches on a choice's answer
    /// into its options, and leaves out of each condition what the branches
    /// around it already tested.
    fn finish(&self, nodes: Vec<FlowNode>) -> Vec<FlowNode> {
        let values: HashMap<u32, &Vec<Constant>> = self
            .calls
            .iter()
            .filter_map(|call| Some((choice_id(call.pc), call.choice.as_ref()?)))
            .collect();
        let mut aliases = HashMap::new();
        let nodes = merge_choices(prune(nodes), &mut aliases);
        known(join(fold(unalias(nodes, &aliases), &values)), &[])
    }
}

/// Most instructions computing the values of one test in an `and` or `or`
/// chain.
const MAX_CONDITION_CODE: usize = 32;

/// A condition joined from the tests of an `and` or `or` chain.
struct Chain {
    condition: Guard,
    /// Where the code goes when the condition holds, and when it does not.
    taken: usize,
    fallthrough: usize,
    taken_registers: Registers,
    fallthrough_registers: Registers,
}

/// What registers hold where two paths meet, as far as tracing knows.
fn merged(left: &Registers, right: &Registers) -> Registers {
    left.iter()
        .filter_map(|(register, value)| {
            let merged = value.clone().merge(right.get(register)?.clone());
            (merged != Value::Unknown).then_some((*register, merged))
        })
        .collect()
}

/// Whether a condition tests a choice's answer.
fn tests_answer(guard: &Guard) -> bool {
    match guard {
        Guard::Test(condition) => match &condition.subject {
            Operand::Answer(_) => true,
            Operand::OneOf(values) => values
                .iter()
                .any(|value| matches!(value, Operand::Answer(_))),
            _ => false,
        },
        Guard::Any(guards) | Guard::All(guards) => guards.iter().any(tests_answer),
    }
}

fn option(label: OptionLabel) -> ChoiceOption {
    ChoiceOption {
        label,
        available: Availability::Always,
        then: Vec::new(),
    }
}

fn is_text_key(name: &str) -> bool {
    name.starts_with("TEXT_")
}

fn choice_id(pc: usize) -> u32 {
    u32::try_from(pc).unwrap_or(u32::MAX)
}

fn loop_id(pc: usize) -> u32 {
    u32::try_from(pc).unwrap_or(u32::MAX)
}

fn offset(base: usize, delta: i64) -> Option<usize> {
    usize::try_from(i64::try_from(base).ok()? + delta).ok()
}

/// The instructions each instruction can continue at, through chains of
/// unconditional jumps, so that a jump is never where paths meet. A test
/// lists where its jump goes, then what follows the jump; a loop instruction
/// lists its body, then its exit.
fn successors(code: &[Instruction]) -> Result<Vec<Vec<usize>>, Untraceable> {
    let resolve = |target: Option<usize>| -> Result<usize, Untraceable> {
        let mut pc = target.filter(|pc| *pc < code.len()).ok_or(Untraceable)?;
        for _ in 0..code.len() {
            if code[pc].op != Op::Jmp {
                return Ok(pc);
            }
            pc = offset(pc + 1, code[pc].sbx())
                .filter(|pc| *pc < code.len())
                .ok_or(Untraceable)?;
        }
        // A jump to itself never continues.
        Err(Untraceable)
    };
    // The jump that must follow a test or a generic `for`.
    let jump = |pc: usize| -> Result<Option<usize>, Untraceable> {
        match code.get(pc + 1) {
            Some(next) if next.op == Op::Jmp => Ok(offset(pc + 2, next.sbx())),
            _ => Err(Untraceable),
        }
    };
    code.iter()
        .enumerate()
        .map(|(pc, instruction)| {
            let targets = match instruction.op {
                Op::Jmp | Op::ForPrep => vec![offset(pc + 1, instruction.sbx())],
                Op::Eq | Op::Lt | Op::Le | Op::Test | Op::TestSet | Op::TForLoop => {
                    vec![jump(pc)?, Some(pc + 2)]
                }
                Op::ForLoop => vec![offset(pc + 1, instruction.sbx()), Some(pc + 1)],
                Op::Return | Op::TailCall => Vec::new(),
                Op::LoadBool if instruction.c != 0 => vec![Some(pc + 2)],
                _ => vec![Some(pc + 1)],
            };
            targets.into_iter().map(resolve).collect()
        })
        .collect()
}

/// The immediate post-dominator of each instruction: the first instruction
/// every path from it to a return passes through. `None` when the paths
/// meet only at the return, or never return.
fn post_dominators(successors: &[Vec<usize>]) -> Vec<Option<usize>> {
    let exit = successors.len();
    let forward = |pc: usize| -> &[usize] {
        if successors[pc].is_empty() {
            &[]
        } else {
            &successors[pc]
        }
    };
    let mut reverse = vec![Vec::new(); exit + 1];
    for (pc, targets) in successors.iter().enumerate() {
        if targets.is_empty() {
            reverse[exit].push(pc);
        }
        for &target in targets {
            reverse[target].push(pc);
        }
    }
    // Postorder of the reverse graph from the exit.
    let mut order = Vec::with_capacity(exit + 1);
    let mut seen = vec![false; exit + 1];
    let mut stack = vec![(exit, 0_usize)];
    seen[exit] = true;
    while let Some((node, next)) = stack.last_mut() {
        if let Some(&child) = reverse[*node].get(*next) {
            *next += 1;
            if !seen[child] {
                seen[child] = true;
                stack.push((child, 0));
            }
        } else {
            order.push(*node);
            stack.pop();
        }
    }
    let mut position = vec![usize::MAX; exit + 1];
    for (index, node) in order.iter().enumerate() {
        position[*node] = index;
    }
    let mut dominator: Vec<Option<usize>> = vec![None; exit + 1];
    dominator[exit] = Some(exit);
    let intersect = |dominator: &[Option<usize>], mut left: usize, mut right: usize| {
        while left != right {
            while position[left] < position[right] {
                left = dominator[left].unwrap_or(exit);
            }
            while position[right] < position[left] {
                right = dominator[right].unwrap_or(exit);
            }
        }
        left
    };
    let mut changed = true;
    while changed {
        changed = false;
        for &node in order.iter().rev() {
            if node == exit {
                continue;
            }
            let targets: Vec<usize> = if forward(node).is_empty() {
                vec![exit]
            } else {
                forward(node).to_vec()
            };
            let mut candidates = targets
                .into_iter()
                .filter(|target| dominator[*target].is_some());
            let Some(first) = candidates.next() else {
                continue;
            };
            let new = candidates.fold(first, |current, target| {
                intersect(&dominator, target, current)
            });
            if dominator[node] != Some(new) {
                dominator[node] = Some(new);
                changed = true;
            }
        }
    }
    dominator
        .into_iter()
        .take(exit)
        .map(|dominator| dominator.filter(|pc| *pc != exit))
        .collect()
}

/// Whether nodes show anything: text, a choice, or a marker.
fn has_content(nodes: &[FlowNode]) -> bool {
    nodes.iter().any(|node| match node {
        FlowNode::Branch {
            then, otherwise, ..
        } => has_content(then) || has_content(otherwise),
        FlowNode::Loop { body, .. } => has_content(body),
        FlowNode::Repeat { .. } => false,
        _ => true,
    })
}

/// Removes branches and loops that show nothing, such as the animations
/// chosen by the player's race.
fn prune(nodes: Vec<FlowNode>) -> Vec<FlowNode> {
    nodes
        .into_iter()
        .filter_map(|node| match node {
            FlowNode::Branch {
                condition,
                then,
                otherwise,
            } => {
                let (then, otherwise) = (prune(then), prune(otherwise));
                (has_content(&then) || has_content(&otherwise)).then_some(FlowNode::Branch {
                    condition,
                    then,
                    otherwise,
                })
            }
            FlowNode::Loop { id, body } => {
                let body = prune(body);
                has_content(&body).then_some(FlowNode::Loop { id, body })
            }
            other => Some(other),
        })
        .collect()
}

/// Merges a branch that asks the same question with the same answers on
/// both sides, only grayed out differently, into one choice whose answers
/// are available where the branch says. The later choice's id is recorded
/// in `aliases` as the one it was merged into.
fn merge_choices(nodes: Vec<FlowNode>, aliases: &mut HashMap<u32, u32>) -> Vec<FlowNode> {
    nodes
        .into_iter()
        .map(|node| match node {
            FlowNode::Branch {
                condition,
                then,
                otherwise,
            } => {
                let (then, otherwise) = (
                    merge_choices(then, aliases),
                    merge_choices(otherwise, aliases),
                );
                if let ([FlowNode::Choice(first)], [FlowNode::Choice(second)]) =
                    (then.as_slice(), otherwise.as_slice())
                    && first.kind == second.kind
                    && first.prompts == second.prompts
                    && first.options.len() == second.options.len()
                    && first
                        .options
                        .iter()
                        .zip(&second.options)
                        .all(|(left, right)| {
                            left.label == right.label
                                && left.then.is_empty()
                                && right.then.is_empty()
                        })
                {
                    aliases.insert(second.id, first.id);
                    let options = first
                        .options
                        .iter()
                        .zip(&second.options)
                        .map(|(left, right)| ChoiceOption {
                            label: left.label.clone(),
                            available: available(&condition, &left.available, &right.available),
                            then: Vec::new(),
                        })
                        .collect();
                    return FlowNode::Choice(Choice {
                        options,
                        ..first.clone()
                    });
                }
                FlowNode::Branch {
                    condition,
                    then,
                    otherwise,
                }
            }
            FlowNode::Loop { id, body } => FlowNode::Loop {
                id,
                body: merge_choices(body, aliases),
            },
            other => other,
        })
        .collect()
}

/// When an answer is available that is `then` where `condition` holds and
/// `otherwise` where it does not.
fn available(condition: &Guard, then: &Availability, otherwise: &Availability) -> Availability {
    match (then, otherwise) {
        _ if then == otherwise => then.clone(),
        (Availability::Always, Availability::Never) => Availability::When(condition.clone()),
        (Availability::Never, Availability::Always) => {
            Availability::When(condition.clone().negated())
        }
        _ => Availability::Unknown,
    }
}

/// Makes conditions on the answer of a merged choice refer to the choice it
/// was merged into, including a value that is the answer of either.
fn unalias(nodes: Vec<FlowNode>, aliases: &HashMap<u32, u32>) -> Vec<FlowNode> {
    if aliases.is_empty() {
        return nodes;
    }
    nodes
        .into_iter()
        .map(|node| match node {
            FlowNode::Branch {
                condition,
                then,
                otherwise,
            } => FlowNode::Branch {
                condition: unalias_guard(condition, aliases),
                then: unalias(then, aliases),
                otherwise: unalias(otherwise, aliases),
            },
            FlowNode::Loop { id, body } => FlowNode::Loop {
                id,
                body: unalias(body, aliases),
            },
            FlowNode::Choice(mut choice) => {
                for option in &mut choice.options {
                    option.then = unalias(std::mem::take(&mut option.then), aliases);
                }
                FlowNode::Choice(choice)
            }
            other => other,
        })
        .collect()
}

fn unalias_guard(guard: Guard, aliases: &HashMap<u32, u32>) -> Guard {
    let answer = |operand: &Operand| match operand {
        Operand::Answer(id) => Some(*aliases.get(id).unwrap_or(id)),
        _ => None,
    };
    match guard {
        Guard::Test(mut condition) => {
            let merged = match &condition.subject {
                Operand::Answer(_) => answer(&condition.subject),
                Operand::OneOf(values) => {
                    let ids: Option<Vec<u32>> = values.iter().map(answer).collect();
                    ids.filter(|ids| ids.windows(2).all(|pair| pair[0] == pair[1]))
                        .and_then(|ids| ids.first().copied())
                }
                _ => None,
            };
            if let Some(id) = merged {
                condition.subject = Operand::Answer(id);
            }
            Guard::Test(condition)
        }
        Guard::Any(guards) => Guard::Any(
            guards
                .into_iter()
                .map(|guard| unalias_guard(guard, aliases))
                .collect(),
        ),
        Guard::All(guards) => Guard::All(
            guards
                .into_iter()
                .map(|guard| unalias_guard(guard, aliases))
                .collect(),
        ),
    }
}

/// Moves the branches on a choice's answer that follow the choice into its
/// options. Scripts often test the answer in consecutive `if`s that each
/// end by asking again; a later branch applies only to the options that did
/// not.
fn fold(nodes: Vec<FlowNode>, values: &HashMap<u32, &Vec<Constant>>) -> Vec<FlowNode> {
    let mut folded: Vec<FlowNode> = Vec::with_capacity(nodes.len());
    for node in nodes {
        let node = match node {
            FlowNode::Branch {
                condition,
                then,
                otherwise,
            } => FlowNode::Branch {
                condition,
                then: fold(then, values),
                otherwise: fold(otherwise, values),
            },
            FlowNode::Loop { id, body } => FlowNode::Loop {
                id,
                body: fold(body, values),
            },
            other => other,
        };
        if let (
            Some(FlowNode::Choice(choice)),
            FlowNode::Branch {
                condition: Guard::Test(condition),
                ..
            },
        ) = (folded.last_mut(), &node)
            && condition.subject == Operand::Answer(choice.id)
            && let Some(choice_values) = values.get(&choice.id)
            && choice_values.len() == choice.options.len()
        {
            let open: Vec<usize> = (0..choice.options.len())
                .filter(|index| !ends_by_repeating(&choice.options[*index].then))
                .collect();
            if !open.is_empty()
                && let Some(bodies) =
                    split(choice.id, &open, std::slice::from_ref(&node), choice_values)
            {
                for (index, body) in bodies {
                    choice.options[index].then.extend(body);
                }
                continue;
            }
        }
        folded.push(node);
    }
    folded
}

/// Leaves out the tests of a branch's condition that hold wherever it is
/// reached, as the branches around it tested them: inside `if a then`, `if
/// a and b` is `if b`. A condition made only of such tests is kept whole.
/// A loop starts over, as what it tests may change between its rounds.
fn known(nodes: Vec<FlowNode>, holding: &[Guard]) -> Vec<FlowNode> {
    nodes
        .into_iter()
        .map(|node| match node {
            FlowNode::Branch {
                condition,
                then,
                otherwise,
            } => {
                let tests = conjuncts(condition.clone());
                let rest: Vec<Guard> = tests
                    .iter()
                    .filter(|test| !holding.contains(test))
                    .cloned()
                    .collect();
                let condition = if rest.is_empty() || rest.len() == tests.len() {
                    condition
                } else {
                    group(rest, Guard::All)
                };
                let with = |guard: Guard| {
                    let mut inner = holding.to_vec();
                    inner.extend(conjuncts(guard));
                    inner
                };
                FlowNode::Branch {
                    then: known(then, &with(condition.clone())),
                    otherwise: known(otherwise, &with(condition.clone().negated())),
                    condition,
                }
            }
            FlowNode::Loop { id, body } => FlowNode::Loop {
                id,
                body: known(body, &[]),
            },
            FlowNode::Choice(mut choice) => {
                for option in &mut choice.options {
                    option.then = known(std::mem::take(&mut option.then), holding);
                }
                FlowNode::Choice(choice)
            }
            other => other,
        })
        .collect()
}

/// Joins the branches the compiler makes of `or` and `and` back into one
/// branch. `if a or b then X else Y` compiles to a branch on `a` whose
/// `then` is `X` and whose `otherwise` is only a branch on `b` with the same
/// `X` on one side; `and` is the same with the sides swapped. Either side of
/// the inner branch may match, in which case its test is inverted.
fn join(nodes: Vec<FlowNode>) -> Vec<FlowNode> {
    nodes
        .into_iter()
        .map(|node| match node {
            FlowNode::Branch {
                condition,
                then,
                otherwise,
            } => join_branch(condition, join(then), join(otherwise)),
            FlowNode::Loop { id, body } => FlowNode::Loop {
                id,
                body: join(body),
            },
            FlowNode::Choice(mut choice) => {
                for option in &mut choice.options {
                    option.then = join(std::mem::take(&mut option.then));
                }
                FlowNode::Choice(choice)
            }
            other => other,
        })
        .collect()
}

fn join_branch(condition: Guard, then: Vec<FlowNode>, otherwise: Vec<FlowNode>) -> FlowNode {
    if let [
        FlowNode::Branch {
            condition: inner,
            then: inner_then,
            otherwise: inner_otherwise,
        },
    ] = otherwise.as_slice()
    {
        if *inner_then == then {
            let (inner, rest) = (inner.clone(), inner_otherwise.clone());
            return oriented(any(condition, inner), then, rest);
        }
        if *inner_otherwise == then {
            let (inner, rest) = (inner.clone().negated(), inner_then.clone());
            return oriented(any(condition, inner), then, rest);
        }
    }
    if let [
        FlowNode::Branch {
            condition: inner,
            then: inner_then,
            otherwise: inner_otherwise,
        },
    ] = then.as_slice()
    {
        if *inner_otherwise == otherwise {
            let (inner, body) = (inner.clone(), inner_then.clone());
            return oriented(all(condition, inner), body, otherwise);
        }
        if *inner_then == otherwise {
            let (inner, body) = (inner.clone().negated(), inner_otherwise.clone());
            return oriented(all(condition, inner), body, otherwise);
        }
    }
    oriented(condition, then, otherwise)
}

/// The branch read in whichever direction has fewer negated tests: `if a
/// and b then X else Y` rather than `if not a or not b then Y else X`.
fn oriented(condition: Guard, then: Vec<FlowNode>, otherwise: Vec<FlowNode>) -> FlowNode {
    let flipped = condition.clone().negated();
    if negations(&flipped) < negations(&condition) {
        branch(flipped, otherwise, then)
    } else {
        branch(condition, then, otherwise)
    }
}

fn negations(guard: &Guard) -> usize {
    match guard {
        Guard::Test(condition) => usize::from(matches!(
            condition.test,
            Test::Truthy(false)
                | Test::Compare(Comparison::Ne, _)
                | Test::Compare(Comparison::Eq, Operand::Constant(Constant::Boolean(false)))
        )),
        Guard::Any(guards) | Guard::All(guards) => guards.iter().map(negations).sum(),
    }
}

fn branch(condition: Guard, then: Vec<FlowNode>, otherwise: Vec<FlowNode>) -> FlowNode {
    FlowNode::Branch {
        condition,
        then,
        otherwise,
    }
}

fn any(left: Guard, right: Guard) -> Guard {
    let mut guards = Vec::new();
    for guard in [left, right] {
        match guard {
            Guard::Any(inner) => guards.extend(inner),
            other => guards.push(other),
        }
    }
    Guard::Any(guards)
}

fn all(left: Guard, right: Guard) -> Guard {
    let mut guards = Vec::new();
    for guard in [left, right] {
        match guard {
            Guard::All(inner) => guards.extend(inner),
            other => guards.push(other),
        }
    }
    Guard::All(guards)
}

/// Whether nodes end by going back to the start of a loop.
fn ends_by_repeating(nodes: &[FlowNode]) -> bool {
    matches!(nodes.last(), Some(FlowNode::Repeat { .. }))
}

/// The nodes each option in `options` leads to, following branches on the
/// answer at the start of `nodes`; `None` when a test cannot be evaluated.
fn split(
    choice: u32,
    options: &[usize],
    nodes: &[FlowNode],
    values: &[Constant],
) -> Option<Vec<(usize, Vec<FlowNode>)>> {
    let Some(FlowNode::Branch {
        condition: Guard::Test(condition),
        then,
        otherwise,
    }) = nodes.first()
    else {
        return Some(
            options
                .iter()
                .map(|option| (*option, nodes.to_vec()))
                .collect(),
        );
    };
    if condition.subject != Operand::Answer(choice) {
        return Some(
            options
                .iter()
                .map(|option| (*option, nodes.to_vec()))
                .collect(),
        );
    }
    let mut holding = Vec::new();
    let mut failing = Vec::new();
    for &option in options {
        if condition.test.holds(&values[option])? {
            holding.push(option);
        } else {
            failing.push(option);
        }
    }
    let rest = &nodes[1..];
    let mut bodies = split(choice, &holding, then, values)?;
    bodies.extend(split(choice, &failing, otherwise, values)?);
    for (_, body) in &mut bodies {
        body.extend_from_slice(rest);
    }
    Some(bodies)
}

impl Comparison {
    const fn flipped(self) -> Self {
        match self {
            Self::Eq => Self::Eq,
            Self::Ne => Self::Ne,
            Self::Lt => Self::Gt,
            Self::Le => Self::Ge,
            Self::Gt => Self::Lt,
            Self::Ge => Self::Le,
        }
    }
}

impl Test {
    /// Whether the test holds for a value; `None` when it cannot be decided.
    fn holds(&self, value: &Constant) -> Option<bool> {
        match self {
            Self::Truthy(expected) => {
                Some(matches!(value, Constant::Nil | Constant::Boolean(false)) != *expected)
            }
            Self::Compare(comparison, Operand::Constant(other)) => match (value, other) {
                (Constant::Boolean(left), Constant::Boolean(right)) => match comparison {
                    Comparison::Eq => Some(left == right),
                    Comparison::Ne => Some(left != right),
                    _ => None,
                },
                (Constant::Number(left), Constant::Number(right)) => Some(match comparison {
                    Comparison::Eq => (left - right).abs() < f64::EPSILON,
                    Comparison::Ne => (left - right).abs() >= f64::EPSILON,
                    Comparison::Lt => left < right,
                    Comparison::Le => left <= right,
                    Comparison::Gt => left > right,
                    Comparison::Ge => left >= right,
                }),
                (Constant::Boolean(_), Constant::Number(_) | Constant::Nil)
                | (Constant::Number(_), Constant::Boolean(_) | Constant::Nil) => match comparison {
                    Comparison::Eq => Some(false),
                    Comparison::Ne => Some(true),
                    _ => None,
                },
                _ => None,
            },
            Self::Compare(..) => None,
        }
    }
}
