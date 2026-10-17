// SPDX-License-Identifier: Apache-2.0

//! Expressions and statements to Rust, typed as they go: integer widths are reconciled, options
//! wrapped, records passed by reference, operations awaited through the runtime's wrappers.

use std::collections::HashMap;

use uiharu_ir::{BinaryOp, Expr, Literal, Op, Pattern, Stmt, UnaryOp};

use crate::names::{pascal, plain_literal, snake, string_literal};
use crate::types::{Defs, Ty};
use crate::{GenError, Kind};

/// A function's parameter as the emitter knows it.
#[derive(Clone, Debug)]
pub struct ParamSig {
    pub name: String,
    pub ty: Ty,
    pub mutable: bool,
}

/// A function or method as the emitter knows it.
#[derive(Clone, Debug)]
pub struct Sig {
    pub params: Vec<ParamSig>,
    pub ret: Ty,
    pub is_async: bool,
    /// Whether a method takes `&mut self`.
    pub mutates: bool,
}

/// What the module gives every function's emitter.
pub struct Env<'a> {
    pub defs: &'a Defs<'a>,
    pub consts: HashMap<String, Ty>,
    pub functions: HashMap<String, Sig>,
    pub methods: HashMap<String, Sig>,
    pub state: HashMap<String, Ty>,
    pub kind: Kind,
    /// The lower event type, for stream and bus decoders.
    pub lower: Option<String>,
}

#[derive(Clone, Debug)]
struct Local {
    ty: Ty,
    /// Whether the local is a reference to a record.
    is_ref: bool,
}

/// An emitted expression: its Rust, its type, and whether it is a reference.
#[derive(Clone, Debug)]
pub struct Val {
    pub code: String,
    pub ty: Ty,
    pub is_ref: bool,
}

impl Val {
    fn new(code: impl Into<String>, ty: Ty) -> Self {
        Self {
            code: code.into(),
            ty,
            is_ref: false,
        }
    }
}

/// Emits one function's body.
pub struct Emitter<'a> {
    pub env: &'a Env<'a>,
    scopes: Vec<HashMap<String, Local>>,
    /// Whether `self` is a decoder's state (a method), and whether it is being made (`new`,
    /// where fields are locals).
    in_method: bool,
    in_new: bool,
    /// Whether the function returns `Result<_, Cut>`.
    returns_result: bool,
    /// The parameter that is the frame cursor, if any.
    cursor: Option<String>,
    /// The function's return type.
    ret: Ty,
    indent: usize,
    /// Temporaries to declare before the statement being emitted: the awaited arguments of
    /// a call on `self`, which cannot borrow `self` while the call does.
    hoisted: Vec<String>,
    temps: usize,
}

impl<'a> Emitter<'a> {
    pub fn new(env: &'a Env<'a>) -> Self {
        Self {
            env,
            scopes: vec![HashMap::new()],
            in_method: false,
            in_new: false,
            returns_result: false,
            cursor: None,
            ret: Ty::Unit,
            indent: 1,
            hoisted: Vec::new(),
            temps: 0,
        }
    }

    pub fn function(
        mut self,
        params: &[ParamSig],
        ret: Ty,
        method: bool,
        is_async: bool,
        body: &[Stmt],
    ) -> Result<String, GenError> {
        self.in_method = method;
        self.returns_result = is_async;
        self.ret = ret.clone();
        for param in params {
            let is_ref = self
                .env
                .defs
                .record_fields(&named(&param.ty).unwrap_or_default())
                .is_some();
            if param.ty == Ty::Cursor {
                self.cursor = Some(snake(&param.name));
            }
            self.declare(&param.name, param.ty.clone(), is_ref);
        }
        let mut out = String::new();
        self.block_into(&mut out, body)?;
        if is_async && !ends_with_return(body) {
            self.line(&mut out, "Ok(())");
        }
        Ok(out)
    }

    /// Emits the initializer of a state field in `State::new`, with earlier fields as locals.
    pub fn initializer(
        mut self,
        fields: &[(String, Ty)],
        init: &Expr,
        expected: &Ty,
    ) -> Result<String, GenError> {
        self.in_method = true;
        self.in_new = true;
        for (name, ty) in fields {
            self.declare(name, ty.clone(), false);
        }
        let val = self.expr(init, Some(expected))?;
        Ok(self.coerce(val, expected)?.code)
    }

    /// Emits an expression with `config` in scope, for requirements and properties.
    pub fn with_config(mut self, config_ty: Ty, expr: &Expr) -> Result<Val, GenError> {
        self.declare("config", config_ty, true);
        self.expr(expr, None)
    }

    // Scopes

