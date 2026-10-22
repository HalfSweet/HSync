// SPDX-License-Identifier: Apache-2.0

//! The machine: a decoder's state and the loop that runs its instructions over the runtime's
//! handles. Every wait is one of the runtime's operations, awaited where the instruction is;
//! what the operation finds becomes a value on the stack. The main loops of
//! `uiharu_core::dsl` drive the hooks through the drivers of [`crate::node`], so that what
//! happens around the body — progress held back, cuts, recovery — is the runtime's own.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};

use uiharu_core::dsl::{
    BusIo, Cut, LogicDecoder, LogicIo, Read, ReadError, StreamDecoder, StreamIo, Timed,
    Transaction, mul_div, mul_div_ceil,
};
use uiharu_core::{ChannelMask, Provenance, Tick, TickRange};

use crate::event::DynEvent;
use crate::framer::DynFramer;
use crate::json;
use crate::node::{LogicDriver, StreamDriver};
use crate::program::{
    Base, BatchQuery, CmpOp, Const, FuncId, Input, Instr, Intrinsic, Op, Place, Program, Step,
};
use crate::types::Ty;
use crate::value::{Batch, Value, arith};

/// What stops a function: a cut of the body, which the main loop closes, or a fault of the
/// interpreter, which ends decoding.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Fault {
    Cut(Cut),
    Internal(String),
}

impl From<Cut> for Fault {
    fn from(cut: Cut) -> Self {
        Fault::Cut(cut)
    }
}

impl From<ReadError> for Fault {
    fn from(error: ReadError) -> Self {
        Fault::Cut(Cut::from(error))
    }
}

fn internal(message: impl Into<String>) -> Fault {
    Fault::Internal(message.into())
}

/// Where an interpreted decoder reports a fault of its own — a construct the module's checks
/// let through, an operation without its input — which ends its decoding: what it emitted
/// before stays, nothing follows but the pass-through of its stream. Clones share the state.
#[derive(Clone, Debug, Default)]
pub struct FaultHandle(Rc<RefCell<Option<String>>>);

impl FaultHandle {
    /// The first fault, if there was one.
    pub fn get(&self) -> Option<String> {
        self.0.borrow().clone()
    }

    /// Reports `message`, unless a fault was reported already.
    pub fn report(&self, message: String) {
        let mut fault = self.0.borrow_mut();
        if fault.is_none() {
            *fault = Some(message);
        }
    }
}

/// The runtime's handles an execution has: none for a synchronous function, or the handles
/// of the decoder's kind, with the transaction cursor while a frame is read.
pub(crate) enum Cx<'a, 'b> {
    None,
    Logic {
        decoder: &'a mut LogicDriver,
        io: &'a mut LogicIo<DynEvent>,
    },
    Stream {
        decoder: &'a mut StreamDriver,
        io: &'a mut StreamIo<DynEvent, DynEvent>,
    },
    Bus {
        io: &'a mut BusIo<DynEvent>,
        tx: Option<&'a mut Transaction<'b, DynEvent, DynFramer>>,
    },
}

#[derive(Clone, Copy)]
struct Frame {
    func: FuncId,
    pc: usize,
    base: usize,
}

/// A decoder's state and the loop that runs its functions.
pub(crate) struct Machine {
    pub(crate) program: Arc<Program>,
    /// The configuration, the rate, then the declared fields.
    fields: RefCell<Vec<Value>>,
    pub(crate) framer: Option<DynFramer>,
    fault: FaultHandle,
    faulted: Cell<bool>,
}

impl Machine {
    /// A machine over `fields`, reporting faults through `fault`.
    pub(crate) fn new(
        program: Arc<Program>,
        fields: Vec<Value>,
        framer: Option<DynFramer>,
        fault: FaultHandle,
    ) -> Self {
        Self {
            program,
            fields: RefCell::new(fields),
            framer,
            fault,
            faulted: Cell::new(false),
        }
    }

    /// A machine without state, for the functions over the configuration alone.
    pub(crate) fn detached(program: Arc<Program>) -> Self {
        Self::new(program, Vec::new(), None, FaultHandle::default())
    }

