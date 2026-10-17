// SPDX-License-Identifier: Apache-2.0

//! The data model of the IR, as `serde` reads and writes it. Every item here has a JSON form;
//! `docs/dsl.md` §4 describes them together.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// One decoder with everything it needs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Module {
    /// The IR version, [`crate::IR_VERSION`].
    pub ir: u32,
    /// The module's name: the decoder's, as the registry names it.
    pub name: String,
    /// The types declared here: records, enums and unions.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub types: Vec<TypeDef>,
    /// The contracts declared here: the schema of what the decoder emits.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub contracts: Vec<Contract>,
    /// The contracts read, copied from the modules that declare them with the types they need.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub imports: Vec<Import>,
    /// Named constants.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub consts: Vec<Const>,
    /// Free functions: pure, synchronous, without access to the decoder's state.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub functions: Vec<Function>,
    /// The decoder; none for a module that only declares contracts, for decoders written by
    /// hand.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decoder: Option<Decoder>,
}

/// A contract another module declares, as this one reads it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Import {
    /// The module that declares it.
    pub module: String,
    /// The types the contract's events are made of.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub types: Vec<TypeDef>,
    /// The contract.
    pub contract: Contract,
}

/// A named type.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TypeDef {
    pub name: String,
    #[serde(flatten)]
    pub kind: TypeKind,
}

/// What a named type is.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum TypeKind {
    /// A struct: fields by name.
    Record { fields: Vec<Field> },
    /// A set of names, written as text in events and configurations.
    Enum { variants: Vec<EnumVariant> },
    /// A tagged union: variants with fields, tagged by `kind`.
    Union { variants: Vec<UnionVariant> },
}

/// A variant of an enum: its name as events and configurations spell it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EnumVariant(pub String);

/// A variant of a union.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnionVariant {
    /// The `kind` tag.
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<Field>,
}

/// A field of a record or a union variant.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Field {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: Type,
    /// Whether the field is a channel of the source (a configuration field the interface
    /// offers as one).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub channel: bool,
}

/// A type: a primitive or a declared type by name, an option, a bounded array or a batch.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Type {
    /// A primitive (`bool`, `u8`, `u16`, `u32`, `u64`, `text`, `mask`, `levels`, `range`,
    /// `provenance`, `cursor`) or a declared type.
    Name(String),
    /// A value that may be missing.
    Option { option: Box<Type> },
    /// At most `bound` items.
    Array { array: Box<Type>, bound: u32 },
    /// A batch of at most `bound` items with the provenance of what it holds.
    Batch { batch: Box<Type>, bound: u32 },
}

impl Type {
    /// The primitive type names.
    pub const PRIMITIVES: [&'static str; 11] = [
        "bool",
        "u8",
        "u16",
        "u32",
        "u64",
        "text",
        "mask",
        "levels",
        "range",
        "provenance",
        "cursor",
    ];

    /// The type named `name`.
    pub fn named(name: &str) -> Self {
        Type::Name(name.to_owned())
    }

    /// An optional `inner`.
    pub fn option(inner: Type) -> Self {
        Type::Option {
            option: Box::new(inner),
        }
    }

    /// Whether this is the primitive `name`.
    pub fn is(&self, name: &str) -> bool {
        matches!(self, Type::Name(have) if have == name)
    }

    /// Whether the name is a primitive type's.
    pub fn is_primitive_name(name: &str) -> bool {
        Self::PRIMITIVES.contains(&name)
    }
}

/// A contract: a schema and the union type of its events, framed or not.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Contract {
    pub schema: SchemaRef,
    /// The union type of the events.
    pub event: String,
    /// How the stream frames its traffic, if it does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub framing: Option<Framing>,
}

/// The name and version of a schema.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SchemaRef {
    pub name: String,
    pub version: u32,
}