    fn declare(&mut self, name: &str, ty: Ty, is_ref: bool) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_owned(), Local { ty, is_ref });
        }
    }

    fn local(&self, name: &str) -> Option<&Local> {
        self.scopes.iter().rev().find_map(|scope| scope.get(name))
    }

    fn line(&self, out: &mut String, text: &str) {
        for _ in 0..self.indent {
            out.push_str("    ");
        }
        out.push_str(text);
        out.push('\n');
    }

    // Statements

    fn block_into(&mut self, out: &mut String, body: &[Stmt]) -> Result<(), GenError> {
        self.scopes.push(HashMap::new());
        for stmt in body {
            self.stmt(out, stmt)?;
        }
        self.scopes.pop();
        Ok(())
    }

    fn nested(&mut self, body: &[Stmt]) -> Result<String, GenError> {
        self.indent += 1;
        let mut out = String::new();
        let result = self.block_into(&mut out, body);
        self.indent -= 1;
        result?;
        Ok(out)
    }

    fn stmt(&mut self, out: &mut String, stmt: &Stmt) -> Result<(), GenError> {
        let mut text = String::new();
        self.stmt_into(&mut text, stmt)?;
        for hoisted in std::mem::take(&mut self.hoisted) {
            self.line(out, &hoisted);
        }
        out.push_str(&text);
        Ok(())
    }

    fn stmt_into(&mut self, out: &mut String, stmt: &Stmt) -> Result<(), GenError> {
        match stmt {
            Stmt::Let {
                name,
                mutable,
                ty,
                value,
            } => {
                let declared = ty.as_ref().map(Ty::from_ir);
                let val = self.expr(value, declared.as_ref())?;
                let (code, ty) = match declared {
                    Some(declared) => (self.coerce(val, &declared)?.code, declared),
                    None => {
                        if val.ty == Ty::IntLit {
                            return Err(GenError::new(format!("the width of {name} is unknown")));
                        }
                        (val.code, val.ty)
                    }
                };
                let rust = ty.rust(self.env.lower.as_deref());
                let mutability = if *mutable { "mut " } else { "" };
                self.line(
                    out,
                    &format!("let {mutability}{}: {rust} = {code};", snake(name)),
                );
                self.declare(name, ty, false);
            }
            Stmt::LetRef { name, place } => {
                let val = self.expr(place, None)?;
                let ty = val.ty.clone();
                self.line(out, &format!("let {} = &mut {};", snake(name), val.code));
                self.declare(name, ty, true);
            }
            Stmt::Assign { target, op, value } => {
                let place = self.expr(target, None)?;
                let value = match op {
                    None => {
                        let val = self.expr(value, Some(&place.ty))?;
                        self.coerce(val, &place.ty)?.code
                    }
                    Some(op) => {
                        let right = self.expr(value, Some(&place.ty))?;
                        self.binary(*op, Val::new(deref(&place), place.ty.clone()), right)?
                            .code
                    }
                };
                self.line(out, &format!("{} = {value};", place.code));
            }
            Stmt::If {
                cond,
                then,
                otherwise,
            } => {
                let cond = self.expr(cond, Some(&Ty::Bool))?;
                let cond = self.coerce(cond, &Ty::Bool)?.code;
                let then = self.nested(then)?;
                self.line(out, &format!("if {cond} {{"));
                out.push_str(&then);
                if otherwise.is_empty() {
                    self.line(out, "}");
                } else {
                    let otherwise = self.nested(otherwise)?;
                    self.line(out, "} else {");
                    out.push_str(&otherwise);
                    self.line(out, "}");
                }
            }
            Stmt::IfSome {
                value,
                bind,
                borrow,
                then,
                otherwise,
            } => {
                let val = self.expr(value, None)?;
                let inner = match &val.ty {
                    Ty::Option(inner) => (**inner).clone(),
                    other => return Err(GenError::new(format!("if-some on a {other:?}"))),
                };
                let subject = if *borrow {
                    format!("{}.as_mut()", val.code)
                } else {
                    val.code
                };
                self.scopes.push(HashMap::new());
                self.declare(bind, inner, *borrow);
                let then = self.nested(then)?;
                self.scopes.pop();
                self.line(out, &format!("if let Some({}) = {subject} {{", snake(bind)));
                out.push_str(&then);
                if otherwise.is_empty() {
                    self.line(out, "}");
                } else {
                    let otherwise = self.nested(otherwise)?;
                    self.line(out, "} else {");
                    out.push_str(&otherwise);
                    self.line(out, "}");
                }
            }
            Stmt::LetSome {
                bind,
                value,
                borrow,
                otherwise,
            } => {
                let val = self.expr(value, None)?;
                let inner = match &val.ty {
                    Ty::Option(inner) => (**inner).clone(),
                    other => return Err(GenError::new(format!("let-some on a {other:?}"))),
                };
                let subject = if *borrow {
                    format!("{}.as_mut()", val.code)
                } else {
                    val.code
                };
                let otherwise = self.nested(otherwise)?;
                self.line(
                    out,
                    &format!("let Some({}) = {subject} else {{", snake(bind)),
                );
                out.push_str(&otherwise);
                self.line(out, "};");
                self.declare(bind, inner, *borrow);
            }
            Stmt::Match { value, arms } => {
                let val = self.expr(value, None)?;
                let union = named(&val.ty);
                let subject =
                    if self.env.defs.is_copy(&val.ty) || val.ty.is_int() || val.ty == Ty::Bool {
                        deref(&val)
                    } else {
                        format!("{}.clone()", val.code)
                    };
                self.line(out, &format!("match {subject} {{"));
                for arm in arms {
                    self.scopes.push(HashMap::new());
                    let pattern = match &arm.pattern {
                        Pattern::Variant { name, binds } => {
                            let union = union
                                .clone()
                                .ok_or_else(|| GenError::new("a variant pattern needs a union"))?;
                            let fields =
                                self.env.defs.variant_fields(&union, name).ok_or_else(|| {
                                    GenError::new(format!("{union} has no variant {name}"))
                                })?;
                            let mut parts = Vec::new();
                            for (field, local) in binds {
                                let ty = fields
                                    .iter()
                                    .find(|f| &f.name == field)
                                    .map(|f| Ty::from_ir(&f.ty))
                                    .ok_or_else(|| {
                                        GenError::new(format!(
                                            "{union}::{name} has no field {field}"
                                        ))
                                    })?;
                                self.declare(local, ty, false);
                                parts.push(pattern_field(field, local));
                            }
                            if fields.is_empty() {
                                format!("{union}::{}", pascal(name))
                            } else {
                                parts.push("..".into());
                                format!("{union}::{} {{ {} }}", pascal(name), parts.join(", "))
                            }
                        }
                        Pattern::Values { values } => values
                            .iter()
                            .map(|value| self.pattern_literal(value, &val.ty))
                            .collect::<Result<Vec<_>, _>>()?
                            .join(" | "),
                        Pattern::Default => "_".into(),
                    };
                    let body = self.nested(&arm.body)?;
                    self.scopes.pop();
                    self.line(out, &format!("{pattern} => {{"));
                    out.push_str(&body);
                    self.line(out, "}");
                }
                self.line(out, "}");
            }
            Stmt::Loop { label, body } => {
                let body = self.nested(body)?;
                self.line(out, &format!("{}loop {{", label_prefix(label)));
                out.push_str(&body);
                self.line(out, "}");
            }
            Stmt::While { label, cond, body } => {
                let cond = self.expr(cond, Some(&Ty::Bool))?;
                let cond = self.coerce(cond, &Ty::Bool)?.code;
                let body = self.nested(body)?;
                self.line(out, &format!("{}while {cond} {{", label_prefix(label)));
                out.push_str(&body);
                self.line(out, "}");
            }
            Stmt::For {
                label,
                var,
                from,
                to,
                body,
            } => {
                let from = self.expr(from, None)?;
                let to = self.expr(to, None)?;
                let ty = common_int(&from.ty, &to.ty).unwrap_or(Ty::U64);
                let from = self.coerce(from, &ty)?.code;
                let to = self.coerce(to, &ty)?.code;
                self.scopes.push(HashMap::new());
                self.declare(var, ty.clone(), false);
                let body = self.nested(body)?;
                self.scopes.pop();
                let rust = ty.rust(self.env.lower.as_deref());
                self.line(
                    out,
                    &format!(
                        "{}for {} in ({from} as {rust})..({to} as {rust}) {{",
                        label_prefix(label),
                        snake(var)
                    ),
                );
                out.push_str(&body);
                self.line(out, "}");
            }
            Stmt::ForEach {
                label,
                var,
                array,
                body,
            } => {
                let array = self.expr(array, None)?;
                let item = match &array.ty {
                    Ty::Array(item, _) => (**item).clone(),
                    other => return Err(GenError::new(format!("for-each over a {other:?}"))),
                };
                self.scopes.push(HashMap::new());
                self.declare(var, item, false);
                let body = self.nested(body)?;
                self.scopes.pop();
                self.line(
                    out,
                    &format!(
                        "{}for {} in {} {{",
                        label_prefix(label),
                        snake(var),
                        array.code
                    ),
                );
                out.push_str(&body);
                self.line(out, "}");
            }
            Stmt::Break { label } => self.line(out, &format!("break{};", label_suffix(label))),
            Stmt::Continue { label } => {
                self.line(out, &format!("continue{};", label_suffix(label)))
            }
            Stmt::Return { value } => {
                let code = match value {
                    Some(value) => {
                        let ret = self.ret.clone();
                        let val = self.expr(value, Some(&ret))?;
                        self.coerce(val, &ret)?.code
                    }
                    None => "()".into(),
                };
                if self.returns_result {
                    self.line(out, &format!("return Ok({code});"));
                } else if code == "()" {
                    self.line(out, "return;");
                } else {
                    self.line(out, &format!("return {code};"));
                }
            }
            Stmt::Expr { expr } => {
                let val = self.expr(expr, None)?;
                self.line(out, &format!("{};", val.code));
            }
        }
        Ok(())
    }

    fn pattern_literal(&self, value: &Literal, ty: &Ty) -> Result<String, GenError> {
        match (value, ty) {
            (Literal::Text(name), Ty::Named(enum_name)) => {
                Ok(format!("{enum_name}::{}", pascal(name)))
            }
            (Literal::Int(n), _) => Ok(n.to_string()),
            (Literal::Bool(b), _) => Ok(b.to_string()),
            (Literal::Text(text), other) => Err(GenError::new(format!(
                "{text:?} is not a pattern of {other:?}"
            ))),
        }
    }

    // Expressions

    pub fn expr(&mut self, expr: &Expr, expected: Option<&Ty>) -> Result<Val, GenError> {
        match expr {
            Expr::Lit {
                value: Literal::Int(n),
            } => Ok(Val::new(n.to_string(), Ty::IntLit)),
            Expr::Lit {
                value: Literal::Bool(b),
            } => Ok(Val::new(b.to_string(), Ty::Bool)),
            Expr::Lit {
                value: Literal::Text(text),
            }
            | Expr::Text { value: text } => Ok(Val::new(
                format!("String::from({})", plain_literal(text)),
                Ty::Text,
            )),
            Expr::Format { parts } => {
                let mut format = String::new();
                let mut args = Vec::new();
                for part in parts {
                    if let Expr::Text { value } = part {
                        let literal = string_literal(value);
                        format.push_str(&literal[1..literal.len() - 1]);
                    } else {
                        format.push_str("{}");
                        args.push(deref(&self.expr(part, None)?));
                    }
                }
                if args.is_empty() {
                    Ok(Val::new(format!("String::from(\"{format}\")"), Ty::Text))
                } else {
                    Ok(Val::new(
                        format!("format!(\"{format}\", {})", args.join(", ")),
                        Ty::Text,
                    ))
                }
            }
            Expr::None => Ok(Val::new(
                "None",
                expected.cloned().unwrap_or(Ty::Option(Box::new(Ty::Unit))),
            )),
            Expr::Local { name } => {
                if let Some(local) = self.local(name) {
                    return Ok(Val {
                        code: snake(name),
                        ty: local.ty.clone(),
                        is_ref: local.is_ref,
                    });
                }
                if let Some(ty) = self.env.consts.get(name) {
                    return Ok(Val::new(name.clone(), ty.clone()));
                }
                Err(GenError::new(format!("{name} is not in scope")))
            }
            Expr::SelfField { field } => {
                let ty =
                    match field.as_str() {
                        "rate" => Ty::U64,
                        "config" => Ty::Named(self.env.config_type()),
                        other => self.env.state.get(other).cloned().ok_or_else(|| {
                            GenError::new(format!("the state has no field {other}"))
                        })?,
                    };
                let code = if self.in_new {
                    snake(field)
                } else {
                    format!("self.{}", snake(field))
                };
                Ok(Val::new(code, ty))
            }
            Expr::Field { of, name } => {
                let of = self.expr(of, None)?;
                self.field(of, name)
            }
            Expr::Index { of, at } => {
                let of = self.expr(of, None)?;
                let at = self.expr(at, Some(&Ty::U64))?;
                let item = match &of.ty {
                    Ty::Array(item, _) => (**item).clone(),
                    other => return Err(GenError::new(format!("index into a {other:?}"))),
                };
                Ok(Val::new(format!("{}[{} as usize]", of.code, at.code), item))
            }
            Expr::Tag { .. } => Err(GenError::new("a tag is read through a match")),
            Expr::Unary { op, value } => {
                let val = self.expr(value, None)?;
                match op {
                    UnaryOp::Not => Ok(Val::new(format!("(!{})", deref(&val)), Ty::Bool)),
                    UnaryOp::BitNot => Ok(Val::new(format!("(!{})", deref(&val)), val.ty)),
                }
            }
            Expr::Binary { op, left, right } => {
                // Arithmetic is done at the width of its context, when that is wider than its
                // operands: what the result is assigned to, passed as or returned as.
                let context = expected
                    .filter(|ty| ty.is_int() && **ty != Ty::IntLit)
                    .cloned();
                let left = self.expr(left, context.as_ref())?;
                let right = self.expr(right, context.as_ref().or(Some(&left.ty)))?;
                self.binary_in(*op, left, right, context.as_ref())
            }
            Expr::Cast { value, to } => {
                let to = Ty::from_ir(to);
                let val = self.expr(value, None)?;
                Ok(Val::new(
                    format!("({} as {})", deref(&val), to.rust(None)),
                    to,
                ))
            }
            Expr::Record { ty, fields } => {
                let defs = self
                    .env
                    .defs
                    .record_fields(ty)
                    .ok_or_else(|| GenError::new(format!("{ty} is not a record")))?;
                let mut parts = Vec::new();
                for field in defs {
                    let value = fields
                        .get(&field.name)
                        .ok_or_else(|| GenError::new(format!("{ty} needs {}", field.name)))?;
                    let want = Ty::from_ir(&field.ty);
                    let val = self.expr(value, Some(&want))?;
                    parts.push(format!(
                        "{}: {}",
                        snake(&field.name),
                        self.coerce(val, &want)?.code
                    ));
                }
                Ok(Val::new(
                    format!("{ty} {{ {} }}", parts.join(", ")),
                    Ty::Named(ty.clone()),
                ))
            }
            Expr::Variant { ty, kind, fields } => {
                let defs = self
                    .env
                    .defs
                    .variant_fields(ty, kind)
                    .ok_or_else(|| GenError::new(format!("{ty} has no variant {kind}")))?;
                if defs.is_empty() {
                    return Ok(Val::new(
                        format!("{ty}::{}", pascal(kind)),
                        Ty::Named(ty.clone()),
                    ));
                }
                let mut parts = Vec::new();
                for field in defs {
                    let value = fields.get(&field.name).ok_or_else(|| {
                        GenError::new(format!("{ty}::{kind} needs {}", field.name))
                    })?;
                    let want = Ty::from_ir(&field.ty);
                    let val = self.expr(value, Some(&want))?;
                    parts.push(format!(
                        "{}: {}",
                        snake(&field.name),
                        self.coerce(val, &want)?.code
                    ));
                }
                Ok(Val::new(
                    format!("{ty}::{} {{ {} }}", pascal(kind), parts.join(", ")),
                    Ty::Named(ty.clone()),
                ))
            }
            Expr::Enum { ty, value } => Ok(Val::new(
                format!("{ty}::{}", pascal(value)),
                Ty::Named(ty.clone()),
            )),
            Expr::Array { items } => {
                let want = match expected {
                    Some(Ty::Array(item, _)) => Some((**item).clone()),
                    _ => None,
                };
                // The items meet at one type: an option when any of them is one.
                let mut vals = Vec::new();
                for item in items {
                    vals.push(self.expr(item, want.as_ref())?);
                }
                let item_ty = want.unwrap_or_else(|| {
                    vals.iter()
                        .map(|val| val.ty.clone())
                        .reduce(|a, b| join_types(&a, &b))
                        .unwrap_or(Ty::U64)
                });
                let item_ty = if item_ty == Ty::IntLit {
                    Ty::U64
                } else {
                    item_ty
                };
                let mut codes = Vec::new();
                for val in vals {
                    codes.push(self.coerce(val, &item_ty)?.code);
                }
                let bound = u32::try_from(items.len()).unwrap_or(u32::MAX);
                let code = if matches!(expected, Some(Ty::Array(..))) {
                    format!("vec![{}]", codes.join(", "))
                } else {
                    format!("[{}]", codes.join(", "))
                };
                Ok(Val::new(code, Ty::Array(Box::new(item_ty), bound)))
            }
            Expr::Call { name, args } => {
                let sig = self
                    .env
                    .functions
                    .get(name)
                    .cloned()
                    .ok_or_else(|| GenError::new(format!("{name} is not a function")))?;
                let args = self.arguments(&sig, args)?;
                Ok(Val::new(
                    format!("{}({})", snake(name), args.join(", ")),
                    sig.ret,
                ))
            }
            Expr::Method { name, args } => {
                let sig = self
                    .env
                    .methods
                    .get(name)
                    .cloned()
                    .ok_or_else(|| GenError::new(format!("{name} is not a method")))?;
                if sig.is_async {
                    return Err(GenError::new(format!("{name} is async: await it")));
                }
                let mut args = self.arguments(&sig, args)?;
                // An awaited argument borrows `self` through the runtime's wrapper while the call
                // borrows it too: every argument of such a call is evaluated first, in order.
                if args.iter().any(|arg| arg.contains(".await")) {
                    for arg in &mut args {
                        self.temps += 1;
                        let temp = format!("temp_{}", self.temps);
                        self.hoisted.push(format!("let {temp} = {arg};"));
                        *arg = temp;
                    }
                }
                Ok(Val::new(
                    format!("self.{}({})", snake(name), args.join(", ")),
                    sig.ret,
                ))
            }
            Expr::Intrinsic { name, args } => self.intrinsic(name, args, expected),
            Expr::Ternary {
                cond,
                then,
                otherwise,
            } => {
                let cond = self.expr(cond, Some(&Ty::Bool))?;
                let cond = self.coerce(cond, &Ty::Bool)?.code;
                let then = self.expr(then, expected)?;
                let otherwise = self.expr(otherwise, expected.or(Some(&then.ty)))?;
                let ty = expected
                    .cloned()
                    .unwrap_or_else(|| join_types(&then.ty, &otherwise.ty));
                let then = self.coerce(then, &ty)?.code;
                let otherwise = self.coerce(otherwise, &ty)?.code;
                Ok(Val::new(
                    format!("if {cond} {{ {then} }} else {{ {otherwise} }}"),
                    ty,
                ))
            }
            Expr::IfSome {
                value,
                bind,
                some,
                none,
            } => {
                let val = self.expr(value, None)?;
                let inner = match &val.ty {
                    Ty::Option(inner) => (**inner).clone(),
                    other => return Err(GenError::new(format!("if-some on a {other:?}"))),
                };
                self.scopes.push(HashMap::new());
                self.declare(bind, inner, false);
                let some = self.expr(some, expected)?;
                self.scopes.pop();
                let none = self.expr(none, expected.or(Some(&some.ty)))?;
                let ty = expected
                    .cloned()
                    .unwrap_or_else(|| join_types(&some.ty, &none.ty));
                let some = self.coerce(some, &ty)?.code;
                let none = self.coerce(none, &ty)?.code;
                Ok(Val::new(
                    format!(
                        "if let Some({}) = {} {{ {some} }} else {{ {none} }}",
                        snake(bind),
                        val.code
                    ),
                    ty,
                ))
            }
            Expr::IfVariant {
                value,
                variant,
                binds,
                then,
                otherwise,
            } => {
                let val = self.expr(value, None)?;
                let union =
                    named(&val.ty).ok_or_else(|| GenError::new("if-variant on a non-union"))?;
                let fields = self
                    .env
                    .defs
                    .variant_fields(&union, variant)
                    .ok_or_else(|| GenError::new(format!("{union} has no variant {variant}")))?;
                self.scopes.push(HashMap::new());
                let mut parts = Vec::new();
                for (field, local) in binds {
                    let ty = fields
                        .iter()
                        .find(|f| &f.name == field)
                        .map(|f| Ty::from_ir(&f.ty))
                        .ok_or_else(|| {
                            GenError::new(format!("{union}::{variant} has no field {field}"))
                        })?;
                    self.declare(local, ty, false);
                    parts.push(pattern_field(field, local));
                }
                let then = self.expr(then, expected)?;
                self.scopes.pop();
                let otherwise = self.expr(otherwise, expected.or(Some(&then.ty)))?;
                let ty = expected
                    .cloned()
                    .unwrap_or_else(|| join_types(&then.ty, &otherwise.ty));
                let then = self.coerce(then, &ty)?.code;
                let otherwise = self.coerce(otherwise, &ty)?.code;
                let pattern = if fields.is_empty() {
                    format!("{union}::{}", pascal(variant))
                } else {
                    parts.push("..".into());
                    format!("{union}::{} {{ {} }}", pascal(variant), parts.join(", "))
                };
                let subject = if self.env.defs.is_copy(&val.ty) {
                    deref(&val)
                } else {
                    format!("{}.clone()", val.code)
                };
                Ok(Val::new(
                    format!("if let {pattern} = {subject} {{ {then} }} else {{ {otherwise} }}"),
                    ty,
                ))
            }
            Expr::Required { value } => {
                let val = self.expr(value, None)?;
                let inner = match &val.ty {
                    Ty::Option(inner) => (**inner).clone(),
                    other => return Err(GenError::new(format!("required on a {other:?}"))),
                };
                Ok(Val::new(format!("required({})?", val.code), inner))
            }
            Expr::Await { op } => self.operation(op),
        }
    }

    fn field(&mut self, of: Val, name: &str) -> Result<Val, GenError> {
        let code = format!("{}.{}", of.code, snake(name));
        match &of.ty {
            Ty::Named(record) => {
                let fields = self.env.defs.record_fields(record).ok_or_else(|| {
                    GenError::new(format!(
                        "{record} is not a record; read its kind in a match"
                    ))
                })?;
                let field = fields
                    .iter()
                    .find(|f| f.name == name)
                    .ok_or_else(|| GenError::new(format!("{record} has no field {name}")))?;
                Ok(Val::new(code, Ty::from_ir(&field.ty)))
            }
            Ty::Edge => match name {
                "tick" => Ok(Val::new(code, Ty::U64)),
                "changed" => Ok(Val::new(code, Ty::Mask)),
                "levels" => Ok(Val::new(code, Ty::Levels)),
                _ => Err(GenError::new(format!("an edge has no {name}"))),
            },
            Ty::Received(event) => match name {
                "event" => Ok(Val::new(code, Ty::Named(event.clone()))),
                "at" => Ok(Val::new(code, Ty::Provenance)),
                _ => Err(GenError::new(format!("a received event has no {name}"))),
            },
            Ty::Taken(item) => match name {
                "items" => Ok(Val::new(code, Ty::Array(item.clone(), u32::MAX))),
                "at" => Ok(Val::new(code, Ty::Option(Box::new(Ty::Provenance)))),
                _ => Err(GenError::new(format!("a batch's items have no {name}"))),
            },
            Ty::Provenance => match name {
                "span" => Ok(Val::new(code, Ty::Range)),
                "settled" => Ok(Val::new(format!("{code}.0"), Ty::U64)),
                _ => Err(GenError::new(format!("a provenance has no {name}"))),
            },
            Ty::Range => match name {
                "start" | "end" => Ok(Val::new(format!("{code}.0"), Ty::U64)),
                _ => Err(GenError::new(format!("a range has no {name}"))),
            },
            Ty::Timed => Err(GenError::new("a timed result is read through a match")),
            other => Err(GenError::new(format!("{other:?} has no field {name}"))),
        }
    }

    /// The type of the items the lower contract's cursor hands out.
    fn item_type(&self) -> Result<Ty, GenError> {
        let lower = self
            .env
            .lower
            .clone()
            .ok_or_else(|| GenError::new("a frame cursor needs a lower contract"))?;
        Ok(Ty::Named(format!("{lower}Item")))
    }

    fn arguments(&mut self, sig: &Sig, args: &[Expr]) -> Result<Vec<String>, GenError> {
        if args.len() != sig.params.len() {
            return Err(GenError::new("wrong number of arguments"));
        }
        let mut out = Vec::new();
        for (arg, param) in args.iter().zip(&sig.params) {
            let val = self.expr(arg, Some(&param.ty))?;
            let is_record = self
                .env
                .defs
                .record_fields(&named(&param.ty).unwrap_or_default())
                .is_some();
            if is_record {
                let code = if val.is_ref {
                    if param.mutable {
                        val.code
                    } else {
                        format!("&*{}", val.code)
                    }
                } else if param.mutable {
                    format!("&mut {}", val.code)
                } else {
                    format!("&{}", val.code)
                };
                out.push(code);
            } else if param.ty == Ty::Cursor {
                out.push(val.code);
            } else {
                out.push(self.coerce(val, &param.ty)?.code);
            }
        }
        Ok(out)
    }

    fn binary(&mut self, op: BinaryOp, left: Val, right: Val) -> Result<Val, GenError> {
        self.binary_in(op, left, right, None)
    }

    /// The width an operation is done at: its context when wider than its operands.
    fn width_in(&self, operands: Option<Ty>, context: Option<&Ty>) -> Option<Ty> {
        match (operands, context) {
            (Some(operands), Some(context)) => {
                let (a, b) = (operands.width().unwrap_or(0), context.width().unwrap_or(0));
                Some(if b > a { context.clone() } else { operands })
            }
            (Some(operands), None) => Some(operands),
            (None, Some(context)) if context.is_int() => Some(context.clone()),
            (None, None) => None,
            (None, Some(_)) => None,
        }
    }

    fn binary_in(
        &mut self,
        op: BinaryOp,
        left: Val,
        right: Val,
        context: Option<&Ty>,
    ) -> Result<Val, GenError> {
        match op {
            BinaryOp::And | BinaryOp::Or => {
                let symbol = if op == BinaryOp::And { "&&" } else { "||" };
                Ok(Val::new(
                    format!("({} {symbol} {})", deref(&left), deref(&right)),
                    Ty::Bool,
                ))
            }
            BinaryOp::Eq | BinaryOp::Ne => {
                let symbol = if op == BinaryOp::Eq { "==" } else { "!=" };
                let (l, r) = self.align(left, right)?;
                Ok(Val::new(format!("({l} {symbol} {r})"), Ty::Bool))
            }
            BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
                let symbol = match op {
                    BinaryOp::Lt => "<",
                    BinaryOp::Le => "<=",
                    BinaryOp::Gt => ">",
                    _ => ">=",
                };
                let (l, r) = self.align(left, right)?;
                Ok(Val::new(format!("({l} {symbol} {r})"), Ty::Bool))
            }
            BinaryOp::Shl | BinaryOp::Shr => {
                let method = if op == BinaryOp::Shl { "shl" } else { "shr" };
                let operand = if left.ty.is_int() && left.ty != Ty::IntLit {
                    Some(left.ty.clone())
                } else {
                    None
                };
                let ty = self.width_in(operand, context).unwrap_or(Ty::U64);
                let ty = if ty == Ty::IntLit { Ty::U64 } else { ty };
                let l = self.coerce(left, &ty)?.code;
                let amount = if right.ty == Ty::U64 || right.ty == Ty::IntLit {
                    deref(&right)
                } else {
                    format!("u64::from({})", deref(&right))
                };
                Ok(Val::new(format!("Int::{method}({l}, {amount})"), ty))
            }
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Rem => {
                let method = match op {
                    BinaryOp::Add => "wadd",
                    BinaryOp::Sub => "wsub",
                    BinaryOp::Mul => "wmul",
                    BinaryOp::Div => "div0",
                    _ => "rem0",
                };
                let operands = common_int(&left.ty, &right.ty).ok_or_else(|| {
                    GenError::new(format!("arithmetic on {:?} and {:?}", left.ty, right.ty))
                })?;
                let ty = self.width_in(Some(operands), context).unwrap_or(Ty::U64);
                let l = self.coerce(left, &ty)?.code;
                let r = self.coerce(right, &ty)?.code;
                Ok(Val::new(format!("Int::{method}({l}, {r})"), ty))
            }
            BinaryOp::BitAnd | BinaryOp::BitOr | BinaryOp::BitXor => {
                let symbol = match op {
                    BinaryOp::BitAnd => "&",
                    BinaryOp::BitOr => "|",
                    _ => "^",
                };
                let operands = common_int(&left.ty, &right.ty)
                    .ok_or_else(|| GenError::new("bit operation on non-integers"))?;
                let ty = self.width_in(Some(operands), context).unwrap_or(Ty::U64);
                let l = self.coerce(left, &ty)?.code;
                let r = self.coerce(right, &ty)?.code;
                Ok(Val::new(format!("({l} {symbol} {r})"), ty))
            }
        }
    }

    /// Both sides of a comparison at one type.
    fn align(&mut self, left: Val, right: Val) -> Result<(String, String), GenError> {
        if left.ty.is_int() && right.ty.is_int() {
            let ty = common_int(&left.ty, &right.ty).unwrap_or(Ty::U64);
            return Ok((self.coerce(left, &ty)?.code, self.coerce(right, &ty)?.code));
        }
        let ty = match (&left.ty, &right.ty) {
            (Ty::Option(_), _) => left.ty.clone(),
            (_, Ty::Option(_)) => right.ty.clone(),
            (l, _) if *l != Ty::IntLit => l.clone(),
            (_, r) => r.clone(),
        };
        let l = self.coerce(left, &ty)?.code;
        let r = self.coerce(right, &ty)?.code;
        Ok((l, r))
    }

    /// `val` as a `want`: integers widened, values wrapped in `Some`, references read.
    pub fn coerce(&self, val: Val, want: &Ty) -> Result<Val, GenError> {
        if &val.ty == want {
            return Ok(Val::new(deref(&val), want.clone()));
        }
        match (&val.ty, want) {
            (Ty::IntLit, w) if w.is_int() => Ok(Val::new(val.code, w.clone())),
            (Ty::IntLit, Ty::Option(inner)) if inner.is_int() => {
                Ok(Val::new(format!("Some({})", val.code), want.clone()))
            }
            (have, w) if have.is_int() && w.is_int() => {
                let (hw, ww) = (have.width().unwrap_or(64), w.width().unwrap_or(64));
                if hw == ww {
                    Ok(Val::new(deref(&val), w.clone()))
                } else if hw < ww {
                    Ok(Val::new(
                        format!("{}::from({})", w.rust(None), deref(&val)),
                        w.clone(),
                    ))
                } else {
                    Err(GenError::new(format!("a {have:?} does not fit a {w:?}")))
                }
            }
            (Ty::Option(inner), Ty::Option(wanted)) => {
                // `None` of any option is any option.
                if val.code == "None" {
                    return Ok(Val::new("None", want.clone()));
                }
                if inner.is_int() && wanted.is_int() && inner.width() != wanted.width() {
                    let inner_rust = wanted.rust(None);
                    return Ok(Val::new(
                        format!("{}.map({inner_rust}::from)", deref(&val)),
                        want.clone(),
                    ));
                }
                if **inner == Ty::Unit {
                    return Ok(Val::new(val.code, want.clone()));
                }
                Ok(Val::new(deref(&val), want.clone()))
            }
            (have, Ty::Option(inner))
                if have == &**inner || (have.is_int() && inner.is_int()) || (have == &Ty::Unit) =>
            {
                let inner_val = self.coerce(Val::new(val.code.clone(), have.clone()), inner)?;
                Ok(Val::new(format!("Some({})", inner_val.code), want.clone()))
            }
            (Ty::Array(item, _), Ty::Array(wanted, _)) if item == wanted => {
                Ok(Val::new(deref(&val), want.clone()))
            }
            (Ty::Named(a), Ty::Named(b)) if a == b => Ok(Val::new(deref(&val), want.clone())),
            (have, Ty::Unit) => Ok(Val::new(val.code, have.clone())),
            (have, w) => Err(GenError::new(format!(
                "a {have:?} is not a {w:?} ({})",
                val.code
            ))),
        }
    }

    fn intrinsic(
        &mut self,
        name: &str,
        args: &[Expr],
        expected: Option<&Ty>,
    ) -> Result<Val, GenError> {
        let arg = |this: &mut Self, index: usize, want: Option<&Ty>| -> Result<Val, GenError> {
            let expr = args
                .get(index)
                .ok_or_else(|| GenError::new(format!("{name} needs argument {index}")))?;
            let val = this.expr(expr, want)?;
            match want {
                Some(want) => this.coerce(val, want),
                None => Ok(val),
            }
        };
        match name {
            "min" | "max" => {
                let a = arg(self, 0, None)?;
                let b = arg(self, 1, Some(&a.ty))?;
                let ty = common_int(&a.ty, &b.ty).unwrap_or(Ty::U64);
                let a = self.coerce(a, &ty)?.code;
                let b = self.coerce(b, &ty)?.code;
                Ok(Val::new(format!("std::cmp::{name}({a}, {b})"), ty))
            }
            "saturating-sub" => {
                let a = arg(self, 0, None)?;
                let b = arg(self, 1, Some(&a.ty))?;
                let ty = common_int(&a.ty, &b.ty).unwrap_or(Ty::U64);
                let a = self.coerce(a, &ty)?.code;
                let b = self.coerce(b, &ty)?.code;
                Ok(Val::new(format!("Int::ssub({a}, {b})"), ty))
            }
            "count-ones" => {
                let a = arg(self, 0, Some(&Ty::U64))?;
                Ok(Val::new(format!("({}.count_ones() as u8)", a.code), Ty::U8))
            }
            "from-bool" => {
                let a = arg(self, 0, Some(&Ty::Bool))?;
                Ok(Val::new(format!("u8::from({})", a.code), Ty::U8))
            }
            "mul-div" | "mul-div-ceil" => {
                let a = arg(self, 0, Some(&Ty::U64))?;
                let b = arg(self, 1, Some(&Ty::U64))?;
                let c = arg(self, 2, Some(&Ty::U64))?;
                let function = if name == "mul-div" {
                    "mul_div"
                } else {
                    "mul_div_ceil"
                };
                Ok(Val::new(
                    format!("{function}({}, {}, {})", a.code, b.code, c.code),
                    Ty::U64,
                ))
            }
            "at" | "one" => {
                let t = arg(self, 0, Some(&Ty::U64))?;
                Ok(Val::new(
                    format!("Provenance::{name}(Tick({}))", t.code),
                    Ty::Provenance,
                ))
            }
            "over" => {
                let a = arg(self, 0, Some(&Ty::U64))?;
                let b = arg(self, 1, Some(&Ty::U64))?;
                let c = arg(self, 2, Some(&Ty::U64))?;
                Ok(Val::new(
                    format!(
                        "Provenance::over(TickRange::new(Tick({}), Tick({})), Tick({}))",
                        a.code, b.code, c.code
                    ),
                    Ty::Provenance,
                ))
            }
            "join" => {
                let a = arg(self, 0, Some(&Ty::Provenance))?;
                let b = arg(self, 1, Some(&Ty::Provenance))?;
                Ok(Val::new(
                    format!("{}.join({})", a.code, b.code),
                    Ty::Provenance,
                ))
            }
            "mask" => {
                let option = Ty::Option(Box::new(Ty::U8));
                let mut parts = Vec::new();
                for index in 0..args.len() {
                    parts.push(arg(self, index, Some(&option))?.code);
                }
                Ok(Val::new(
                    format!("ChannelMask::of([{}])", parts.join(", ")),
                    Ty::Mask,
                ))
            }
            "mask-has" | "mask-with" => {
                let m = arg(self, 0, Some(&Ty::Mask))?;
                let c = arg(self, 1, Some(&Ty::U8))?;
                let method = if name == "mask-has" { "has" } else { "with" };
                let ty = if name == "mask-has" {
                    Ty::Bool
                } else {
                    Ty::Mask
                };
                Ok(Val::new(format!("{}.{method}({})", m.code, c.code), ty))
            }
            "levels-high" => {
                let l = arg(self, 0, Some(&Ty::Levels))?;
                let c = arg(self, 1, Some(&Ty::U8))?;
                Ok(Val::new(format!("{}.high({})", l.code, c.code), Ty::Bool))
            }
            "max-channels" => Ok(Val::new("MAX_CHANNELS", Ty::U8)),
            "position" => Ok(Val::new("io.signal.position().0", Ty::U64)),
            "seek" => {
                let t = arg(self, 0, Some(&Ty::U64))?;
                Ok(Val::new(
                    format!("io.signal.seek(Tick({}))", t.code),
                    Ty::Unit,
                ))
            }
            "batch" => {
                let ty = expected
                    .cloned()
                    .ok_or_else(|| GenError::new("a batch is a field with its capacity"))?;
                Ok(Val::new("Batch::new()", ty))
            }
            "batch-length" | "batch-capacity" | "batch-is-empty" | "batch-is-full" | "batch-at"
            | "batch-take" | "batch-clear" => {
                let b = arg(self, 0, None)?;
                let item = match &b.ty {
                    Ty::Batch(item, _) => (**item).clone(),
                    other => return Err(GenError::new(format!("{name} on a {other:?}"))),
                };
                let (method, ty) = match name {
                    "batch-length" => ("len", Ty::U32),
                    "batch-capacity" => ("capacity", Ty::U32),
                    "batch-is-empty" => ("is_empty", Ty::Bool),
                    "batch-is-full" => ("is_full", Ty::Bool),
                    "batch-at" => ("at", Ty::Option(Box::new(Ty::Provenance))),
                    "batch-take" => ("take", Ty::Taken(Box::new(item))),
                    _ => ("clear", Ty::Unit),
                };
                Ok(Val::new(format!("{}.{method}()", b.code), ty))
            }
            "batch-push" => {
                let b = arg(self, 0, None)?;
                let item = match &b.ty {
                    Ty::Batch(item, _) => (**item).clone(),
                    other => return Err(GenError::new(format!("batch-push on a {other:?}"))),
                };
                let value = arg(self, 1, Some(&item))?;
                let at = arg(self, 2, Some(&Ty::Provenance))?;
                Ok(Val::new(
                    format!("{}.push({}, {})", b.code, value.code, at.code),
                    Ty::Unit,
                ))
            }
            "consumed" => {
                if !self.returns_result {
                    return Err(GenError::new("consumed is read in an async method"));
                }
                Ok(Val::new("io.consumed()", Ty::Provenance))
            }
            "tx-id" => {
                let tx = arg(self, 0, None)?;
                Ok(Val::new(format!("{}.id()", tx.code), Ty::U64))
            }
            "tx-partial" => {
                let tx = arg(self, 0, None)?;
                Ok(Val::new(format!("{}.is_partial()", tx.code), Ty::Bool))
            }
            "unwrap-or" => {
                let a = arg(self, 0, None)?;
                let inner = match &a.ty {
                    Ty::Option(inner) => (**inner).clone(),
                    other => return Err(GenError::new(format!("unwrap-or on a {other:?}"))),
                };
                let b = arg(self, 1, Some(&inner))?;
                Ok(Val::new(format!("{}.unwrap_or({})", a.code, b.code), inner))
            }
            other => Err(GenError::new(format!("{other} is not an intrinsic"))),
        }
    }

    fn operation(&mut self, op: &Op) -> Result<Val, GenError> {
        if !self.returns_result {
            return Err(GenError::new("an operation is awaited in an async method"));
        }
        match op {
            Op::NextChange { mask } => {
                let mask = self.expr(mask, Some(&Ty::Mask))?;
                let mask = self.coerce(mask, &Ty::Mask)?.code;
                Ok(Val::new(
                    format!("self.next_change(io, {mask}).await?"),
                    Ty::Edge,
                ))
            }
            Op::NextChangeBefore { mask, until } => {
                let mask = self.expr(mask, Some(&Ty::Mask))?;
                let mask = self.coerce(mask, &Ty::Mask)?.code;
                let until = self.expr(until, Some(&Ty::U64))?;
                let until = self.coerce(until, &Ty::U64)?.code;
                Ok(Val::new(
                    format!("self.next_change_before(io, {mask}, {until}).await?"),
                    Ty::Timed,
                ))
            }
            Op::Sample => Ok(Val::new("self.sample(io).await?", Ty::Levels)),
            Op::Next => {
                let lower = self
                    .env
                    .lower
                    .clone()
                    .ok_or_else(|| GenError::new("next needs a lower stream"))?;
                Ok(Val::new("self.next(io).await?", Ty::Received(lower)))
            }
            Op::Item => {
                let tx = self
                    .cursor
                    .clone()
                    .ok_or_else(|| GenError::new("item needs the transaction cursor"))?;
                Ok(Val::new(
                    format!("{{ io.sink.hold(self.hold().map(Tick)); {tx}.item().await? }}"),
                    self.item_type()?,
                ))
            }
            Op::ItemOrEnd => {
                let tx = self
                    .cursor
                    .clone()
                    .ok_or_else(|| GenError::new("item-or-end needs the transaction cursor"))?;
                Ok(Val::new(
                    format!(
                        "{{ io.sink.hold(self.hold().map(Tick)); match {tx}.item_or_end().await? {{ Read::Item(item) => Some(item), Read::End => None }} }}"
                    ),
                    Ty::Option(Box::new(self.item_type()?)),
                ))
            }
            Op::Drain => {
                let tx = self
                    .cursor
                    .clone()
                    .ok_or_else(|| GenError::new("drain needs the transaction cursor"))?;
                Ok(Val::new(
                    format!("{{ io.sink.hold(self.hold().map(Tick)); {tx}.drain().await? }}"),
                    Ty::Unit,
                ))
            }
            Op::Emit { event, at } => {
                let want = Ty::Named(self.env.event_type());
                let event = self.expr(event, Some(&want))?;
                let event = self.coerce(event, &want)?.code;
                let at = match at {
                    Some(at) => {
                        let at = self.expr(at, Some(&Ty::Provenance))?;
                        self.coerce(at, &Ty::Provenance)?.code
                    }
                    None => {
                        if self.env.kind != Kind::Bus {
                            return Err(GenError::new(
                                "an event without provenance is a bus decoder's",
                            ));
                        }
                        "io.consumed()".to_owned()
                    }
                };
                Ok(Val::new(
                    format!("io.out.emit({at}, {event}).await"),
                    Ty::Unit,
                ))
            }
            Op::Method { name, args } => {
                let sig = self
                    .env
                    .methods
                    .get(name)
                    .cloned()
                    .ok_or_else(|| GenError::new(format!("{name} is not a method")))?;
                if !sig.is_async {
                    return Err(GenError::new(format!(
                        "{name} is synchronous: call it without await"
                    )));
                }
                let args = self.arguments(&sig, args)?;
                let mut all = vec!["io".to_owned()];
                all.extend(args);
                Ok(Val::new(
                    format!("self.{}({}).await?", snake(name), all.join(", ")),
                    sig.ret,
                ))
            }
        }
    }
}