    pub(crate) fn set_field(&self, index: usize, value: Value) {
        if let Some(slot) = self.fields.borrow_mut().get_mut(index) {
            *slot = value;
        }
    }

    pub(crate) fn record(&self, fault: Fault) {
        if let Fault::Internal(message) = fault {
            self.faulted.set(true);
            self.fault.report(message);
        }
    }

    /// Runs a synchronous function to its result.
    pub(crate) fn execute_sync(&self, func: FuncId, args: Vec<Value>) -> Result<Value, Fault> {
        let mut cx = Cx::None;
        let future = std::pin::pin!(self.execute(&mut cx, func, args));
        match future.poll(&mut Context::from_waker(Waker::noop())) {
            Poll::Ready(result) => result,
            Poll::Pending => Err(internal("a synchronous function waited")),
        }
    }

    /// The `hold` hook: from which tick on the decoder holds what it has not reported.
    pub(crate) fn hold(&self) -> Option<u64> {
        let hook = self.program.hooks.hold?;
        match self.execute_sync(hook, Vec::new()) {
            Ok(Value::Int(tick)) => Some(tick),
            Ok(Value::None) => None,
            Ok(other) => {
                self.record(internal(format!("hold returned {}", other.describe())));
                None
            }
            Err(fault) => {
                self.record(fault);
                None
            }
        }
    }

    /// Runs a hook with the runtime's handles: a cut of the body goes to the main loop, a
    /// fault ends decoding, after which every hook does nothing.
    pub(crate) async fn hook(
        &self,
        cx: &mut Cx<'_, '_>,
        func: Option<FuncId>,
        args: Vec<Value>,
    ) -> Result<(), Cut> {
        let Some(func) = func else {
            return Ok(());
        };
        if self.faulted.get() {
            return Ok(());
        }
        match self.execute(cx, func, args).await {
            Ok(_) => Ok(()),
            Err(Fault::Cut(cut)) => Err(cut),
            Err(fault) => {
                self.record(fault);
                Ok(())
            }
        }
    }

