// SPDX-License-Identifier: Apache-2.0

//! A module as the interpreter runs it: the instructions of its functions and methods, its
//! types, its ports and configuration, and what the main loop needs to call its hooks.

use std::fmt;
use std::sync::Arc;

use serde_json::Value as Json;
use uiharu_core::SchemaId;
use uiharu_core::dsl::Partial;
use uiharu_ir::{Category, Diagnostic, Module, RequireKind};

use crate::types::{Ty, TypeTable};
use crate::value::{ArithOp, Width};

/// Why a module cannot be run.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LoadError {
    /// The module breaks the engine's rules; see [`uiharu_ir::validate`].
    #[error("the module breaks the engine's rules: {}", diagnostics(.0))]
    Invalid(Vec<Diagnostic>),
    /// A construct the interpreter does not run, or one the Rust backend would not emit
    /// either, with the path of the item it was found in.
    #[error("{path}: {message}")]
    Unsupported { path: String, message: String },
}

fn diagnostics(list: &[Diagnostic]) -> String {
    list.iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

/// A function or a method, by its index in [`Program::functions`].
pub type FuncId = u32;

/// The kind of decoder, by its input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Logic samples, through the signal cursor.
    Logic,
    /// The events of an unframed lower stream.
    Stream,
    /// The frames of a framed lower stream, through its cursor.
    Bus,
}

/// A constant an instruction pushes.
#[derive(Clone, Debug, PartialEq)]
pub enum Const {
    Unit,
    None,
    Bool(bool),
    Int(u64),
    Enum(u16),
    /// A unit variant of an externally tagged enum.
    UnitVariant(u16),
    Text(Arc<str>),
}

/// Where a place starts: a local of the frame or a field of the decoder's state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Base {
    Local(u32),
    Field(u32),
}

/// A step into a place: a field by index, an item at a constant index, or an item at an
/// index taken from the stack.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Field(u32),
    At(u32),
    Index,
}

/// A place a value is read from or written to: a base and the steps into it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    pub base: Base,
    pub steps: Vec<Step>,
}

impl Place {
    pub fn local(slot: u32) -> Self {
        Self {
            base: Base::Local(slot),
            steps: Vec::new(),
        }
    }

    pub fn field(index: u32) -> Self {
        Self {
            base: Base::Field(index),
            steps: Vec::new(),
        }
    }

    /// How many indices the place takes from the stack.
    pub fn dynamic_indices(&self) -> usize {
        self.steps
            .iter()
            .filter(|step| matches!(step, Step::Index))
            .count()
    }
}

/// A comparison.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

/// A runtime helper, as an instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Intrinsic {
    Min,
    Max,
    SaturatingSub,
    CountOnes,
    FromBool,
    MulDiv,
    MulDivCeil,
    At,
    One,
    Over,
    Join,
    /// A mask of this many optional channels.
    Mask(u32),
    MaskHas,
    MaskWith,
    LevelsHigh,
    Position,
    Seek,
    Consumed,
    TxId,
    TxPartial,
    UnwrapOr,
}

/// A question asked of a batch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BatchQuery {
    Length,
    Capacity,
    IsEmpty,
    IsFull,
    At,
}

/// A runtime operation the body waits for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    NextChange,
    NextChangeBefore,
    Sample,
    Next,
    Item,
    ItemOrEnd,
    Drain,
    /// An event; `at` says whether its provenance is on the stack, under it.
    Emit {
        at: bool,
    },
}

/// One instruction of a function. Expressions leave exactly one value on the stack;
/// statements leave none.
#[derive(Clone, Debug, PartialEq)]
pub enum Instr {
    Const(Const),
    LoadLocal(u32),
    StoreLocal(u32),
    /// Pushes the value at a place; the place's dynamic indices are popped first.
    LoadPath(Place),
    /// Pops a value into a place; the place's dynamic indices are under it.
    StorePath(Place),
    /// Moves the value out of a place, leaving nothing there.
    TakePath(Place),
    Pop,
    Dup,
    Swap,
    /// Pops a record or union value and pushes its field.
    GetField(u32),
    /// Pops an index and an array and pushes the item.
    Index,
    MakeRecord(u32),
    MakeVariant {
        tag: u16,
        fields: u32,
    },
    MakeArray(u32),
    Binary {
        op: ArithOp,
        width: Width,
    },
    Compare(CmpOp),
    Not,
    BitNot(Width),
    /// Truncates an integer to a width.
    Narrow(Width),
    /// Pops an enum value and pushes its name as text.
    EnumName(u32),
    Jump(u32),
    JumpIfFalse(u32),
    JumpIfTrue(u32),
    /// Pops a value and pushes whether it is the missing value of an option.
    IsNone,
    /// Pops a union value and pushes whether it is this variant.
    IsVariant(u16),
    /// Pops a value and pushes whether it equals one of these.
    IsIn(Vec<Const>),
    /// Pops a record or union value and stores the fields named into locals.
    Destructure(Vec<(u32, u32)>),
    /// The next item of the array in `array`, counted by `index`, into `var`; jumps to `end`
    /// when there is none.
    ForEachNext {
        array: u32,
        index: u32,
        var: u32,
        end: u32,
    },
    /// Calls a function: its parameters are popped, its result is pushed, then the final
    /// values of its mutable record parameters, in order.
    Call(FuncId),
    Ret,
    /// Pops an option; a missing value cuts the body.
    Required,
    /// Pops this many values and pushes their text.
    Format(u32),
    Intrinsic(Intrinsic),
    NewBatch(u32),
    /// Pops the provenance, the item and the batch; pushes the batch.
    BatchPush,
    /// Pops the batch; pushes the batch, then what it held.
    BatchTake,
    /// Pops the batch; pushes it emptied.
    BatchClear,
    /// Pops the batch; pushes the answer.
    BatchQuery(BatchQuery),
    Await(Op),
}