impl Env<'_> {
    fn config_type(&self) -> String {
        self.state.get("config").and_then(named).unwrap_or_default()
    }

    fn event_type(&self) -> String {
        self.state.get("#event").and_then(named).unwrap_or_default()
    }
}

/// The name of a declared type.
pub fn named(ty: &Ty) -> Option<String> {
    match ty {
        Ty::Named(name) => Some(name.clone()),
        _ => None,
    }
}

/// A value read through a reference.
fn deref(val: &Val) -> String {
    if val.is_ref {
        format!("(*{})", val.code)
    } else {
        val.code.clone()
    }
}

/// The width two integers meet at.
fn common_int(a: &Ty, b: &Ty) -> Option<Ty> {
    match (a.width(), b.width()) {
        (Some(x), Some(y)) => Some(Ty::of_width(x.max(y))),
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

/// `field: local` in a pattern, or the shorthand when they are one name.
fn pattern_field(field: &str, local: &str) -> String {
    let (field, local) = (snake(field), snake(local));
    if field == local {
        field
    } else {
        format!("{field}: {local}")
    }
}

fn label_prefix(label: &Option<String>) -> String {
    label
        .as_ref()
        .map_or(String::new(), |l| format!("'{}: ", snake(l)))
}

fn label_suffix(label: &Option<String>) -> String {
    label
        .as_ref()
        .map_or(String::new(), |l| format!(" '{}", snake(l)))
}

/// Whether every path through the block leaves the function: it ends with a return, a loop
/// that never breaks, a match or an if-else whose every branch does.
fn ends_with_return(body: &[Stmt]) -> bool {
    match body.last() {
        Some(Stmt::Return { .. }) => true,
        Some(Stmt::Loop { body, .. }) => !breaks(body),
        Some(Stmt::Match { arms, .. }) => arms.iter().all(|arm| ends_with_return(&arm.body)),
        Some(Stmt::If {
            then, otherwise, ..
        }) => !otherwise.is_empty() && ends_with_return(then) && ends_with_return(otherwise),
        Some(Stmt::IfSome {
            then, otherwise, ..
        }) => !otherwise.is_empty() && ends_with_return(then) && ends_with_return(otherwise),
        _ => false,
    }
}

/// Whether a loop body breaks out of the loop (an unlabelled break outside nested loops, or
/// any labelled one).
fn breaks(body: &[Stmt]) -> bool {
    body.iter().any(|stmt| match stmt {
        Stmt::Break { .. } => true,
        Stmt::If {
            then, otherwise, ..
        }
        | Stmt::IfSome {
            then, otherwise, ..
        } => breaks(then) || breaks(otherwise),
        Stmt::LetSome { otherwise, .. } => breaks(otherwise),
        Stmt::Match { arms, .. } => arms.iter().any(|arm| breaks(&arm.body)),
        Stmt::Loop { body, .. }
        | Stmt::While { body, .. }
        | Stmt::For { body, .. }
        | Stmt::ForEach { body, .. } => body
            .iter()
            .any(|s| matches!(s, Stmt::Break { label: Some(_) })),
        _ => false,
    })
}
