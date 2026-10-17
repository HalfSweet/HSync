// SPDX-License-Identifier: Apache-2.0

//! From the IR to the interpreter's instructions: one pass over each function that types
//! every expression the way the Rust backend does — arithmetic at the width of its context
//! when wider than its operands, options without a wrapper, records by value — resolves names
//! to slots and indices, flattens the control flow into jumps and emits the code. What the
//! backend would not emit is refused here, with the path of the construct.

use std::collections::HashMap;

use uiharu_core::SchemaId;
use uiharu_core::dsl::Partial;
use uiharu_ir::{
    BinaryOp, Expr, Function as IrFunction, Input as IrInput, Literal, Module, Op as IrOp, Pattern,
    Stmt, UnaryOp,
};

use crate::intern::intern;
use crate::program::{
    BatchQuery, CmpOp, Const, FramingData, FuncId, Function, Hooks, Input, Instr, Intrinsic, Kind,
    LoadError, Op, Place, Program, Prop, Recovery, Require, StateField, Step,
};
use crate::types::{FieldDef, Ty, TypeTable, builtin_field, item_type_name, timed_variant};
use crate::value::{ArithOp, Width};

type Result<T> = std::result::Result<T, LoadError>;

/// A parameter as the compiler knows it.
#[derive(Clone, Debug)]
struct ParamSig {
    name: String,
    ty: Ty,
    mutable: bool,
}

/// A function or method as the compiler knows it.
#[derive(Clone, Debug)]
struct Sig {
    id: FuncId,
    params: Vec<ParamSig>,
    ret: Ty,
    is_async: bool,
}

/// What every function's compiler is given: the types, the signatures and the state.
struct Env<'a> {
    types: &'a TypeTable,
    consts: HashMap<String, (Const, Ty)>,
    functions: HashMap<String, Sig>,
    methods: HashMap<String, Sig>,
    /// The fields of the decoder's state by name: `config` and `rate` first.
    state: HashMap<String, (u32, Ty)>,
    kind: Kind,
    /// The lower event type, for stream and bus decoders.
    lower: Option<String>,
    /// The output's event type.
    event: String,
}

/// What a name in scope stands for: a slot of the frame, or a place of the decoder's state
/// it borrows.
#[derive(Clone, Debug)]
enum Binding {
    Slot(u32, Ty),
    Alias(Place, Ty),
}

struct LoopCx {
    label: Option<String>,
    breaks: Vec<usize>,
    continues: Vec<usize>,
}

/// Compiles one function.
struct Compiler<'e> {
    env: &'e Env<'e>,
    code: Vec<Instr>,
    scopes: Vec<HashMap<String, Binding>>,
    slots: u32,
    loops: Vec<LoopCx>,
    in_method: bool,
    is_async: bool,
    has_cursor: bool,
    ret: Ty,
    outs: Vec<u32>,
    path: String,
}

fn unsupported(path: &str, message: impl Into<String>) -> LoadError {
    LoadError::Unsupported {
        path: path.to_owned(),
        message: message.into(),
    }
}

impl<'e> Compiler<'e> {
    fn new(env: &'e Env<'e>, path: &str) -> Self {
        Self {
            env,
            code: Vec::new(),
            scopes: vec![HashMap::new()],
            slots: 0,
            loops: Vec::new(),
            in_method: false,
            is_async: false,
            has_cursor: false,
            ret: Ty::Unit,
            outs: Vec::new(),
            path: path.to_owned(),
        }
    }

    fn err<T>(&self, message: impl Into<String>) -> Result<T> {
        Err(unsupported(&self.path, message))
    }

    // Emission

    fn emit(&mut self, instr: Instr) -> usize {
        self.code.push(instr);
        self.code.len() - 1
    }

    fn here(&self) -> u32 {
        u32::try_from(self.code.len()).unwrap_or(u32::MAX)
    }

    fn patch(&mut self, at: usize, target: u32) {
        match &mut self.code[at] {
            Instr::Jump(t) | Instr::JumpIfFalse(t) | Instr::JumpIfTrue(t) => *t = target,
            Instr::ForEachNext { end, .. } => *end = target,
            _ => {}
        }
    }

    fn patch_here(&mut self, at: usize) {
        let here = self.here();
        self.patch(at, here);
    }

    // Scopes

    fn slot(&mut self) -> u32 {
        let slot = self.slots;
        self.slots += 1;
        slot
    }