    /// Runs `func` on `args` to its result.
    pub(crate) async fn execute(
        &self,
        cx: &mut Cx<'_, '_>,
        func: FuncId,
        args: Vec<Value>,
    ) -> Result<Value, Fault> {
        let program = Arc::clone(&self.program);
        let mut stack: Vec<Value> = Vec::with_capacity(16);
        let mut locals: Vec<Value> = Vec::with_capacity(32);
        let mut frames: Vec<Frame> = Vec::new();
        let mut frame = Frame {
            func,
            pc: 0,
            base: 0,
        };
        locals.resize(program.function(func).locals as usize, Value::Unit);
        for (slot, arg) in args.into_iter().enumerate() {
            if let Some(local) = locals.get_mut(slot) {
                *local = arg;
            }
        }
        loop {
            let function = program.function(frame.func);
            let Some(instr) = function.code.get(frame.pc) else {
                return Err(internal(format!("{} ran off its code", function.name)));
            };
            frame.pc += 1;
            match instr {
                Instr::Const(value) => stack.push(constant(value)),
                Instr::LoadLocal(slot) => stack.push(local(&locals, frame.base, *slot)?.clone()),
                Instr::StoreLocal(slot) => {
                    let value = pop(&mut stack)?;
                    *local_mut(&mut locals, frame.base, *slot)? = value;
                }
                Instr::LoadPath(place) => {
                    let indices = indices(&mut stack, place)?;
                    let value = self.load_path(&locals, frame.base, place, &indices)?;
                    stack.push(value);
                }
                Instr::StorePath(place) => {
                    let value = pop(&mut stack)?;
                    let indices = indices(&mut stack, place)?;
                    let writable = !matches!(cx, Cx::None);
                    self.store_path(writable, &mut locals, frame.base, place, &indices, value)?;
                }
                Instr::TakePath(place) => {
                    let indices = indices(&mut stack, place)?;
                    let writable = !matches!(cx, Cx::None);
                    let value =
                        self.take_path(writable, &mut locals, frame.base, place, &indices)?;
                    stack.push(value);
                }
                Instr::Pop => {
                    pop(&mut stack)?;
                }
                Instr::Dup => {
                    let top = stack
                        .last()
                        .cloned()
                        .ok_or_else(|| internal("an empty stack"))?;
                    stack.push(top);
                }
                Instr::Swap => {
                    let a = pop(&mut stack)?;
                    let b = pop(&mut stack)?;
                    stack.push(a);
                    stack.push(b);
                }
                Instr::GetField(index) => {
                    let value = pop(&mut stack)?;
                    stack.push(field_of(&value, *index)?);
                }
                Instr::Index => {
                    let at = pop_int(&mut stack)?;
                    let of = pop(&mut stack)?;
                    stack.push(item_of(&of, at)?);
                }
                Instr::MakeRecord(count) => {
                    let fields = split(&mut stack, *count)?;
                    stack.push(Value::record(fields));
                }
                Instr::MakeVariant { tag, fields } => {
                    let fields = split(&mut stack, *fields)?;
                    stack.push(Value::variant(*tag, fields));
                }
                Instr::MakeArray(count) => {
                    let items = split(&mut stack, *count)?;
                    stack.push(Value::Array(Rc::new(items)));
                }
                Instr::Binary { op, width } => {
                    let b = pop_int(&mut stack)?;
                    let a = pop_int(&mut stack)?;
                    stack.push(Value::Int(arith(*op, *width, a, b)));
                }
                Instr::Compare(op) => {
                    let b = pop(&mut stack)?;
                    let a = pop(&mut stack)?;
                    stack.push(Value::Bool(compare(*op, &a, &b)?));
                }
                Instr::Not => {
                    let value = pop_bool(&mut stack)?;
                    stack.push(Value::Bool(!value));
                }
                Instr::BitNot(width) => {
                    let value = pop_int(&mut stack)?;
                    stack.push(Value::Int(width.wrap(!value)));
                }
                Instr::Narrow(width) => {
                    let value = pop_int(&mut stack)?;
                    stack.push(Value::Int(width.wrap(value)));
                }
                Instr::EnumName(index) => {
                    let value = pop(&mut stack)?;
                    let Value::Enum(variant) = value else {
                        return Err(internal(format!(
                            "{} is not an enum value",
                            value.describe()
                        )));
                    };
                    let name = program
                        .types
                        .enums
                        .get(*index as usize)
                        .and_then(|def| match &def.numbers {
                            Some(numbers) => {
                                numbers.get(usize::from(variant)).map(ToString::to_string)
                            }
                            None => def.variants.get(usize::from(variant)).cloned(),
                        })
                        .unwrap_or_default();
                    stack.push(Value::Text(name.into()));
                }
                Instr::Jump(target) => frame.pc = *target as usize,
                Instr::JumpIfFalse(target) => {
                    if !pop_bool(&mut stack)? {
                        frame.pc = *target as usize;
                    }
                }
                Instr::JumpIfTrue(target) => {
                    if pop_bool(&mut stack)? {
                        frame.pc = *target as usize;
                    }
                }
                Instr::IsNone => {
                    let value = pop(&mut stack)?;
                    stack.push(Value::Bool(value == Value::None));
                }
                Instr::IsVariant(tag) => {
                    let value = pop(&mut stack)?;
                    stack.push(Value::Bool(
                        matches!(value, Value::Variant(have, _) if have == *tag),
                    ));
                }
                Instr::IsIn(consts) => {
                    let value = pop(&mut stack)?;
                    stack.push(Value::Bool(consts.iter().any(|c| const_eq(c, &value))));
                }
                Instr::Destructure(pairs) => {
                    let value = pop(&mut stack)?;
                    let fields = match &value {
                        Value::Record(fields) | Value::Variant(_, fields) => fields,
                        other => {
                            return Err(internal(format!("{} has no fields", other.describe())));
                        }
                    };
                    for (field, slot) in pairs {
                        let field = fields
                            .get(*field as usize)
                            .cloned()
                            .ok_or_else(|| internal("a field that is not there"))?;
                        *local_mut(&mut locals, frame.base, *slot)? = field;
                    }
                }
                Instr::ForEachNext {
                    array,
                    index,
                    var,
                    end,
                } => {
                    let at = local(&locals, frame.base, *index)?
                        .as_int()
                        .ok_or_else(|| internal("an index that is not an integer"))?;
                    let item = match local(&locals, frame.base, *array)? {
                        Value::Array(items) => items
                            .get(usize::try_from(at).unwrap_or(usize::MAX))
                            .cloned(),
                        other => {
                            return Err(internal(format!("iteration over {}", other.describe())));
                        }
                    };
                    match item {
                        Some(item) => {
                            *local_mut(&mut locals, frame.base, *var)? = item;
                            *local_mut(&mut locals, frame.base, *index)? = Value::Int(at + 1);
                        }
                        None => frame.pc = *end as usize,
                    }
                }
                Instr::Call(id) => {
                    let callee = program.function(*id);
                    let base = locals.len();
                    locals.resize(base + callee.locals as usize, Value::Unit);
                    for slot in (0..callee.params as usize).rev() {
                        locals[base + slot] = pop(&mut stack)?;
                    }
                    frames.push(frame);
                    frame = Frame {
                        func: *id,
                        pc: 0,
                        base,
                    };
                }
                Instr::Ret => {
                    locals.truncate(frame.base);
                    match frames.pop() {
                        Some(caller) => frame = caller,
                        None => return Ok(stack.pop().unwrap_or(Value::Unit)),
                    }
                }
                Instr::Required => {
                    let value = pop(&mut stack)?;
                    if value == Value::None {
                        return Err(Fault::Cut(Cut::Missing));
                    }
                    stack.push(value);
                }
                Instr::Format(count) => {
                    let parts = split(&mut stack, *count)?;
                    let text: String = parts.iter().map(ToString::to_string).collect();
                    stack.push(Value::Text(text.into()));
                }
                Instr::Intrinsic(intrinsic) => self.intrinsic(cx, *intrinsic, &mut stack)?,
                Instr::NewBatch(capacity) => {
                    stack.push(Value::Batch(Rc::new(Batch::new(*capacity))))
                }
                Instr::BatchPush => {
                    let at = pop_provenance(&mut stack)?;
                    let item = pop(&mut stack)?;
                    let mut batch = pop_batch(&mut stack)?;
                    Rc::make_mut(&mut batch).push(item, at);
                    stack.push(Value::Batch(batch));
                }
                Instr::BatchTake => {
                    let mut batch = pop_batch(&mut stack)?;
                    let (items, at) = Rc::make_mut(&mut batch).take();
                    stack.push(Value::Batch(batch));
                    stack.push(Value::record(vec![
                        Value::Array(Rc::new(items)),
                        at.map_or(Value::None, Value::Provenance),
                    ]));
                }
                Instr::BatchClear => {
                    let mut batch = pop_batch(&mut stack)?;
                    Rc::make_mut(&mut batch).clear();
                    stack.push(Value::Batch(batch));
                }
                Instr::BatchQuery(query) => {
                    let batch = pop_batch(&mut stack)?;
                    stack.push(match query {
                        BatchQuery::Length => Value::Int(u64::from(batch.len())),
                        BatchQuery::Capacity => Value::Int(u64::from(batch.capacity())),
                        BatchQuery::IsEmpty => Value::Bool(batch.is_empty()),
                        BatchQuery::IsFull => Value::Bool(batch.is_full()),
                        BatchQuery::At => batch.at().map_or(Value::None, Value::Provenance),
                    });
                }
                Instr::Await(op) => {
                    let value = self.operation(cx, *op, &mut stack).await?;
                    stack.push(value);
                }
            }
        }
    }

