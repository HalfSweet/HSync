// SPDX-License-Identifier: Apache-2.0

//! The values a decoder computes with, as the interpreter holds them. Integers of every width
//! share one representation and are kept below their width; an option is its value or
//! [`Value::None`], without a wrapper; records, union values and arrays are shared until one is
//! written, which copies it — the value semantics of the DSL, where every record has one owner.

use std::fmt;
use std::rc::Rc;
use std::sync::Arc;

use uiharu_core::{ChannelMask, Levels, Provenance, TickRange};

/// The width of an integer type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Width {
    W8,
    W16,
    W32,
    W64,
}

impl Width {
    /// The bits of the type.
    pub const fn bits(self) -> u32 {
        match self {
            Width::W8 => 8,
            Width::W16 => 16,
            Width::W32 => 32,
            Width::W64 => 64,
        }
    }

    /// The bits a value of the width may have set.
    pub const fn mask(self) -> u64 {
        match self {
            Width::W64 => u64::MAX,
            other => (1u64 << other.bits()) - 1,
        }
    }

    /// `value` wrapped at the width.
    pub const fn wrap(self, value: u64) -> u64 {
        value & self.mask()
    }

    /// Whether `value` fits the width.
    pub const fn fits(self, value: u64) -> bool {
        value & !self.mask() == 0
    }

    /// The type's name.
    pub const fn name(self) -> &'static str {
        match self {
            Width::W8 => "u8",
            Width::W16 => "u16",
            Width::W32 => "u32",
            Width::W64 => "u64",
        }
    }
}

/// An arithmetic or bitwise operation on integers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArithOp {
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
}

/// `a op b` at `width`, with the DSL's integer arithmetic: `+`, `-` and `*` wrap at the width,
/// division and remainder by zero yield zero, a shift by the width or more yields zero.
pub fn arith(op: ArithOp, width: Width, a: u64, b: u64) -> u64 {
    match op {
        ArithOp::Add => width.wrap(a.wrapping_add(b)),
        ArithOp::Sub => width.wrap(a.wrapping_sub(b)),
        ArithOp::Mul => width.wrap(a.wrapping_mul(b)),
        ArithOp::Div => a.checked_div(b).unwrap_or(0),
        ArithOp::Rem => a.checked_rem(b).unwrap_or(0),
        ArithOp::BitAnd => a & b,
        ArithOp::BitOr => a | b,
        ArithOp::BitXor => a ^ b,
        ArithOp::Shl => {
            if b >= u64::from(width.bits()) {
                0
            } else {
                width.wrap(a << b)
            }
        }
        ArithOp::Shr => {
            if b >= u64::from(width.bits()) {
                0
            } else {
                a >> b
            }
        }
    }
}

/// At most `capacity` items waiting to go out in one event, with the provenance of what it
/// holds: the interpreter's [`uiharu_core::dsl::Batch`].
#[derive(Clone, Debug, PartialEq)]
pub struct Batch {
    capacity: u32,
    items: Vec<Value>,
    at: Option<Provenance>,
}

impl Batch {
    /// An empty batch of at most `capacity` items.
    pub fn new(capacity: u32) -> Self {
        Self {
            capacity,
            items: Vec::new(),
            at: None,
        }
    }

    pub fn len(&self) -> u32 {
        u32::try_from(self.items.len()).unwrap_or(u32::MAX)
    }

    pub fn capacity(&self) -> u32 {
        self.capacity
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn is_full(&self) -> bool {
        self.len() >= self.capacity
    }

    pub fn at(&self) -> Option<Provenance> {
        self.at
    }

    /// Adds an item, joining its provenance into the batch's; a full batch takes nothing, as
    /// the runtime's batch does outside debug builds.
    pub fn push(&mut self, item: Value, at: Provenance) {
        if self.is_full() {
            return;
        }
        self.at = Some(self.at.map_or(at, |have| have.join(at)));
        self.items.push(item);
    }

    /// Takes every item out: the items and the provenance of all of them.
    pub fn take(&mut self) -> (Vec<Value>, Option<Provenance>) {
        (std::mem::take(&mut self.items), self.at.take())
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.at = None;
    }
}

/// A value of the DSL, or one of the runtime's own.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// Nothing: the result of a statement-like operation, or an uninitialized slot.
    Unit,
    /// The missing value of an option.
    None,
    Bool(bool),
    /// An integer of any width, below its width.
    Int(u64),
    /// Text: only the messages of configuration checks.
    Text(Arc<str>),
    /// A variant of an enum, by its index in the declaration.
    Enum(u16),
    /// A record: its fields in declaration order.
    Record(Rc<[Value]>),
    /// A union value: the variant's index in the declaration and its fields in order.
    Variant(u16, Rc<[Value]>),
    /// A bounded array.
    Array(Rc<Vec<Value>>),
    Mask(ChannelMask),
    Levels(Levels),
    Range(TickRange),
    Provenance(Provenance),
    Batch(Rc<Batch>),
}