    fn declare(&mut self, name: &str, binding: Binding) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_owned(), binding);
        }
    }

    fn declare_slot(&mut self, name: &str, ty: Ty) -> u32 {
        let slot = self.slot();
        self.declare(name, Binding::Slot(slot, ty));
        slot
    }

    fn lookup(&self, name: &str) -> Option<&Binding> {
        self.scopes.iter().rev().find_map(|scope| scope.get(name))
    }

    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn with_path<T>(&mut self, suffix: &str, f: impl FnOnce(&mut Self) -> Result<T>) -> Result<T> {
        let saved = self.path.clone();
        self.path.push_str(suffix);
        let result = f(self);
        self.path = saved;
        result
    }

    // Functions

    /// The code of a function with `sig`'s parameters over `body`.
    fn function(mut self, sig: &Sig, is_method: bool, body: &[Stmt]) -> Result<Function> {
        self.in_method = is_method;
        self.is_async = sig.is_async;
        self.ret = sig.ret.clone();
        for param in &sig.params {
            let slot = self.declare_slot(&param.name, param.ty.clone());
            if param.ty == Ty::Cursor {
                self.has_cursor = true;
            }
            if param.mutable {
                self.outs.push(slot);
            }
        }
        self.with_path(".body", |c| c.block(body))?;
        self.emit(Instr::Const(Const::Unit));
        self.emit_outs();
        self.emit(Instr::Ret);
        Ok(Function {
            name: String::new(),
            code: self.code,
            params: u32::try_from(sig.params.len()).unwrap_or(u32::MAX),
            locals: self.slots,
            outs: self.outs,
            is_async: sig.is_async,
        })
    }

    /// The code of a function of `params` returning `expr` as `expected`, and the type of
    /// what it returns.
    fn thunk(
        mut self,
        params: &[(String, Ty)],
        in_method: bool,
        expr: &Expr,
        expected: Option<&Ty>,
    ) -> Result<(Function, Ty)> {
        self.in_method = in_method;
        for (name, ty) in params {
            self.declare_slot(name, ty.clone());
        }
        let ty = self.expr(expr, expected)?;
        let ty = match expected {
            Some(expected) => self.coerce(&ty, expected)?,
            None => ty,
        };
        self.emit(Instr::Ret);
        Ok((
            Function {
                name: String::new(),
                code: self.code,
                params: u32::try_from(params.len()).unwrap_or(u32::MAX),
                locals: self.slots,
                outs: Vec::new(),
                is_async: false,
            },
            ty,
        ))
    }

    fn emit_outs(&mut self) {
        for slot in self.outs.clone() {
            self.emit(Instr::LoadLocal(slot));
        }
    }

    // Statements

    fn block(&mut self, body: &[Stmt]) -> Result<()> {
        self.push_scope();
        for (index, stmt) in body.iter().enumerate() {
            self.with_path(&format!("[{index}]"), |c| c.stmt(stmt))?;
        }
        self.pop_scope();
        Ok(())
    }

    fn nested(&mut self, suffix: &str, body: &[Stmt]) -> Result<()> {
        self.with_path(suffix, |c| c.block(body))
    }

    fn loop_body(
        &mut self,
        label: &Option<String>,
        body: &[Stmt],
        continue_target: Option<u32>,
    ) -> Result<LoopCx> {
        self.loops.push(LoopCx {
            label: label.clone(),
            breaks: Vec::new(),
            continues: Vec::new(),
        });
        let result = self.nested(".body", body);
        let mut cx = self.loops.pop().unwrap_or(LoopCx {
            label: None,
            breaks: Vec::new(),
            continues: Vec::new(),
        });
        result?;
        if let Some(target) = continue_target {
            for at in cx.continues.drain(..) {
                self.patch(at, target);
            }
        }
        Ok(cx)
    }

    fn finish_loop(&mut self, cx: LoopCx) {
        let end = self.here();
        for at in cx.breaks {
            self.patch(at, end);
        }
        for at in cx.continues {
            self.patch(at, end);
        }
    }

    fn stmt(&mut self, stmt: &Stmt) -> Result<()> {
        match stmt {
            Stmt::Let {
                name,
                ty,
                value,
                mutable: _,
            } => {
                let declared = ty.as_ref().map(Ty::from_ir);
                let vty = self.expr(value, declared.as_ref())?;
                let ty = match declared {
                    Some(declared) => self.coerce(&vty, &declared)?,
                    None => {
                        if vty == Ty::IntLit {
                            return self.err(format!("the width of {name} is unknown"));
                        }
                        vty
                    }
                };
                let slot = self.declare_slot(name, ty);
                self.emit(Instr::StoreLocal(slot));
            }
            Stmt::LetRef { name, place } => {
                let Some((place, ty)) = self.place_of(place, false)? else {
                    return self.err("a reference borrows a field of self");
                };
                self.declare(name, Binding::Alias(place, ty));
            }
            Stmt::Assign { target, op, value } => {
                let Some((place, pty)) = self.place_of(target, true)? else {
                    return self.err("the target of an assignment is a local, a field or an item");
                };
                match op {
                    None => {
                        let vty = self.expr(value, Some(&pty))?;
                        self.coerce(&vty, &pty)?;
                        self.emit(Instr::StorePath(place));
                    }
                    Some(op) => {
                        if place.dynamic_indices() > 0 {
                            return self
                                .err("a compound assignment to an item needs a constant index");
                        }
                        self.emit(Instr::LoadPath(place.clone()));
                        let vty = self.expr(value, Some(&pty))?;
                        self.binary_in(*op, &pty, &vty, None)?;
                        self.emit(Instr::StorePath(place));
                    }
                }
            }
            Stmt::If {
                cond,
                then,
                otherwise,
            } => {
                let cty = self.expr(cond, Some(&Ty::Bool))?;
                self.coerce(&cty, &Ty::Bool)?;
                let to_else = self.emit(Instr::JumpIfFalse(0));
                self.nested(".then", then)?;
                if otherwise.is_empty() {
                    self.patch_here(to_else);
                } else {
                    let to_end = self.emit(Instr::Jump(0));
                    self.patch_here(to_else);
                    self.nested(".else", otherwise)?;
                    self.patch_here(to_end);
                }
            }
            Stmt::IfSome {
                value,
                bind,
                borrow,
                then,
                otherwise,
            } => {
                let (inner, alias) = self.option_subject(value, *borrow)?;
                let to_else = self.emit(Instr::JumpIfTrue(0));
                self.push_scope();
                match alias {
                    Some(place) => self.declare(bind, Binding::Alias(place, inner)),
                    None => {
                        let slot = self.declare_slot(bind, inner);
                        self.emit(Instr::StoreLocal(slot));
                    }
                }
                let result = self.nested(".then", then);
                self.pop_scope();
                result?;
                let to_end = self.emit(Instr::Jump(0));
                self.patch_here(to_else);
                if !*borrow {
                    self.emit(Instr::Pop);
                }
                self.nested(".else", otherwise)?;
                self.patch_here(to_end);
            }
            Stmt::LetSome {
                bind,
                value,
                borrow,
                otherwise,
            } => {
                let (inner, alias) = self.option_subject(value, *borrow)?;
                let to_some = self.emit(Instr::JumpIfFalse(0));
                if !*borrow {
                    self.emit(Instr::Pop);
                }
                self.nested(".else", otherwise)?;
                self.patch_here(to_some);
                match alias {
                    Some(place) => self.declare(bind, Binding::Alias(place, inner)),
                    None => {
                        let slot = self.declare_slot(bind, inner);
                        self.emit(Instr::StoreLocal(slot));
                    }
                }
            }
            Stmt::Match { value, arms } => {
                let vty = self.expr(value, None)?;
                let subject = self.slot();
                self.emit(Instr::StoreLocal(subject));
                let mut to_end = Vec::new();
                for (index, arm) in arms.iter().enumerate() {
                    self.push_scope();
                    let next = match &arm.pattern {
                        Pattern::Variant { name, binds } => {
                            let (tag, fields) = self.variant_of(&vty, name)?;
                            self.emit(Instr::LoadLocal(subject));
                            self.emit(Instr::IsVariant(tag));
                            let next = self.emit(Instr::JumpIfFalse(0));
                            let pairs = self.bind_fields(&fields, binds, name)?;
                            if !pairs.is_empty() {
                                self.emit(Instr::LoadLocal(subject));
                                self.emit(Instr::Destructure(pairs));
                            }
                            Some(next)
                        }
                        Pattern::Values { values } => {
                            let consts = values
                                .iter()
                                .map(|value| self.pattern_const(value, &vty))
                                .collect::<Result<Vec<_>>>()?;
                            self.emit(Instr::LoadLocal(subject));
                            self.emit(Instr::IsIn(consts));
                            Some(self.emit(Instr::JumpIfFalse(0)))
                        }
                        Pattern::Default => None,
                    };
                    let result = self.nested(&format!(".arms[{index}].body"), &arm.body);
                    self.pop_scope();
                    result?;
                    to_end.push(self.emit(Instr::Jump(0)));
                    if let Some(next) = next {
                        self.patch_here(next);
                    }
                }
                for at in to_end {
                    self.patch_here(at);
                }
            }
            Stmt::Loop { label, body } => {
                let start = self.here();
                let cx = self.loop_body(label, body, Some(start))?;
                self.emit(Instr::Jump(start));
                self.finish_loop(cx);
            }
            Stmt::While { label, cond, body } => {
                let start = self.here();
                let cty = self.expr(cond, Some(&Ty::Bool))?;
                self.coerce(&cty, &Ty::Bool)?;
                let to_end = self.emit(Instr::JumpIfFalse(0));
                let cx = self.loop_body(label, body, Some(start))?;
                self.emit(Instr::Jump(start));
                self.patch_here(to_end);
                self.finish_loop(cx);
            }
            Stmt::For {
                label,
                var,
                from,
                to,
                body,
            } => {
                let fty = self.expr(from, None)?;
                let var_slot = self.slot();
                self.emit(Instr::StoreLocal(var_slot));
                let tty = self.expr(to, None)?;
                let end_slot = self.slot();
                self.emit(Instr::StoreLocal(end_slot));
                let ty = common_int(&fty, &tty).unwrap_or(Ty::U64);
                let ty = if ty == Ty::IntLit { Ty::U64 } else { ty };
                self.coerce(&fty, &ty)?;
                self.coerce(&tty, &ty)?;
                let width = ty.width().unwrap_or(Width::W64);
                self.push_scope();
                self.declare(var, Binding::Slot(var_slot, ty));
                let start = self.here();
                self.emit(Instr::LoadLocal(var_slot));
                self.emit(Instr::LoadLocal(end_slot));
                self.emit(Instr::Compare(CmpOp::Lt));
                let to_end = self.emit(Instr::JumpIfFalse(0));
                let cx = self.loop_body(label, body, None);
                self.pop_scope();
                let mut cx = cx?;
                let step = self.here();
                for at in cx.continues.drain(..) {
                    self.patch(at, step);
                }
                self.emit(Instr::LoadLocal(var_slot));
                self.emit(Instr::Const(Const::Int(1)));
                self.emit(Instr::Binary {
                    op: ArithOp::Add,
                    width,
                });
                self.emit(Instr::StoreLocal(var_slot));
                self.emit(Instr::Jump(start));
                self.patch_here(to_end);
                self.finish_loop(cx);
            }
            Stmt::ForEach {
                label,
                var,
                array,
                body,
            } => {
                let aty = self.expr(array, None)?;
                let item = match aty {
                    Ty::Array(item, _) => *item,
                    other => return self.err(format!("for-each over {}", other.describe())),
                };
                let array_slot = self.slot();
                self.emit(Instr::StoreLocal(array_slot));
                let index_slot = self.slot();
                self.emit(Instr::Const(Const::Int(0)));
                self.emit(Instr::StoreLocal(index_slot));
                self.push_scope();
                let var_slot = self.declare_slot(var, item);
                let start = self.here();
                let next = self.emit(Instr::ForEachNext {
                    array: array_slot,
                    index: index_slot,
                    var: var_slot,
                    end: 0,
                });
                let cx = self.loop_body(label, body, Some(start));
                self.pop_scope();
                let cx = cx?;
                self.emit(Instr::Jump(start));
                self.patch_here(next);
                self.finish_loop(cx);
            }
            Stmt::Break { label } => {
                let at = self.emit(Instr::Jump(0));
                let Some(cx) = self.loop_for(label) else {
                    return self.err("break outside a loop");
                };
                cx.breaks.push(at);
            }
            Stmt::Continue { label } => {
                let at = self.emit(Instr::Jump(0));
                let Some(cx) = self.loop_for(label) else {
                    return self.err("continue outside a loop");
                };
                cx.continues.push(at);
            }
            Stmt::Return { value } => {
                match value {
                    Some(value) => {
                        let ret = self.ret.clone();
                        let vty = self.expr(value, Some(&ret))?;
                        self.coerce(&vty, &ret)?;
                    }
                    None => {
                        self.emit(Instr::Const(Const::Unit));
                    }
                }
                self.emit_outs();
                self.emit(Instr::Ret);
            }
            Stmt::Expr { expr } => {
                self.expr(expr, None)?;
                self.emit(Instr::Pop);
            }
        }
        Ok(())
    }

    fn loop_for(&mut self, label: &Option<String>) -> Option<&mut LoopCx> {
        match label {
            None => self.loops.last_mut(),
            Some(label) => self
                .loops
                .iter_mut()
                .rev()
                .find(|cx| cx.label.as_ref() == Some(label)),
        }
    }

    /// Emits the test of an option: with `borrow`, the place is read and the binding will
    /// alias it; without, the value is left on the stack under the test's answer. Returns the
    /// inner type and the place aliased.
    fn option_subject(&mut self, value: &Expr, borrow: bool) -> Result<(Ty, Option<Place>)> {
        if borrow {
            let Some((place, ty)) = self.place_of(value, false)? else {
                return self.err("a borrow binds a field of self");
            };
            let inner = self.option_inner(&ty)?;
            self.emit(Instr::LoadPath(place.clone()));
            self.emit(Instr::IsNone);
            Ok((inner, Some(place)))
        } else {
            let ty = self.expr(value, None)?;
            let inner = self.option_inner(&ty)?;
            self.emit(Instr::Dup);
            self.emit(Instr::IsNone);
            Ok((inner, None))
        }
    }

    fn option_inner(&self, ty: &Ty) -> Result<Ty> {
        match ty {
            Ty::Option(inner) => Ok((**inner).clone()),
            other => self.err(format!("{} is not an option", other.describe())),
        }
    }

    /// The variant `name` of the union `ty`: its tag and its fields.
    fn variant_of(&self, ty: &Ty, name: &str) -> Result<(u16, Vec<FieldDef>)> {
        match ty {
            Ty::Named(union) => match self.env.types.variant(union, name) {
                Some((tag, def)) => Ok((tag, def.fields.clone())),
                None => self.err(format!("{union} has no variant {name}")),
            },
            Ty::Timed => timed_variant(name).ok_or_else(|| {
                unsupported(
                    &self.path,
                    format!("an edge or quiet has no variant {name}"),
                )
            }),
            other => self.err(format!("{} has no variants", other.describe())),
        }
    }

    /// Declares the locals `binds` names for the fields of a variant; the field indices and
    /// slots, for a `Destructure`.
    fn bind_fields(
        &mut self,
        fields: &[FieldDef],
        binds: &std::collections::BTreeMap<String, String>,
        variant: &str,
    ) -> Result<Vec<(u32, u32)>> {
        let mut pairs = Vec::new();
        for (field, local) in binds {
            let Some(index) = fields.iter().position(|f| &f.name == field) else {
                return self.err(format!("{variant} has no field {field}"));
            };
            let ty = fields[index].ty.clone();
            let slot = self.declare_slot(local, ty);
            pairs.push((u32::try_from(index).unwrap_or(u32::MAX), slot));
        }
        Ok(pairs)
    }

    fn pattern_const(&self, value: &Literal, ty: &Ty) -> Result<Const> {
        match (value, ty) {
            (Literal::Text(name), Ty::Named(enum_name)) => {
                match self.env.types.enum_variant(enum_name, name) {
                    Some(index) => Ok(Const::Enum(index)),
                    None => self.err(format!("{enum_name} has no variant {name}")),
                }
            }
            (Literal::Int(n), _) => Ok(Const::Int(*n)),
            (Literal::Bool(b), _) => Ok(Const::Bool(*b)),
            (Literal::Text(text), other) => {
                self.err(format!("{text:?} is not a pattern of {}", other.describe()))
            }
        }
    }

    // Places

    /// The place `expr` names, if it is one: a local, a field of self, or a field or item of
    /// those. With `dynamic`, an item at a computed index emits the index and takes it from
    /// the stack; without, such an expression is no place.
    fn place_of(&mut self, expr: &Expr, dynamic: bool) -> Result<Option<(Place, Ty)>> {
        match expr {
            Expr::Local { name } => match self.lookup(name).cloned() {
                Some(Binding::Slot(slot, ty)) => Ok(Some((Place::local(slot), ty))),
                Some(Binding::Alias(place, ty)) => Ok(Some((place, ty))),
                None => Ok(None),
            },
            Expr::SelfField { field } => {
                if !self.in_method {
                    return self.err("self is only in scope in a method");
                }
                match self.env.state.get(field) {
                    Some((index, ty)) => Ok(Some((Place::field(*index), ty.clone()))),
                    None => self.err(format!("the state has no field {field}")),
                }
            }
            Expr::Field { of, name } => {
                let Some((mut place, ty)) = self.place_of(of, dynamic)? else {
                    return Ok(None);
                };
                let (index, fty) = self.field_of(&ty, name)?;
                place.steps.push(Step::Field(index));
                Ok(Some((place, fty)))
            }
            Expr::Index { of, at } => {
                let Some((mut place, ty)) = self.place_of(of, dynamic)? else {
                    return Ok(None);
                };
                let item = match ty {
                    Ty::Array(item, _) => *item,
                    other => return self.err(format!("index into {}", other.describe())),
                };
                match &**at {
                    Expr::Lit {
                        value: Literal::Int(n),
                    } => place
                        .steps
                        .push(Step::At(u32::try_from(*n).unwrap_or(u32::MAX))),
                    _ if dynamic => {
                        let aty = self.expr(at, Some(&Ty::U64))?;
                        self.coerce(&aty, &Ty::U64)?;
                        place.steps.push(Step::Index);
                    }
                    _ => return Ok(None),
                }
                Ok(Some((place, item)))
            }
            _ => Ok(None),
        }
    }

    /// The field `name` of a value of `ty`: its index and type.
    fn field_of(&self, ty: &Ty, name: &str) -> Result<(u32, Ty)> {
        if let Some(found) = builtin_field(ty, name) {
            return Ok(found);
        }
        match ty {
            Ty::Named(record) => {
                let Some((_, def)) = self.env.types.record(record) else {
                    return self.err(format!(
                        "{record} is not a record; read its kind in a match"
                    ));
                };
                match def.fields.iter().position(|field| field.name == name) {
                    Some(index) => Ok((
                        u32::try_from(index).unwrap_or(u32::MAX),
                        def.fields[index].ty.clone(),
                    )),
                    None => self.err(format!("{record} has no field {name}")),
                }
            }
            Ty::Timed => self.err("an edge or quiet is read through a match"),
            other => self.err(format!("{} has no field {name}", other.describe())),
        }
    }

    // Expressions

    /// Emits `expr`, leaving its value on the stack; its type.
    fn expr(&mut self, expr: &Expr, expected: Option<&Ty>) -> Result<Ty> {
        match expr {
            Expr::Lit {
                value: Literal::Int(n),
            } => {
                self.emit(Instr::Const(Const::Int(*n)));
                Ok(Ty::IntLit)
            }
            Expr::Lit {
                value: Literal::Bool(b),
            } => {
                self.emit(Instr::Const(Const::Bool(*b)));
                Ok(Ty::Bool)
            }
            Expr::Lit {
                value: Literal::Text(text),
            }
            | Expr::Text { value: text } => {
                self.emit(Instr::Const(Const::Text(text.as_str().into())));
                Ok(Ty::Text)
            }
            Expr::Format { parts } => {
                for part in parts {
                    if let Expr::Text { value } = part {
                        self.emit(Instr::Const(Const::Text(value.as_str().into())));
                        continue;
                    }
                    let ty = self.expr(part, None)?;
                    if let Ty::Named(name) = &ty
                        && let Some((index, _)) = self.env.types.enumeration(name)
                    {
                        self.emit(Instr::EnumName(u32::try_from(index).unwrap_or(u32::MAX)));
                    }
                }
                self.emit(Instr::Format(
                    u32::try_from(parts.len()).unwrap_or(u32::MAX),
                ));
                Ok(Ty::Text)
            }
            Expr::None => {
                self.emit(Instr::Const(Const::None));
                Ok(expected.cloned().unwrap_or(Ty::Option(Box::new(Ty::Unit))))
            }
            Expr::Local { name } => {
                if let Some(binding) = self.lookup(name).cloned() {
                    return match binding {
                        Binding::Slot(slot, ty) => {
                            self.emit(Instr::LoadLocal(slot));
                            Ok(ty)
                        }
                        Binding::Alias(place, ty) => {
                            self.emit(Instr::LoadPath(place));
                            Ok(ty)
                        }
                    };
                }
                if let Some((value, ty)) = self.env.consts.get(name).cloned() {
                    self.emit(Instr::Const(value));
                    return Ok(ty);
                }
                self.err(format!("{name} is not in scope"))
            }
            Expr::SelfField { .. } | Expr::Field { .. } | Expr::Index { .. } => {
                if let Some((place, ty)) = self.place_of(expr, true)? {
                    self.emit(Instr::LoadPath(place));
                    return Ok(ty);
                }
                match expr {
                    Expr::Field { of, name } => {
                        let of_ty = self.expr(of, None)?;
                        let (index, ty) = self.field_of(&of_ty, name)?;
                        self.emit(Instr::GetField(index));
                        Ok(ty)
                    }
                    Expr::Index { of, at } => {
                        let of_ty = self.expr(of, None)?;
                        let item = match of_ty {
                            Ty::Array(item, _) => *item,
                            other => return self.err(format!("index into {}", other.describe())),
                        };
                        let aty = self.expr(at, Some(&Ty::U64))?;
                        self.coerce(&aty, &Ty::U64)?;
                        self.emit(Instr::Index);
                        Ok(item)
                    }
                    _ => self.err("self is only in scope in a method"),
                }
            }
            Expr::Tag { .. } => self.err("a tag is read through a match"),
            Expr::Unary { op, value } => {
                let ty = self.expr(value, None)?;
                match op {
                    UnaryOp::Not => {
                        self.coerce(&ty, &Ty::Bool)?;
                        self.emit(Instr::Not);
                        Ok(Ty::Bool)
                    }
                    UnaryOp::BitNot => {
                        if !ty.is_int() {
                            return self.err(format!("bitwise not of {}", ty.describe()));
                        }
                        let ty = if ty == Ty::IntLit { Ty::U64 } else { ty };
                        self.emit(Instr::BitNot(ty.width().unwrap_or(Width::W64)));
                        Ok(ty)
                    }
                }
            }
            Expr::Binary { op, left, right } => match op {
                BinaryOp::And | BinaryOp::Or => {
                    let lty = self.expr(left, Some(&Ty::Bool))?;
                    self.coerce(&lty, &Ty::Bool)?;
                    self.emit(Instr::Dup);
                    let short = self.emit(if *op == BinaryOp::And {
                        Instr::JumpIfFalse(0)
                    } else {
                        Instr::JumpIfTrue(0)
                    });
                    self.emit(Instr::Pop);
                    let rty = self.expr(right, Some(&Ty::Bool))?;
                    self.coerce(&rty, &Ty::Bool)?;
                    self.patch_here(short);
                    Ok(Ty::Bool)
                }
                _ => {
                    // Arithmetic is done at the width of its context, when that is wider than
                    // its operands: what the result is assigned to, passed as or returned as.
                    let context = expected
                        .filter(|ty| ty.is_int() && **ty != Ty::IntLit)
                        .cloned();
                    let lty = self.expr(left, context.as_ref())?;
                    let rty = self.expr(right, context.as_ref().or(Some(&lty)))?;
                    self.binary_in(*op, &lty, &rty, context.as_ref())
                }
            },
            Expr::Cast { value, to } => {
                let to = Ty::from_ir(to);
                let Some(width) = to.width() else {
                    return self.err(format!("a cast to {}", to.describe()));
                };
                let ty = self.expr(value, None)?;
                if !ty.is_int() {
                    return self.err(format!("a cast of {}", ty.describe()));
                }
                self.emit(Instr::Narrow(width));
                Ok(to)
            }
            Expr::Record { ty, fields } => {
                let Some((_, def)) = self.env.types.record(ty) else {
                    return self.err(format!("{ty} is not a record"));
                };
                let defs = def.fields.clone();
                for field in &defs {
                    let Some(value) = fields.get(&field.name) else {
                        return self.err(format!("{ty} needs {}", field.name));
                    };
                    let vty = self.expr(value, Some(&field.ty))?;
                    self.coerce(&vty, &field.ty)?;
                }
                self.emit(Instr::MakeRecord(
                    u32::try_from(defs.len()).unwrap_or(u32::MAX),
                ));
                Ok(Ty::Named(ty.clone()))
            }
            Expr::Variant { ty, kind, fields } => {
                let Some((tag, def)) = self.env.types.variant(ty, kind) else {
                    return self.err(format!("{ty} has no variant {kind}"));
                };
                let defs = def.fields.clone();
                for field in &defs {
                    let Some(value) = fields.get(&field.name) else {
                        return self.err(format!("{ty}::{kind} needs {}", field.name));
                    };
                    let vty = self.expr(value, Some(&field.ty))?;
                    self.coerce(&vty, &field.ty)?;
                }
                self.emit(Instr::MakeVariant {
                    tag,
                    fields: u32::try_from(defs.len()).unwrap_or(u32::MAX),
                });
                Ok(Ty::Named(ty.clone()))
            }
            Expr::Enum { ty, value } => match self.env.types.enum_variant(ty, value) {
                Some(index) => {
                    self.emit(Instr::Const(Const::Enum(index)));
                    Ok(Ty::Named(ty.clone()))
                }
                None => self.err(format!("{ty} has no variant {value}")),
            },
            Expr::Array { items } => {
                let want = match expected {
                    Some(Ty::Array(item, _)) => Some((**item).clone()),
                    _ => None,
                };
                let mut types = Vec::new();
                for item in items {
                    types.push(self.expr(item, want.as_ref())?);
                }
                // The items meet at one type: an option when any of them is one.
                let item_ty = want.unwrap_or_else(|| {
                    types
                        .iter()
                        .cloned()
                        .reduce(|a, b| join_types(&a, &b))
                        .unwrap_or(Ty::U64)
                });
                let item_ty = if item_ty == Ty::IntLit {
                    Ty::U64
                } else {
                    item_ty
                };
                for ty in &types {
                    self.coerce(ty, &item_ty)?;
                }
                let bound = u32::try_from(items.len()).unwrap_or(u32::MAX);
                self.emit(Instr::MakeArray(bound));
                Ok(Ty::Array(Box::new(item_ty), bound))
            }
            Expr::Call { name, args } => {
                let Some(sig) = self.env.functions.get(name).cloned() else {
                    return self.err(format!("{name} is not a function"));
                };
                self.call(&sig, args)
            }
            Expr::Method { name, args } => {
                if !self.in_method {
                    return self.err("a method is called on self, in a method");
                }
                let Some(sig) = self.env.methods.get(name).cloned() else {
                    return self.err(format!("{name} is not a method"));
                };
                if sig.is_async {
                    return self.err(format!("{name} is async: await it"));
                }
                self.call(&sig, args)
            }
            Expr::Intrinsic { name, args } => self.intrinsic(name, args, expected),
            Expr::Ternary {
                cond,
                then,
                otherwise,
            } => {
                let cty = self.expr(cond, Some(&Ty::Bool))?;
                self.coerce(&cty, &Ty::Bool)?;
                let to_else = self.emit(Instr::JumpIfFalse(0));
                let tty = self.expr(then, expected)?;
                let to_end = self.emit(Instr::Jump(0));
                self.patch_here(to_else);
                let oty = self.expr(otherwise, expected.or(Some(&tty)))?;
                self.patch_here(to_end);
                let ty = expected.cloned().unwrap_or_else(|| join_types(&tty, &oty));
                self.coerce(&tty, &ty)?;
                self.coerce(&oty, &ty)?;
                Ok(ty)
            }
            Expr::IfSome {
                value,
                bind,
                some,
                none,
            } => {
                let vty = self.expr(value, None)?;
                let inner = self.option_inner(&vty)?;
                self.emit(Instr::Dup);
                self.emit(Instr::IsNone);
                let to_none = self.emit(Instr::JumpIfTrue(0));
                self.push_scope();
                let slot = self.declare_slot(bind, inner);
                self.emit(Instr::StoreLocal(slot));
                let sty = self.expr(some, expected);
                self.pop_scope();
                let sty = sty?;
                let to_end = self.emit(Instr::Jump(0));
                self.patch_here(to_none);
                self.emit(Instr::Pop);
                let nty = self.expr(none, expected.or(Some(&sty)))?;
                self.patch_here(to_end);
                let ty = expected.cloned().unwrap_or_else(|| join_types(&sty, &nty));
                self.coerce(&sty, &ty)?;
                self.coerce(&nty, &ty)?;
                Ok(ty)
            }
            Expr::IfVariant {
                value,
                variant,
                binds,
                then,
                otherwise,
            } => {
                let vty = self.expr(value, None)?;
                let (tag, fields) = self.variant_of(&vty, variant)?;
                self.emit(Instr::Dup);
                self.emit(Instr::IsVariant(tag));
                let to_else = self.emit(Instr::JumpIfFalse(0));
                self.push_scope();
                let pairs = self.bind_fields(&fields, binds, variant);
                let tty = pairs.and_then(|pairs| {
                    self.emit(Instr::Destructure(pairs));
                    self.expr(then, expected)
                });
                self.pop_scope();
                let tty = tty?;
                let to_end = self.emit(Instr::Jump(0));
                self.patch_here(to_else);
                self.emit(Instr::Pop);
                let oty = self.expr(otherwise, expected.or(Some(&tty)))?;
                self.patch_here(to_end);
                let ty = expected.cloned().unwrap_or_else(|| join_types(&tty, &oty));
                self.coerce(&tty, &ty)?;
                self.coerce(&oty, &ty)?;
                Ok(ty)
            }
            Expr::Required { value } => {
                if self.env.kind != Kind::Bus {
                    return self.err("a required value (`!`) is for bus decoders");
                }
                let vty = self.expr(value, None)?;
                let inner = self.option_inner(&vty)?;
                self.emit(Instr::Required);
                Ok(inner)
            }
            Expr::Await { op } => self.operation(op),
        }
    }

    /// Emits a call: the arguments, the call, and the final values of the mutable record
    /// parameters stored back where they came from.
    fn call(&mut self, sig: &Sig, args: &[Expr]) -> Result<Ty> {
        if args.len() != sig.params.len() {
            return self.err("wrong number of arguments");
        }
        for (arg, param) in args.iter().zip(&sig.params) {
            let aty = self.expr(arg, Some(&param.ty))?;
            if param.ty != Ty::Cursor {
                self.coerce(&aty, &param.ty)?;
            }
        }
        self.emit(Instr::Call(sig.id));
        for (arg, param) in args.iter().zip(&sig.params).rev() {
            if !param.mutable {
                continue;
            }
            match self.place_of(arg, false)? {
                Some((place, _)) => {
                    self.emit(Instr::StorePath(place));
                }
                None => {
                    self.emit(Instr::Pop);
                }
            }
        }
        Ok(sig.ret.clone())
    }

    fn binary_in(
        &mut self,
        op: BinaryOp,
        left: &Ty,
        right: &Ty,
        context: Option<&Ty>,
    ) -> Result<Ty> {
        match op {
            BinaryOp::And | BinaryOp::Or => {
                self.err("a short-circuit operator in a compound assignment")
            }
            BinaryOp::Eq | BinaryOp::Ne => {
                self.align(left, right)?;
                self.emit(Instr::Compare(if op == BinaryOp::Eq {
                    CmpOp::Eq
                } else {
                    CmpOp::Ne
                }));
                Ok(Ty::Bool)
            }
            BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
                self.align(left, right)?;
                self.emit(Instr::Compare(match op {
                    BinaryOp::Lt => CmpOp::Lt,
                    BinaryOp::Le => CmpOp::Le,
                    BinaryOp::Gt => CmpOp::Gt,
                    _ => CmpOp::Ge,
                }));
                Ok(Ty::Bool)
            }
            BinaryOp::Shl | BinaryOp::Shr => {
                let operand = if left.is_int() && *left != Ty::IntLit {
                    Some(left.clone())
                } else {
                    None
                };
                let ty = width_in(operand, context).unwrap_or(Ty::U64);
                let ty = if ty == Ty::IntLit { Ty::U64 } else { ty };
                self.coerce(left, &ty)?;
                if !right.is_int() {
                    return self.err(format!("a shift by {}", right.describe()));
                }
                self.emit(Instr::Binary {
                    op: if op == BinaryOp::Shl {
                        ArithOp::Shl
                    } else {
                        ArithOp::Shr
                    },
                    width: ty.width().unwrap_or(Width::W64),
                });
                Ok(ty)
            }
            BinaryOp::Add
            | BinaryOp::Sub
            | BinaryOp::Mul
            | BinaryOp::Div
            | BinaryOp::Rem
            | BinaryOp::BitAnd
            | BinaryOp::BitOr
            | BinaryOp::BitXor => {
                let Some(operands) = common_int(left, right) else {
                    return self.err(format!(
                        "arithmetic on {} and {}",
                        left.describe(),
                        right.describe()
                    ));
                };
                let ty = width_in(Some(operands), context).unwrap_or(Ty::U64);
                let ty = if ty == Ty::IntLit { Ty::U64 } else { ty };
                self.coerce(left, &ty)?;
                self.coerce(right, &ty)?;
                self.emit(Instr::Binary {
                    op: match op {
                        BinaryOp::Add => ArithOp::Add,
                        BinaryOp::Sub => ArithOp::Sub,
                        BinaryOp::Mul => ArithOp::Mul,
                        BinaryOp::Div => ArithOp::Div,
                        BinaryOp::Rem => ArithOp::Rem,
                        BinaryOp::BitAnd => ArithOp::BitAnd,
                        BinaryOp::BitOr => ArithOp::BitOr,
                        _ => ArithOp::BitXor,
                    },
                    width: ty.width().unwrap_or(Width::W64),
                });
                Ok(ty)
            }
        }
    }

    /// Both sides of a comparison at one type.
    fn align(&self, left: &Ty, right: &Ty) -> Result<()> {
        if left.is_int() && right.is_int() {
            let ty = common_int(left, right).unwrap_or(Ty::U64);
            self.coerce(left, &ty)?;
            self.coerce(right, &ty)?;
            return Ok(());
        }
        let ty = match (left, right) {
            (Ty::Option(_), _) => left.clone(),
            (_, Ty::Option(_)) => right.clone(),
            (l, _) if *l != Ty::IntLit => l.clone(),
            (_, r) => r.clone(),
        };
        self.coerce(left, &ty)?;
        self.coerce(right, &ty)?;
        Ok(())
    }

    /// Checks that a value of `have` is a `want`: integers widen, values go into options as
    /// they are, nothing else changes — so no instruction is emitted, only the type checked.
    fn coerce(&self, have: &Ty, want: &Ty) -> Result<Ty> {
        if have == want {
            return Ok(want.clone());
        }
        match (have, want) {
            (Ty::IntLit, w) if w.is_int() => Ok(w.clone()),
            (Ty::IntLit, Ty::Option(inner)) if inner.is_int() => Ok(want.clone()),
            (h, w) if h.is_int() && w.is_int() => {
                let (hw, ww) = (
                    h.width().unwrap_or(Width::W64),
                    w.width().unwrap_or(Width::W64),
                );
                if hw.bits() <= ww.bits() {
                    Ok(w.clone())
                } else {
                    self.err(format!(
                        "a {} does not fit a {}",
                        h.describe(),
                        w.describe()
                    ))
                }
            }
            (Ty::Option(inner), Ty::Option(wanted)) => {
                if **inner == Ty::Unit || (inner.is_int() && wanted.is_int()) {
                    return Ok(want.clone());
                }
                Ok(want.clone())
            }
            (h, Ty::Option(inner))
                if h == &**inner || (h.is_int() && inner.is_int()) || *h == Ty::Unit =>
            {
                self.coerce(h, inner)?;
                Ok(want.clone())
            }
            (Ty::Array(item, _), Ty::Array(wanted, _)) if item == wanted => Ok(want.clone()),
            (Ty::Named(a), Ty::Named(b)) if a == b => Ok(want.clone()),
            (h, Ty::Unit) => Ok(h.clone()),
            (h, w) => self.err(format!("a {} is not a {}", h.describe(), w.describe())),
        }
    }

    fn arg(&mut self, name: &str, args: &[Expr], index: usize, want: Option<&Ty>) -> Result<Ty> {
        let Some(expr) = args.get(index) else {
            return self.err(format!("{name} needs argument {index}"));
        };
        let ty = self.expr(expr, want)?;
        match want {
            Some(want) => self.coerce(&ty, want),
            None => Ok(ty),
        }
    }

    fn intrinsic(&mut self, name: &str, args: &[Expr], expected: Option<&Ty>) -> Result<Ty> {
        match name {
            "min" | "max" => {
                let a = self.arg(name, args, 0, None)?;
                let b = self.arg(name, args, 1, Some(&a))?;
                let ty = common_int(&a, &b).unwrap_or(Ty::U64);
                self.coerce(&a, &ty)?;
                self.coerce(&b, &ty)?;
                self.emit(Instr::Intrinsic(if name == "min" {
                    Intrinsic::Min
                } else {
                    Intrinsic::Max
                }));
                Ok(ty)
            }
            "saturating-sub" => {
                let a = self.arg(name, args, 0, None)?;
                let b = self.arg(name, args, 1, Some(&a))?;
                let ty = common_int(&a, &b).unwrap_or(Ty::U64);
                self.coerce(&a, &ty)?;
                self.coerce(&b, &ty)?;
                self.emit(Instr::Intrinsic(Intrinsic::SaturatingSub));
                Ok(ty)
            }
            "count-ones" => {
                self.arg(name, args, 0, Some(&Ty::U64))?;
                self.emit(Instr::Intrinsic(Intrinsic::CountOnes));
                Ok(Ty::U8)
            }
            "from-bool" => {
                self.arg(name, args, 0, Some(&Ty::Bool))?;
                self.emit(Instr::Intrinsic(Intrinsic::FromBool));
                Ok(Ty::U8)
            }
            "mul-div" | "mul-div-ceil" => {
                for index in 0..3 {
                    self.arg(name, args, index, Some(&Ty::U64))?;
                }
                self.emit(Instr::Intrinsic(if name == "mul-div" {
                    Intrinsic::MulDiv
                } else {
                    Intrinsic::MulDivCeil
                }));
                Ok(Ty::U64)
            }
            "at" | "one" => {
                self.arg(name, args, 0, Some(&Ty::U64))?;
                self.emit(Instr::Intrinsic(if name == "at" {
                    Intrinsic::At
                } else {
                    Intrinsic::One
                }));
                Ok(Ty::Provenance)
            }
            "over" => {
                for index in 0..3 {
                    self.arg(name, args, index, Some(&Ty::U64))?;
                }
                self.emit(Instr::Intrinsic(Intrinsic::Over));
                Ok(Ty::Provenance)
            }
            "join" => {
                self.arg(name, args, 0, Some(&Ty::Provenance))?;
                self.arg(name, args, 1, Some(&Ty::Provenance))?;
                self.emit(Instr::Intrinsic(Intrinsic::Join));
                Ok(Ty::Provenance)
            }
            "mask" => {
                let option = Ty::Option(Box::new(Ty::U8));
                for index in 0..args.len() {
                    self.arg(name, args, index, Some(&option))?;
                }
                self.emit(Instr::Intrinsic(Intrinsic::Mask(
                    u32::try_from(args.len()).unwrap_or(u32::MAX),
                )));
                Ok(Ty::Mask)
            }
            "mask-has" | "mask-with" => {
                self.arg(name, args, 0, Some(&Ty::Mask))?;
                self.arg(name, args, 1, Some(&Ty::U8))?;
                if name == "mask-has" {
                    self.emit(Instr::Intrinsic(Intrinsic::MaskHas));
                    Ok(Ty::Bool)
                } else {
                    self.emit(Instr::Intrinsic(Intrinsic::MaskWith));
                    Ok(Ty::Mask)
                }
            }
            "levels-high" => {
                self.arg(name, args, 0, Some(&Ty::Levels))?;
                self.arg(name, args, 1, Some(&Ty::U8))?;
                self.emit(Instr::Intrinsic(Intrinsic::LevelsHigh));
                Ok(Ty::Bool)
            }
            "max-channels" => {
                self.emit(Instr::Const(Const::Int(u64::from(
                    uiharu_core::MAX_CHANNELS,
                ))));
                Ok(Ty::U8)
            }
            "position" => {
                self.needs_io(name)?;
                self.emit(Instr::Intrinsic(Intrinsic::Position));
                Ok(Ty::U64)
            }
            "seek" => {
                self.needs_io(name)?;
                self.arg(name, args, 0, Some(&Ty::U64))?;
                self.emit(Instr::Intrinsic(Intrinsic::Seek));
                Ok(Ty::Unit)
            }
            "batch" => {
                let Some(Ty::Batch(_, bound)) = expected else {
                    return self.err("a batch is a field with its capacity");
                };
                self.emit(Instr::NewBatch(*bound));
                Ok(expected.cloned().unwrap_or(Ty::Unit))
            }
            "batch-length" | "batch-capacity" | "batch-is-empty" | "batch-is-full" | "batch-at" => {
                let ty = self.arg(name, args, 0, None)?;
                let Ty::Batch(..) = ty else {
                    return self.err(format!("{name} on {}", ty.describe()));
                };
                let (query, ty) = match name {
                    "batch-length" => (BatchQuery::Length, Ty::U32),
                    "batch-capacity" => (BatchQuery::Capacity, Ty::U32),
                    "batch-is-empty" => (BatchQuery::IsEmpty, Ty::Bool),
                    "batch-is-full" => (BatchQuery::IsFull, Ty::Bool),
                    _ => (BatchQuery::At, Ty::Option(Box::new(Ty::Provenance))),
                };
                self.emit(Instr::BatchQuery(query));
                Ok(ty)
            }
            "batch-take" | "batch-clear" | "batch-push" => {
                let Some(batch) = args.first() else {
                    return self.err(format!("{name} needs the batch"));
                };
                let Some((place, ty)) = self.place_of(batch, false)? else {
                    return self.err("a batch is a field of the decoder");
                };
                let item = match ty {
                    Ty::Batch(item, _) => *item,
                    other => return self.err(format!("{name} on {}", other.describe())),
                };
                self.emit(Instr::TakePath(place.clone()));
                match name {
                    "batch-take" => {
                        self.emit(Instr::BatchTake);
                        self.emit(Instr::Swap);
                        self.emit(Instr::StorePath(place));
                        Ok(Ty::Taken(Box::new(item)))
                    }
                    "batch-clear" => {
                        self.emit(Instr::BatchClear);
                        self.emit(Instr::StorePath(place));
                        self.emit(Instr::Const(Const::Unit));
                        Ok(Ty::Unit)
                    }
                    _ => {
                        self.arg(name, args, 1, Some(&item))?;
                        self.arg(name, args, 2, Some(&Ty::Provenance))?;
                        self.emit(Instr::BatchPush);
                        self.emit(Instr::StorePath(place));
                        self.emit(Instr::Const(Const::Unit));
                        Ok(Ty::Unit)
                    }
                }
            }
            "consumed" => {
                if self.env.kind != Kind::Bus {
                    return self.err("consumed is for bus decoders");
                }
                self.needs_io(name)?;
                self.emit(Instr::Intrinsic(Intrinsic::Consumed));
                Ok(Ty::Provenance)
            }
            "tx-id" | "tx-partial" => {
                let ty = self.arg(name, args, 0, None)?;
                if ty != Ty::Cursor {
                    return self.err(format!("{name} takes the transaction cursor"));
                }
                if name == "tx-id" {
                    self.emit(Instr::Intrinsic(Intrinsic::TxId));
                    Ok(Ty::U64)
                } else {
                    self.emit(Instr::Intrinsic(Intrinsic::TxPartial));
                    Ok(Ty::Bool)
                }
            }
            "unwrap-or" => {
                let a = self.arg(name, args, 0, None)?;
                let inner = self.option_inner(&a)?;
                self.arg(name, args, 1, Some(&inner))?;
                self.emit(Instr::Intrinsic(Intrinsic::UnwrapOr));
                Ok(inner)
            }
            other => self.err(format!("{other} is not an intrinsic")),
        }
    }

    /// Checks that the runtime's handles are in reach: in an async method, which the main
    /// loop gives them to.
    fn needs_io(&self, what: &str) -> Result<()> {
        if !self.is_async {
            return self.err(format!("{what} is used in an async method"));
        }
        Ok(())
    }

    fn operation(&mut self, op: &IrOp) -> Result<Ty> {
        if !self.is_async {
            return self.err("an operation is awaited in an async method");
        }
        match op {
            IrOp::NextChange { mask } => {
                let mty = self.expr(mask, Some(&Ty::Mask))?;
                self.coerce(&mty, &Ty::Mask)?;
                self.emit(Instr::Await(Op::NextChange));
                Ok(Ty::Edge)
            }
            IrOp::NextChangeBefore { mask, until } => {
                let mty = self.expr(mask, Some(&Ty::Mask))?;
                self.coerce(&mty, &Ty::Mask)?;
                let uty = self.expr(until, Some(&Ty::U64))?;
                self.coerce(&uty, &Ty::U64)?;
                self.emit(Instr::Await(Op::NextChangeBefore));
                Ok(Ty::Timed)
            }
            IrOp::Sample => {
                self.emit(Instr::Await(Op::Sample));
                Ok(Ty::Levels)
            }
            IrOp::Next => {
                let Some(lower) = self.env.lower.clone() else {
                    return self.err("next needs a lower stream");
                };
                self.emit(Instr::Await(Op::Next));
                Ok(Ty::Received(lower))
            }
            IrOp::Item | IrOp::ItemOrEnd | IrOp::Drain => {
                if !self.has_cursor {
                    return self.err("a read of the frame needs the transaction cursor");
                }
                let Some(lower) = self.env.lower.clone() else {
                    return self.err("a frame cursor needs a lower contract");
                };
                let item = Ty::Named(item_type_name(&lower));
                match op {
                    IrOp::Item => {
                        self.emit(Instr::Await(Op::Item));
                        Ok(item)
                    }
                    IrOp::ItemOrEnd => {
                        self.emit(Instr::Await(Op::ItemOrEnd));
                        Ok(Ty::Option(Box::new(item)))
                    }
                    _ => {
                        self.emit(Instr::Await(Op::Drain));
                        Ok(Ty::Unit)
                    }
                }
            }
            IrOp::Emit { event, at } => {
                // The provenance is evaluated before the event, as the generated Rust
                // evaluates the arguments of `emit(at, event)`.
                match at {
                    Some(at) => {
                        let aty = self.expr(at, Some(&Ty::Provenance))?;
                        self.coerce(&aty, &Ty::Provenance)?;
                    }
                    None => {
                        if self.env.kind != Kind::Bus {
                            return self.err("an event without provenance is a bus decoder's");
                        }
                    }
                }
                let want = Ty::Named(self.env.event.clone());
                let ety = self.expr(event, Some(&want))?;
                self.coerce(&ety, &want)?;
                self.emit(Instr::Await(Op::Emit { at: at.is_some() }));
                Ok(Ty::Unit)
            }
            IrOp::Method { name, args } => {
                if !self.in_method {
                    return self.err("a method is called on self, in a method");
                }
                let Some(sig) = self.env.methods.get(name).cloned() else {
                    return self.err(format!("{name} is not a method"));
                };
                if !sig.is_async {
                    return self.err(format!("{name} is synchronous: call it without await"));
                }
                self.call(&sig, args)
            }
        }
    }
}