    // Places

    fn load_path(
        &self,
        locals: &[Value],
        base: usize,
        place: &Place,
        indices: &[u64],
    ) -> Result<Value, Fault> {
        let mut value = match place.base {
            Base::Local(slot) => local(locals, base, slot)?.clone(),
            Base::Field(index) => self
                .fields
                .borrow()
                .get(index as usize)
                .cloned()
                .ok_or_else(|| internal("a field that is not there"))?,
        };
        let mut next = 0;
        for step in &place.steps {
            value = match step {
                Step::Field(index) => field_of(&value, *index)?,
                Step::At(at) => item_of(&value, u64::from(*at))?,
                Step::Index => {
                    let at = indices.get(next).copied().unwrap_or(u64::MAX);
                    next += 1;
                    item_of(&value, at)?
                }
            };
        }
        Ok(value)
    }

    fn store_path(
        &self,
        writable: bool,
        locals: &mut [Value],
        base: usize,
        place: &Place,
        indices: &[u64],
        value: Value,
    ) -> Result<(), Fault> {
        match place.base {
            Base::Local(slot) => {
                set_path(local_mut(locals, base, slot)?, &place.steps, indices, value)
            }
            Base::Field(index) => {
                if !writable {
                    return Err(internal("the decoder's state is changed outside its body"));
                }
                let mut fields = self.fields.borrow_mut();
                let slot = fields
                    .get_mut(index as usize)
                    .ok_or_else(|| internal("a field that is not there"))?;
                set_path(slot, &place.steps, indices, value)
            }
        }
    }