/// How a framed contract frames its traffic: the event kinds that begin a frame, carry its
/// items and end it, the field naming the frame on all of them, the `begin` field saying the
/// frame was under way, the `end` field saying why it ended and its values for a normal end.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Framing {
    pub begin: String,
    /// The kinds of event that carry a frame's items: one, and the cursor hands out its
    /// fields; several, and it hands out their union, tagged by kind.
    pub items: Vec<String>,
    pub end: String,
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub partial: Option<String>,
    pub reason: String,
    /// The values of `reason` for a normal end.
    pub normal: Vec<String>,
    /// The id of the one frame of a bus without a framing line, which never begins or ends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unframed: Option<u64>,
}

/// A named constant.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Const {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: Type,
    pub value: Literal,
}

/// A literal value.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Literal {
    Bool(bool),
    Int(u64),
    Text(String),
}

/// A function or a method: a method has the decoder's state as `self`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Function {
    pub name: String,
    /// Whether the body awaits: only methods may.
    #[serde(default, rename = "async", skip_serializing_if = "std::ops::Not::not")]
    pub is_async: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<Param>,
    /// What it returns; nothing when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ret: Option<Type>,
    pub body: Vec<Stmt>,
}

/// A parameter. Records are passed by reference, everything else by value.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Param {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: Type,
    /// Whether the function writes through the parameter (a record it changes, or passes on
    /// to a function that does): a mutable reference in Rust.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub mutable: bool,
}

/// The decoder: its ports, configuration, state, methods and hooks.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Decoder {
    pub name: String,
    pub category: Category,
    pub config: Config,
    pub input: Input,
    /// What the input requires of the output it reads, evaluated with `config` in scope.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires: Vec<RequireSpec>,
    pub output: Output,
    /// What the output declares, evaluated with `config` in scope.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub props: Vec<PropSpec>,
    /// What a bus decoder does with a frame whose beginning was not seen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub partial: Option<Partial>,
    /// How a bus decoder's cut-off body is closed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery: Option<Recovery>,
    /// The fields of the decoder's state, initialized in order with `config` and `rate` in
    /// scope.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub state: Vec<StateField>,
    /// The methods, among them the hooks by name: `run` (logic and stream decoders),
    /// `transaction` and `recover` (bus decoders), and the optional `hold`, `onGap`, `onEnd`
    /// and `onProgress`.
    pub methods: Vec<Function>,
}

/// Where the catalog lists a decoder.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Category {
    Bus,
    Device,
    Line,
}

/// The decoder's configuration.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// The record type.
    #[serde(rename = "type")]
    pub ty: String,
    /// The defaults, as JSON.
    pub defaults: serde_json::Value,
    /// The free function that checks a configuration: `(config) -> option<text>`, the message
    /// of the error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validate: Option<String>,
}

/// The decoder's input port: what it reads.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Input {
    /// Logic samples, through the signal cursor.
    Logic,
    /// The events of an unframed lower stream.
    Stream { schema: SchemaRef },
    /// The frames of a framed lower stream, through its cursor.
    Bus { schema: SchemaRef },
}

/// A requirement on a property of the output the input reads.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequireSpec {
    pub key: String,
    pub requirement: RequireKind,
    /// One value for `equals`, `at-least` and `at-most`, two for `within`, any number for
    /// `one-of`.
    pub values: Vec<Expr>,
}

/// The kinds of requirement.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RequireKind {
    Equals,
    OneOf,
    AtLeast,
    AtMost,
    Within,
}

/// The decoder's output port.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Output {
    pub schema: SchemaRef,
}

/// A property the output declares.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PropSpec {
    pub key: String,
    pub value: Expr,
}

/// What a bus decoder does with a frame whose beginning was not seen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Partial {
    /// Drain it without running the body.
    Drain,
    /// Run the body on it.
    Read,
}