/// The width an operation is done at: its context when wider than its operands.
fn width_in(operands: Option<Ty>, context: Option<&Ty>) -> Option<Ty> {
    match (operands, context) {
        (Some(operands), Some(context)) => {
            let a = operands.width().map_or(0, Width::bits);
            let b = context.width().map_or(0, Width::bits);
            Some(if b > a { context.clone() } else { operands })
        }
        (Some(operands), None) => Some(operands),
        (None, Some(context)) if context.is_int() => Some(context.clone()),
        _ => None,
    }
}

/// The width two integers meet at.
fn common_int(a: &Ty, b: &Ty) -> Option<Ty> {
    match (a.width(), b.width()) {
        (Some(x), Some(y)) => Some(Ty::of_width(if x.bits() >= y.bits() { x } else { y })),
        (Some(x), None) if *b == Ty::IntLit => Some(Ty::of_width(x)),
        (None, Some(y)) if *a == Ty::IntLit => Some(Ty::of_width(y)),
        (None, None) if *a == Ty::IntLit && *b == Ty::IntLit => Some(Ty::IntLit),
        _ => None,
    }
}

/// The type two branches meet at.
fn join_types(a: &Ty, b: &Ty) -> Ty {
    if a == b {
        return a.clone();
    }
    if let Some(ty) = common_int(a, b) {
        return ty;
    }
    match (a, b) {
        (Ty::Option(inner), other) if **inner == Ty::Unit => Ty::Option(Box::new(other.clone())),
        (other, Ty::Option(inner)) if **inner == Ty::Unit => Ty::Option(Box::new(other.clone())),
        (Ty::Option(_), _) => a.clone(),
        (_, Ty::Option(_)) => b.clone(),
        _ => a.clone(),
    }
}