    fn take_path(
        &self,
        writable: bool,
        locals: &mut [Value],
        base: usize,
        place: &Place,
        indices: &[u64],
    ) -> Result<Value, Fault> {
        match place.base {
            Base::Local(slot) => take_leaf(local_mut(locals, base, slot)?, &place.steps, indices),
            Base::Field(index) => {
                if !writable {
                    return Err(internal("the decoder's state is changed outside its body"));
                }
                let mut fields = self.fields.borrow_mut();
                let slot = fields
                    .get_mut(index as usize)
                    .ok_or_else(|| internal("a field that is not there"))?;
                take_leaf(slot, &place.steps, indices)
            }
        }
    }

    // Helpers and operations

    fn intrinsic(
        &self,
        cx: &mut Cx<'_, '_>,
        intrinsic: Intrinsic,
        stack: &mut Vec<Value>,
    ) -> Result<(), Fault> {
        let value = match intrinsic {
            Intrinsic::Min => {
                let b = pop_int(stack)?;
                let a = pop_int(stack)?;
                Value::Int(a.min(b))
            }
            Intrinsic::Max => {
                let b = pop_int(stack)?;
                let a = pop_int(stack)?;
                Value::Int(a.max(b))
            }
            Intrinsic::SaturatingSub => {
                let b = pop_int(stack)?;
                let a = pop_int(stack)?;
                Value::Int(a.saturating_sub(b))
            }
            Intrinsic::CountOnes => Value::Int(u64::from(pop_int(stack)?.count_ones())),
            Intrinsic::FromBool => Value::Int(u64::from(pop_bool(stack)?)),
            Intrinsic::MulDiv | Intrinsic::MulDivCeil => {
                let c = pop_int(stack)?;
                let b = pop_int(stack)?;
                let a = pop_int(stack)?;
                Value::Int(if intrinsic == Intrinsic::MulDiv {
                    mul_div(a, b, c)
                } else {
                    mul_div_ceil(a, b, c)
                })
            }
            Intrinsic::At => Value::Provenance(Provenance::at(Tick(pop_int(stack)?))),
            Intrinsic::One => Value::Provenance(Provenance::one(Tick(pop_int(stack)?))),
            Intrinsic::Over => {
                let settled = pop_int(stack)?;
                let end = pop_int(stack)?;
                let start = pop_int(stack)?;
                Value::Provenance(Provenance::over(
                    TickRange::new(Tick(start), Tick(end)),
                    Tick(settled),
                ))
            }
            Intrinsic::Join => {
                let b = pop_provenance(stack)?;
                let a = pop_provenance(stack)?;
                Value::Provenance(a.join(b))
            }
            Intrinsic::Mask(count) => {
                let channels = split(stack, count)?;
                let mut mask = ChannelMask::EMPTY;
                for channel in channels {
                    match channel {
                        Value::None => {}
                        Value::Int(channel) => mask = mask.with(channel_of(channel)),
                        other => {
                            return Err(internal(format!("{} is not a channel", other.describe())));
                        }
                    }
                }
                Value::Mask(mask)
            }
            Intrinsic::MaskHas => {
                let channel = channel_of(pop_int(stack)?);
                Value::Bool(pop_mask(stack)?.has(channel))
            }
            Intrinsic::MaskWith => {
                let channel = channel_of(pop_int(stack)?);
                Value::Mask(pop_mask(stack)?.with(channel))
            }
            Intrinsic::LevelsHigh => {
                let channel = channel_of(pop_int(stack)?);
                let levels = match pop(stack)? {
                    Value::Levels(levels) => levels,
                    other => return Err(internal(format!("{} are not levels", other.describe()))),
                };
                Value::Bool(levels.high(channel))
            }
            Intrinsic::Position => match cx {
                Cx::Logic { io, .. } => Value::Int(io.signal.position().0),
                _ => return Err(internal("position without logic samples")),
            },
            Intrinsic::Seek => {
                let tick = pop_int(stack)?;
                match cx {
                    Cx::Logic { io, .. } => io.signal.seek(Tick(tick)),
                    _ => return Err(internal("seek without logic samples")),
                }
                Value::Unit
            }
            Intrinsic::Consumed => match cx {
                Cx::Bus { io, .. } => Value::Provenance(io.consumed()),
                _ => return Err(internal("consumed without a bus")),
            },
            Intrinsic::TxId => {
                pop(stack)?;
                match cx {
                    Cx::Bus { tx: Some(tx), .. } => Value::Int(tx.id()),
                    _ => return Err(internal("the transaction's id outside its body")),
                }
            }
            Intrinsic::TxPartial => {
                pop(stack)?;
                match cx {
                    Cx::Bus { tx: Some(tx), .. } => Value::Bool(tx.is_partial()),
                    _ => return Err(internal("the transaction's state outside its body")),
                }
            }
            Intrinsic::UnwrapOr => {
                let default = pop(stack)?;
                let value = pop(stack)?;
                if value == Value::None { default } else { value }
            }
        };
        stack.push(value);
        Ok(())
    }