impl Value {
    /// A record of `fields`.
    pub fn record(fields: Vec<Value>) -> Self {
        Value::Record(Rc::from(fields))
    }

    /// The variant `tag` of a union, with `fields`.
    pub fn variant(tag: u16, fields: Vec<Value>) -> Self {
        Value::Variant(tag, Rc::from(fields))
    }

    /// The integer, if this is one.
    pub fn as_int(&self) -> Option<u64> {
        match self {
            Value::Int(value) => Some(*value),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(value) => Some(*value),
            _ => None,
        }
    }

    /// What the value is, for diagnostics.
    pub fn describe(&self) -> &'static str {
        match self {
            Value::Unit => "nothing",
            Value::None => "a missing value",
            Value::Bool(_) => "a boolean",
            Value::Int(_) => "an integer",
            Value::Text(_) => "text",
            Value::Enum(_) => "an enum value",
            Value::Record(_) => "a record",
            Value::Variant(..) => "a union value",
            Value::Array(_) => "an array",
            Value::Mask(_) => "a channel mask",
            Value::Levels(_) => "levels",
            Value::Range(_) => "a tick range",
            Value::Provenance(_) => "a provenance",
            Value::Batch(_) => "a batch",
        }
    }
}

impl fmt::Display for Value {
    /// The value as a message writes it: integers in decimal, text as it is.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Bool(value) => write!(f, "{value}"),
            Value::Int(value) => write!(f, "{value}"),
            Value::Text(text) => f.write_str(text),
            Value::None => f.write_str("none"),
            other => write!(f, "{other:?}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uiharu_core::Tick;

    #[test]
    fn integers_wrap_at_their_width_and_divide_by_zero_to_zero() {
        assert_eq!(arith(ArithOp::Add, Width::W8, 250, 10), 4);
        assert_eq!(
            arith(ArithOp::Sub, Width::W16, 3, 5),
            u64::from(u16::MAX) - 1
        );
        assert_eq!(arith(ArithOp::Mul, Width::W32, 0x8000_0000, 2), 0);
        assert_eq!(arith(ArithOp::Div, Width::W64, 7, 0), 0);
        assert_eq!(arith(ArithOp::Rem, Width::W64, 7, 0), 0);
        assert_eq!(arith(ArithOp::Div, Width::W64, 7, 2), 3);
        assert_eq!(arith(ArithOp::Shl, Width::W8, 0x80, 1), 0);
        assert_eq!(arith(ArithOp::Shl, Width::W8, 1, 8), 0);
        assert_eq!(arith(ArithOp::Shl, Width::W16, 1, 8), 0x100);
        assert_eq!(arith(ArithOp::Shr, Width::W16, 0x100, 8), 1);
        assert_eq!(arith(ArithOp::Shr, Width::W64, 1, 64), 0);
        assert_eq!(arith(ArithOp::Sub, Width::W64, 0, 1), u64::MAX);
        assert!(Width::W8.fits(255));
        assert!(!Width::W8.fits(256));
        assert!(Width::W64.fits(u64::MAX));
    }

    #[test]
    fn batches_join_provenance_and_hand_everything_out() {
        let mut batch = Batch::new(2);
        assert!(batch.is_empty());
        batch.push(
            Value::Int(1),
            Provenance::over(TickRange::new(Tick(10), Tick(20)), Tick(19)),
        );
        batch.push(
            Value::Int(2),
            Provenance::over(TickRange::new(Tick(20), Tick(30)), Tick(29)),
        );
        assert!(batch.is_full());
        batch.push(Value::Int(3), Provenance::at(Tick(40)));
        assert_eq!(batch.len(), 2, "a full batch takes nothing");
        assert_eq!(
            batch.at().map(|at| at.span),
            Some(TickRange::new(Tick(10), Tick(30)))
        );
        let (items, at) = batch.take();
        assert_eq!(items, vec![Value::Int(1), Value::Int(2)]);
        assert_eq!(at.map(|at| at.settled), Some(Tick(29)));
        assert!(batch.is_empty());
        assert_eq!(batch.take().1, None);
    }
}