// The module

fn signature(id: FuncId, function: &IrFunction) -> Sig {
    Sig {
        id,
        params: function
            .params
            .iter()
            .map(|param| ParamSig {
                name: param.name.clone(),
                ty: Ty::from_ir(&param.ty),
                mutable: param.mutable,
            })
            .collect(),
        ret: function.ret.as_ref().map_or(Ty::Unit, Ty::from_ir),
        is_async: function.is_async,
    }
}

fn schema_id(schema: &uiharu_ir::SchemaRef) -> SchemaId {
    SchemaId::new(intern(&schema.name), schema.version)
}

/// Compiles a validated module.
pub(crate) fn load(module: &Module) -> Result<Program> {
    let decoder = module
        .decoder
        .as_ref()
        .ok_or_else(|| unsupported("decoder", "the module declares no decoder"))?;
    let types = TypeTable::from_module(module).map_err(|message| unsupported("types", message))?;
    let (kind, input_schema) = match &decoder.input {
        IrInput::Logic => (Kind::Logic, None),
        IrInput::Stream { schema } => (Kind::Stream, Some(schema)),
        IrInput::Bus { schema } => (Kind::Bus, Some(schema)),
    };
    let lower = match input_schema {
        Some(schema) => Some(
            module
                .imports
                .iter()
                .map(|import| &import.contract)
                .chain(module.contracts.iter())
                .find(|contract| &contract.schema == schema)
                .ok_or_else(|| {
                    unsupported("decoder.input", format!("{} is not imported", schema.name))
                })?,
        ),
        None => None,
    };
    let event = module
        .contracts
        .iter()
        .find(|contract| contract.schema == decoder.output.schema)
        .map(|contract| contract.event.clone())
        .ok_or_else(|| {
            unsupported(
                "decoder.output",
                "the output's contract is not declared here",
            )
        })?;
    if !types.is_data(&Ty::Named(event.clone())) {
        return Err(unsupported(
            "decoder.output",
            format!("{event} carries values that are not data"),
        ));
    }
    if types.record(&decoder.config.ty).is_none() {
        return Err(unsupported(
            "decoder.config",
            format!("{} is not a record", decoder.config.ty),
        ));
    }

    let mut functions: HashMap<String, Sig> = HashMap::new();
    let mut methods: HashMap<String, Sig> = HashMap::new();
    let mut next_id: FuncId = 0;
    for function in &module.functions {
        functions.insert(function.name.clone(), signature(next_id, function));
        next_id += 1;
    }
    for method in &decoder.methods {
        methods.insert(method.name.clone(), signature(next_id, method));
        next_id += 1;
    }
    let mut state: HashMap<String, (u32, Ty)> = HashMap::new();
    state.insert("config".into(), (0, Ty::Named(decoder.config.ty.clone())));
    state.insert("rate".into(), (1, Ty::U64));
    for (index, field) in decoder.state.iter().enumerate() {
        state.insert(
            field.name.clone(),
            (
                u32::try_from(index + 2).unwrap_or(u32::MAX),
                Ty::from_ir(&field.ty),
            ),
        );
    }
    let mut consts = HashMap::new();
    for constant in &module.consts {
        let ty = Ty::from_ir(&constant.ty);
        let value = match &constant.value {
            Literal::Bool(b) => Const::Bool(*b),
            Literal::Int(n) => Const::Int(*n),
            Literal::Text(text) => Const::Text(text.as_str().into()),
        };
        consts.insert(constant.name.clone(), (value, ty));
    }
    let env = Env {
        types: &types,
        consts,
        functions,
        methods,
        state,
        kind,
        lower: lower.map(|contract| contract.event.clone()),
        event: event.clone(),
    };

    let mut compiled: Vec<Function> = Vec::new();
    for (index, function) in module.functions.iter().enumerate() {
        let sig = &env.functions[&function.name];
        let path = format!("functions[{index}]");
        let mut code = Compiler::new(&env, &path).function(sig, false, &function.body)?;
        code.name = function.name.clone();
        compiled.push(code);
    }
    for (index, method) in decoder.methods.iter().enumerate() {
        let sig = &env.methods[&method.name];
        let path = format!("decoder.methods[{index}]");
        let mut code = Compiler::new(&env, &path).function(sig, true, &method.body)?;
        code.name = method.name.clone();
        compiled.push(code);
    }
    let add_thunk = |compiled: &mut Vec<Function>, name: String, function: Function| -> FuncId {
        let id = u32::try_from(compiled.len()).unwrap_or(u32::MAX);
        compiled.push(Function { name, ..function });
        id
    };
    let mut state_fields = Vec::new();
    for (index, field) in decoder.state.iter().enumerate() {
        let ty = Ty::from_ir(&field.ty);
        let path = format!("decoder.state[{index}]");
        let (function, _) = Compiler::new(&env, &path).thunk(&[], true, &field.init, Some(&ty))?;
        let init = add_thunk(&mut compiled, format!("init {}", field.name), function);
        state_fields.push(StateField {
            name: field.name.clone(),
            ty,
            init,
        });
    }
    let config_param = [("config".to_owned(), Ty::Named(decoder.config.ty.clone()))];
    let mut props = Vec::new();
    for (index, prop) in decoder.props.iter().enumerate() {
        let path = format!("decoder.props[{index}]");
        let (function, ty) =
            Compiler::new(&env, &path).thunk(&config_param, false, &prop.value, None)?;
        let value = add_thunk(&mut compiled, format!("prop {}", prop.key), function);
        props.push(Prop {
            key: prop.key.clone(),
            value,
            ty,
        });
    }
    let mut requires = Vec::new();
    for (index, require) in decoder.requires.iter().enumerate() {
        let mut values = Vec::new();
        for (j, value) in require.values.iter().enumerate() {
            let path = format!("decoder.requires[{index}].values[{j}]");
            let (function, ty) =
                Compiler::new(&env, &path).thunk(&config_param, false, value, None)?;
            let id = add_thunk(&mut compiled, format!("require {}", require.key), function);
            values.push((id, ty));
        }
        requires.push(Require {
            key: require.key.clone(),
            kind: require.requirement,
            values,
        });
    }

    let method_id = |name: &str| env.methods.get(name).map(|sig| sig.id);
    let hooks = Hooks {
        run: method_id("run"),
        transaction: method_id("transaction"),
        recover: method_id("recover"),
        hold: method_id("hold"),
        on_gap: method_id("onGap"),
        on_end: method_id("onEnd"),
        on_progress: method_id("onProgress"),
    };
    let validate = match &decoder.config.validate {
        Some(name) => Some(env.functions.get(name).map(|sig| sig.id).ok_or_else(|| {
            unsupported(
                "decoder.config.validate",
                format!("{name} is not a function"),
            )
        })?),
        None => None,
    };
    let defaults = decoder.config.defaults.clone();
    if let Some((_, record)) = types.record(&decoder.config.ty) {
        let object = defaults
            .as_object()
            .ok_or_else(|| unsupported("decoder.config.defaults", "the defaults are an object"))?;
        for field in &record.fields {
            if !object.contains_key(&field.name) {
                return Err(unsupported(
                    "decoder.config.defaults",
                    format!("no default for {}", field.name),
                ));
            }
        }
    }
    let recovery = match &decoder.recovery {
        Some(recovery) => {
            let variant = |name: &str| {
                types.enum_variant(&recovery.reason, name).ok_or_else(|| {
                    unsupported(
                        "decoder.recovery",
                        format!("{name} is not a {}", recovery.reason),
                    )
                })
            };
            Some(Recovery {
                incomplete: variant(&recovery.incomplete)?,
                truncated: variant(&recovery.truncated)?,
                missing: variant(&recovery.missing)?,
                gap: variant(&recovery.gap)?,
                end: variant(&recovery.end)?,
            })
        }
        None => None,
    };
    let input = match (kind, lower) {
        (Kind::Logic, _) => Input::Logic,
        (Kind::Stream, Some(contract)) => Input::Stream {
            schema: schema_id(&contract.schema),
            event: contract.event.clone(),
        },
        (Kind::Bus, Some(contract)) => {
            let framing = contract.framing.as_ref().ok_or_else(|| {
                unsupported("decoder.input", "a bus input reads a framed contract")
            })?;
            Input::Bus {
                schema: schema_id(&contract.schema),
                event: contract.event.clone(),
                framing: Box::new(FramingData {
                    begin: framing.begin.clone(),
                    items: framing.items.clone(),
                    end: framing.end.clone(),
                    id: framing.id.clone(),
                    partial: framing.partial.clone(),
                    reason: framing.reason.clone(),
                    normal: framing.normal.clone(),
                    unframed: framing.unframed,
                    item: Ty::Named(item_type_name(&contract.event)),
                }),
            }
        }
        (_, None) => {
            return Err(unsupported(
                "decoder.input",
                "the lower contract is missing",
            ));
        }
    };
    Ok(Program {
        name: module.name.clone(),
        category: decoder.category,
        kind,
        types,
        functions: compiled,
        hooks,
        config_ty: decoder.config.ty.clone(),
        defaults,
        validate,
        state: state_fields,
        props,
        requires,
        input,
        output: schema_id(&decoder.output.schema),
        event,
        partial: match decoder.partial {
            Some(uiharu_ir::Partial::Read) => Partial::Read,
            _ => Partial::Drain,
        },
        recovery,
    })
}