    async fn operation(
        &self,
        cx: &mut Cx<'_, '_>,
        op: Op,
        stack: &mut Vec<Value>,
    ) -> Result<Value, Fault> {
        match op {
            Op::NextChange => {
                let mask = pop_mask(stack)?;
                let Cx::Logic { decoder, io } = cx else {
                    return Err(internal("next-change without logic samples"));
                };
                let edge = decoder.next_change(io, mask).await?;
                Ok(Value::record(vec![
                    Value::Int(edge.tick),
                    Value::Mask(edge.changed),
                    Value::Levels(edge.levels),
                ]))
            }
            Op::NextChangeBefore => {
                let until = pop_int(stack)?;
                let mask = pop_mask(stack)?;
                let Cx::Logic { decoder, io } = cx else {
                    return Err(internal("next-change-before without logic samples"));
                };
                Ok(match decoder.next_change_before(io, mask, until).await? {
                    Timed::Edge {
                        tick,
                        changed,
                        levels,
                    } => Value::variant(
                        0,
                        vec![
                            Value::Int(tick),
                            Value::Mask(changed),
                            Value::Levels(levels),
                        ],
                    ),
                    Timed::Quiet { tick } => Value::variant(1, vec![Value::Int(tick)]),
                })
            }
            Op::Sample => {
                let Cx::Logic { decoder, io } = cx else {
                    return Err(internal("sample without logic samples"));
                };
                Ok(Value::Levels(decoder.sample(io).await?))
            }
            Op::Next => {
                let Cx::Stream { decoder, io } = cx else {
                    return Err(internal("next without a lower stream"));
                };
                let received = decoder.next(io).await?;
                let Input::Stream { event, .. } = &self.program.input else {
                    return Err(internal("next without a lower stream"));
                };
                let event = json::from_json(
                    &self.program.types,
                    &Ty::Named(event.clone()),
                    &received.event.0,
                )
                .map_err(|message| {
                    internal(format!(
                        "a lower event does not fit its contract: {message}"
                    ))
                })?;
                Ok(Value::record(vec![event, Value::Provenance(received.at)]))
            }
            Op::Item | Op::ItemOrEnd | Op::Drain => {
                let Cx::Bus { io, tx: Some(tx) } = cx else {
                    return Err(internal("a read of the frame outside a transaction"));
                };
                io.sink.hold(self.hold().map(Tick));
                Ok(match op {
                    Op::Item => tx.item().await?,
                    Op::ItemOrEnd => match tx.item_or_end().await? {
                        Read::Item(item) => item,
                        Read::End => Value::None,
                    },
                    _ => {
                        tx.drain().await?;
                        Value::Unit
                    }
                })
            }
            Op::Emit { at } => {
                let event = pop(stack)?;
                let at = if at {
                    pop_provenance(stack)?
                } else {
                    match cx {
                        Cx::Bus { io, .. } => io.consumed(),
                        _ => {
                            return Err(internal(
                                "an event without provenance outside a bus decoder",
                            ));
                        }
                    }
                };
                let json = json::to_json(
                    &self.program.types,
                    &Ty::Named(self.program.event.clone()),
                    &event,
                );
                let payload = DynEvent(json);
                match cx {
                    Cx::Logic { io, .. } => io.out.emit(at, payload).await,
                    Cx::Stream { io, .. } => io.out.emit(at, payload).await,
                    Cx::Bus { io, .. } => io.out.emit(at, payload).await,
                    Cx::None => return Err(internal("an event emitted outside the body")),
                }
                Ok(Value::Unit)
            }
        }
    }
}