/// How a cut-off body is closed: the reason `recover` gets for each cut, as variants of the
/// reason enum.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recovery {
    /// The enum type of the reasons.
    pub reason: String,
    pub incomplete: String,
    pub truncated: String,
    pub missing: String,
    pub gap: String,
    pub end: String,
}

/// A field of the decoder's state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateField {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: Type,
    pub init: Expr,
    /// Whether the field never changes after initialization.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub readonly: bool,
}

/// A statement.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "stmt", rename_all = "kebab-case")]
pub enum Stmt {
    /// A local bound to a value; a copy, for records.
    Let {
        name: String,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        mutable: bool,
        #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
        ty: Option<Type>,
        value: Expr,
    },
    /// A local that borrows a field of `self` (a record, or an option of one already known to
    /// be present): while it is in scope, no method of `self` may be called.
    LetRef {
        name: String,
        place: Expr,
    },
    /// An assignment, compound with `op`.
    Assign {
        target: Expr,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        op: Option<BinaryOp>,
        value: Expr,
    },
    If {
        cond: Expr,
        then: Vec<Stmt>,
        #[serde(default, rename = "else", skip_serializing_if = "Vec::is_empty")]
        otherwise: Vec<Stmt>,
    },
    /// `if (value !== undefined)`, with the value bound to `bind` in `then`. When the value is
    /// a record field of `self` that `then` writes through, `borrow` says the binding borrows
    /// it: no method of `self` may be called in `then`.
    IfSome {
        value: Expr,
        bind: String,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        borrow: bool,
        then: Vec<Stmt>,
        #[serde(default, rename = "else", skip_serializing_if = "Vec::is_empty")]
        otherwise: Vec<Stmt>,
    },
    /// Binds `bind` to the value, or runs `else`, which leaves the block (return, break,
    /// continue). `borrow` as for [`Stmt::IfSome`], for the rest of the block.
    LetSome {
        bind: String,
        value: Expr,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        borrow: bool,
        #[serde(rename = "else")]
        otherwise: Vec<Stmt>,
    },
    Match {
        value: Expr,
        arms: Vec<Arm>,
    },
    Loop {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        body: Vec<Stmt>,
    },
    While {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        cond: Expr,
        body: Vec<Stmt>,
    },
    /// `var` over the half-open range `from..to`.
    For {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        var: String,
        from: Expr,
        to: Expr,
        body: Vec<Stmt>,
    },
    /// `var` over the items of a bounded array.
    ForEach {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        var: String,
        array: Expr,
        body: Vec<Stmt>,
    },
    Break {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    Continue {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    Return {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<Expr>,
    },
    /// An expression for its effect: an awaited operation, a call.
    Expr {
        expr: Expr,
    },
}

/// An arm of a match.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Arm {
    pub pattern: Pattern,
    pub body: Vec<Stmt>,
}

/// What an arm matches.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "pattern", rename_all = "kebab-case")]
pub enum Pattern {
    /// One variant of a union, its fields bound to locals.
    Variant {
        name: String,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        binds: BTreeMap<String, String>,
    },
    /// Any of these values: enum variant names, integers or booleans.
    Values { values: Vec<Literal> },
    /// Anything else.
    Default,
}

/// An expression.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "expr", rename_all = "kebab-case")]
pub enum Expr {
    /// An integer or a boolean.
    Lit {
        value: Literal,
    },
    /// Text, only in messages.
    Text {
        value: String,
    },
    /// Text with values in it, only in messages.
    Format {
        parts: Vec<Expr>,
    },
    /// The missing value of an option.
    None,
    /// A local or a parameter.
    Local {
        name: String,
    },
    /// A field of the decoder's state: the declared fields, `config` and `rate`.
    #[serde(rename = "self")]
    SelfField {
        field: String,
    },
    /// A field of a record or of a variant whose kind is known.
    Field {
        of: Box<Expr>,
        name: String,
    },
    /// An item of an array.
    Index {
        of: Box<Expr>,
        at: Box<Expr>,
    },
    /// The `kind` of a union value, compared with variant names.
    Tag {
        value: Box<Expr>,
    },
    Unary {
        op: UnaryOp,
        value: Box<Expr>,
    },
    Binary {
        op: BinaryOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    /// Truncation to a narrower width.
    Cast {
        value: Box<Expr>,
        to: Type,
    },
    /// A record value.
    Record {
        #[serde(rename = "type")]
        ty: String,
        fields: BTreeMap<String, Expr>,
    },
    /// A union value.
    Variant {
        #[serde(rename = "type")]
        ty: String,
        kind: String,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        fields: BTreeMap<String, Expr>,
    },
    /// An enum value.
    Enum {
        #[serde(rename = "type")]
        ty: String,
        value: String,
    },
    /// An array of these items, bounded by their number.
    Array {
        items: Vec<Expr>,
    },
    /// A free function.
    Call {
        name: String,
        args: Vec<Expr>,
    },
    /// A synchronous method of `self`.
    Method {
        name: String,
        args: Vec<Expr>,
    },
    /// A runtime helper: `min`, `max`, `saturating-sub`, `count-ones`, `from-bool`, `mul-div`,
    /// `mul-div-ceil`, `at`, `one`, `over`, `join`, `mask`, `mask-has`, `mask-with`,
    /// `levels-high`, `max-channels`, `position`, `seek`, `batch`, `batch-length`,
    /// `batch-is-empty`, `batch-is-full`, `batch-at`, `batch-push`, `batch-take`,
    /// `batch-clear`, `consumed`, `tx-id`, `tx-partial`, `unwrap-or`.
    Intrinsic {
        name: String,
        args: Vec<Expr>,
    },
    /// `cond ? then : else`.
    Ternary {
        cond: Box<Expr>,
        then: Box<Expr>,
        #[serde(rename = "else")]
        otherwise: Box<Expr>,
    },
    /// `some` with the value bound to `bind` when it is present, `none` otherwise.
    IfSome {
        value: Box<Expr>,
        bind: String,
        some: Box<Expr>,
        none: Box<Expr>,
    },
    /// `then` with the variant's fields bound when the value is that variant, `else`
    /// otherwise.
    IfVariant {
        value: Box<Expr>,
        variant: String,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        binds: BTreeMap<String, String>,
        then: Box<Expr>,
        #[serde(rename = "else")]
        otherwise: Box<Expr>,
    },
    /// The value of an option a bus decoder's body requires (`!`): a missing one cuts the
    /// body, which the recovery closes as `missing`.
    Required {
        value: Box<Expr>,
    },
    /// A runtime operation, awaited.
    Await {
        op: Op,
    },
}

/// A unary operator.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UnaryOp {
    /// Logical not.
    Not,
    /// Bitwise not, at the operand's width.
    BitNot,
}

/// A binary operator. Arithmetic wraps at the width of the wider operand; a shift keeps the
/// left operand's width; division by zero yields zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    /// Short-circuit and.
    And,
    /// Short-circuit or.
    Or,
}

/// A runtime operation: the only things a body waits for.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case")]
pub enum Op {
    /// The next edge on the watched channels (logic).
    NextChange { mask: Box<Expr> },
    /// The next edge before a deadline, or the deadline (logic).
    NextChangeBefore { mask: Box<Expr>, until: Box<Expr> },
    /// The levels at the cursor (logic).
    Sample,
    /// The next event of the lower stream (stream).
    Next,
    /// The next item of the frame (bus).
    Item,
    /// The next item of the frame or its normal end (bus).
    ItemOrEnd,
    /// The rest of the frame (bus).
    Drain,
    /// An event; `at` may be left out in a bus decoder, where it is what was consumed.
    Emit {
        event: Box<Expr>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        at: Option<Box<Expr>>,
    },
    /// An asynchronous method of `self`.
    Method { name: String, args: Vec<Expr> },
}