/// A function or method, compiled.
#[derive(Clone, Debug, PartialEq)]
pub struct Function {
    pub name: String,
    pub code: Vec<Instr>,
    /// The parameters, in the first slots.
    pub params: u32,
    /// The slots a frame needs.
    pub locals: u32,
    /// The slots of the mutable record parameters, pushed after the result on return.
    pub outs: Vec<u32>,
    pub is_async: bool,
}

/// The hooks of a decoder, by function.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Hooks {
    pub run: Option<FuncId>,
    pub transaction: Option<FuncId>,
    pub recover: Option<FuncId>,
    pub hold: Option<FuncId>,
    pub on_gap: Option<FuncId>,
    pub on_end: Option<FuncId>,
    pub on_progress: Option<FuncId>,
}

/// A field of the decoder's state and the function computing its initial value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StateField {
    pub name: String,
    pub ty: Ty,
    pub init: FuncId,
}

/// A property the output declares: its key, the function computing its value from the
/// configuration, and the value's type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prop {
    pub key: String,
    pub value: FuncId,
    pub ty: Ty,
}

/// A requirement on the lower stream, its values computed from the configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Require {
    pub key: String,
    pub kind: RequireKind,
    pub values: Vec<(FuncId, Ty)>,
}

/// How the lower stream of a bus decoder frames its traffic, as data: the item type the
/// cursor hands out is the record `item_record` or the union `item_union` of the types.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FramingData {
    pub begin: String,
    pub items: Vec<String>,
    pub end: String,
    pub id: String,
    pub partial: Option<String>,
    pub reason: String,
    pub normal: Vec<String>,
    pub unframed: Option<u64>,
    /// The item type, as `<Event>Item`.
    pub item: Ty,
}

/// The input port of the decoder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Input {
    Logic,
    Stream {
        schema: SchemaId,
        /// The lower event union.
        event: String,
    },
    Bus {
        schema: SchemaId,
        event: String,
        framing: Box<FramingData>,
    },
}

/// The reasons `recover` gets for each cut, as variants of the reason enum.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Recovery {
    pub incomplete: u16,
    pub truncated: u16,
    pub missing: u16,
    pub gap: u16,
    pub end: u16,
}

/// A module, loaded: everything the interpreter needs to make and run its decoder.
pub struct Program {
    pub(crate) name: String,
    pub(crate) category: Category,
    pub(crate) kind: Kind,
    pub(crate) types: TypeTable,
    pub(crate) functions: Vec<Function>,
    pub(crate) hooks: Hooks,
    /// The configuration record, its defaults and its check.
    pub(crate) config_ty: String,
    pub(crate) defaults: Json,
    pub(crate) validate: Option<FuncId>,
    pub(crate) state: Vec<StateField>,
    /// The properties the output declares.
    pub(crate) props: Vec<Prop>,
    pub(crate) requires: Vec<Require>,
    pub(crate) input: Input,
    pub(crate) output: SchemaId,
    /// The event union of the output.
    pub(crate) event: String,
    pub(crate) partial: Partial,
    pub(crate) recovery: Option<Recovery>,
}

impl fmt::Debug for Program {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Program")
            .field("name", &self.name)
            .field("kind", &self.kind)
            .field("functions", &self.functions.len())
            .finish_non_exhaustive()
    }
}

impl Program {
    /// Loads `module`: checks it against the engine's rules and compiles it.
    pub fn load(module: &Module) -> Result<Self, LoadError> {
        let diagnostics = uiharu_ir::validate(module);
        if !diagnostics.is_empty() {
            return Err(LoadError::Invalid(diagnostics));
        }
        crate::compile::load(module)
    }

    /// The module's name: the decoder's, as the registry names it.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Where the catalog lists the decoder.
    pub fn category(&self) -> Category {
        self.category
    }

    /// What the decoder reads.
    pub fn kind(&self) -> Kind {
        self.kind
    }

    /// The schema of what the decoder reads; `None` for logic samples.
    pub fn input_schema(&self) -> Option<SchemaId> {
        match &self.input {
            Input::Logic => None,
            Input::Stream { schema, .. } | Input::Bus { schema, .. } => Some(*schema),
        }
    }

    /// The schema of the events the decoder emits.
    pub fn output_schema(&self) -> SchemaId {
        self.output
    }

    /// The `kind` tags of the events, in the order the contract declares them.
    pub fn event_kinds(&self) -> Vec<String> {
        self.types
            .union(&self.event)
            .map(|(_, def)| def.variants.iter().map(|v| v.name.clone()).collect())
            .unwrap_or_default()
    }

    /// The default configuration, as JSON.
    pub fn defaults(&self) -> &Json {
        &self.defaults
    }

    /// The declared types.
    pub fn types(&self) -> &TypeTable {
        &self.types
    }

    /// The name of the configuration record.
    pub fn config_type(&self) -> &str {
        &self.config_ty
    }

    pub(crate) fn function(&self, id: FuncId) -> &Function {
        &self.functions[id as usize]
    }
}