fn constant(value: &Const) -> Value {
    match value {
        Const::Unit => Value::Unit,
        Const::None => Value::None,
        Const::Bool(b) => Value::Bool(*b),
        Const::Int(n) => Value::Int(*n),
        Const::Enum(index) => Value::Enum(*index),
        Const::UnitVariant(index) => Value::variant(*index, Vec::new()),
        Const::Text(text) => Value::Text(Arc::clone(text)),
    }
}

fn const_eq(constant: &Const, value: &Value) -> bool {
    match (constant, value) {
        (Const::Unit, Value::Unit) | (Const::None, Value::None) => true,
        (Const::Bool(a), Value::Bool(b)) => a == b,
        (Const::Int(a), Value::Int(b)) => a == b,
        (Const::Enum(a), Value::Enum(b)) => a == b,
        (Const::UnitVariant(a), Value::Variant(b, fields)) => a == b && fields.is_empty(),
        (Const::Text(a), Value::Text(b)) => **a == **b,
        _ => false,
    }
}

fn channel_of(value: u64) -> u8 {
    u8::try_from(value).unwrap_or(u8::MAX)
}

fn pop(stack: &mut Vec<Value>) -> Result<Value, Fault> {
    stack.pop().ok_or_else(|| internal("an empty stack"))
}

fn pop_int(stack: &mut Vec<Value>) -> Result<u64, Fault> {
    match pop(stack)? {
        Value::Int(value) => Ok(value),
        other => Err(internal(format!("{} is not an integer", other.describe()))),
    }
}

fn pop_bool(stack: &mut Vec<Value>) -> Result<bool, Fault> {
    match pop(stack)? {
        Value::Bool(value) => Ok(value),
        other => Err(internal(format!("{} is not a boolean", other.describe()))),
    }
}

fn pop_mask(stack: &mut Vec<Value>) -> Result<ChannelMask, Fault> {
    match pop(stack)? {
        Value::Mask(mask) => Ok(mask),
        other => Err(internal(format!("{} is not a mask", other.describe()))),
    }
}

fn pop_provenance(stack: &mut Vec<Value>) -> Result<Provenance, Fault> {
    match pop(stack)? {
        Value::Provenance(at) => Ok(at),
        other => Err(internal(format!(
            "{} is not a provenance",
            other.describe()
        ))),
    }
}

fn pop_batch(stack: &mut Vec<Value>) -> Result<Rc<Batch>, Fault> {
    match pop(stack)? {
        Value::Batch(batch) => Ok(batch),
        other => Err(internal(format!("{} is not a batch", other.describe()))),
    }
}

/// The last `count` values, in order.
fn split(stack: &mut Vec<Value>, count: u32) -> Result<Vec<Value>, Fault> {
    let count = count as usize;
    if stack.len() < count {
        return Err(internal("an empty stack"));
    }
    Ok(stack.split_off(stack.len() - count))
}

/// The dynamic indices of `place`, popped from the stack, in order.
fn indices(stack: &mut Vec<Value>, place: &Place) -> Result<Vec<u64>, Fault> {
    let count = place.dynamic_indices();
    let mut out = vec![0; count];
    for slot in out.iter_mut().rev() {
        *slot = pop_int(stack)?;
    }
    Ok(out)
}

fn local(locals: &[Value], base: usize, slot: u32) -> Result<&Value, Fault> {
    locals
        .get(base + slot as usize)
        .ok_or_else(|| internal("a local that is not there"))
}

fn local_mut(locals: &mut [Value], base: usize, slot: u32) -> Result<&mut Value, Fault> {
    locals
        .get_mut(base + slot as usize)
        .ok_or_else(|| internal("a local that is not there"))
}

/// Field `index` of a record, a union value or one of the runtime's own values.
fn field_of(value: &Value, index: u32) -> Result<Value, Fault> {
    match (value, index) {
        (Value::Record(fields) | Value::Variant(_, fields), _) => fields
            .get(index as usize)
            .cloned()
            .ok_or_else(|| internal("a field that is not there")),
        (Value::Provenance(at), 0) => Ok(Value::Range(at.span)),
        (Value::Provenance(at), 1) => Ok(Value::Int(at.settled.0)),
        (Value::Range(range), 0) => Ok(Value::Int(range.start.0)),
        (Value::Range(range), 1) => Ok(Value::Int(range.end.0)),
        (other, _) => Err(internal(format!(
            "{} has no field {index}",
            other.describe()
        ))),
    }
}

fn item_of(value: &Value, at: u64) -> Result<Value, Fault> {
    match value {
        Value::Array(items) => items
            .get(usize::try_from(at).unwrap_or(usize::MAX))
            .cloned()
            .ok_or_else(|| internal(format!("index {at} is past the end of the array"))),
        other => Err(internal(format!("{} is not an array", other.describe()))),
    }
}

/// Writes `value` into `slot` at `steps`, copying what is shared on the way.
fn set_path(slot: &mut Value, steps: &[Step], indices: &[u64], value: Value) -> Result<(), Fault> {
    let child = child_mut(slot, steps, indices)?;
    *child = value;
    Ok(())
}

/// Moves the value at `steps` out of `slot`.
fn take_leaf(slot: &mut Value, steps: &[Step], indices: &[u64]) -> Result<Value, Fault> {
    let child = child_mut(slot, steps, indices)?;
    Ok(std::mem::replace(child, Value::Unit))
}

fn child_mut<'v>(
    slot: &'v mut Value,
    steps: &[Step],
    indices: &[u64],
) -> Result<&'v mut Value, Fault> {
    let Some((step, rest)) = steps.split_first() else {
        return Ok(slot);
    };
    match step {
        Step::Field(index) => match slot {
            Value::Record(fields) | Value::Variant(_, fields) => {
                let fields = Rc::make_mut(fields);
                let child = fields
                    .get_mut(*index as usize)
                    .ok_or_else(|| internal("a field that is not there"))?;
                child_mut(child, rest, indices)
            }
            other => Err(internal(format!(
                "{} has no field {index}",
                other.describe()
            ))),
        },
        Step::At(_) | Step::Index => {
            let (at, indices) = match step {
                Step::At(at) => (u64::from(*at), indices),
                _ => {
                    let (first, rest) = indices
                        .split_first()
                        .ok_or_else(|| internal("an index that is not there"))?;
                    (*first, rest)
                }
            };
            match slot {
                Value::Array(items) => {
                    let items = Rc::make_mut(items);
                    let child = items
                        .get_mut(usize::try_from(at).unwrap_or(usize::MAX))
                        .ok_or_else(|| {
                            internal(format!("index {at} is past the end of the array"))
                        })?;
                    child_mut(child, rest, indices)
                }
                other => Err(internal(format!("{} is not an array", other.describe()))),
            }
        }
    }
}

fn compare(op: CmpOp, a: &Value, b: &Value) -> Result<bool, Fault> {
    match op {
        CmpOp::Eq => Ok(a == b),
        CmpOp::Ne => Ok(a != b),
        _ => {
            let (Value::Int(a), Value::Int(b)) = (a, b) else {
                return Err(internal(format!(
                    "{} and {} are not ordered",
                    a.describe(),
                    b.describe()
                )));
            };
            Ok(match op {
                CmpOp::Lt => a < b,
                CmpOp::Le => a <= b,
                CmpOp::Gt => a > b,
                _ => a >= b,
            })
        }
    }
}
