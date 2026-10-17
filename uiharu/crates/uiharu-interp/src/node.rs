// SPDX-License-Identifier: Apache-2.0

//! The node that runs a loaded module, and the drivers through which the runtime's main loops
//! call its hooks.

use std::fmt;
use std::rc::Rc;
use std::sync::Arc;

use serde_json::Value as Json;
use uiharu_core::dsl::{
    BusDecoder, BusIo, ConfigError, Cut, CutKind, LogicDecoder, LogicIo, Partial, StreamDecoder,
    StreamIo, Transaction, run_bus, run_logic, run_stream,
};
use uiharu_core::{
    InputRef, InputSpec, Io, LogicBlock, Node, NodeFuture, NodeId, NodeSpec, OutputRef, OutputSpec,
    Props, Provenance, Requires, TickRange,
};
use uiharu_ir::RequireKind;

use crate::event::DynEvent;
use crate::framer::DynFramer;
use crate::intern::intern;
use crate::json;
use crate::program::{Input, Kind, Program};
use crate::value::Value;
use crate::vm::{Cx, FaultHandle, Machine};

/// A decoder interpreted from its module: the node a pipeline runs.
pub struct Interpreter {
    program: Arc<Program>,
    config: Value,
    props: Props,
    requires: Requires,
    fault: FaultHandle,
}

impl fmt::Debug for Interpreter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Interpreter")
            .field("program", &self.program.name())
            .finish_non_exhaustive()
    }
}

impl Interpreter {
    /// A decoder of `program` configured by `config`, which may leave fields to their defaults;
    /// the configuration is checked as the module's `validate` checks it.
    pub fn new(program: Arc<Program>, config: &Json) -> Result<Self, ConfigError> {
        let config = json::config_from_json(&program, config).map_err(ConfigError)?;
        let machine = Machine::detached(Arc::clone(&program));
        let run = |func: u32| {
            machine
                .execute_sync(func, vec![config.clone()])
                .map_err(|fault| ConfigError(format!("{fault:?}")))
        };
        if let Some(validate) = program.validate {
            match run(validate)? {
                Value::None => {}
                Value::Text(message) => return Err(ConfigError(message.to_string())),
                other => {
                    return Err(ConfigError(format!(
                        "the check returned {}",
                        other.describe()
                    )));
                }
            }
        }
        let mut props = Props::new();
        for prop in &program.props {
            let value = run(prop.value)?;
            let value = json::prop_value(&program.types, &prop.ty, &value).map_err(ConfigError)?;
            props = props.with(intern(&prop.key), value);
        }
        let mut requires = Requires::new();
        for require in &program.requires {
            let key = intern(&require.key);
            let mut values = Vec::new();
            for (func, ty) in &require.values {
                values.push((run(*func)?, ty));
            }
            let first = values
                .first()
                .ok_or_else(|| ConfigError("a requirement without a value".into()))?;
            requires = match require.kind {
                RequireKind::AtLeast => {
                    requires.at_least(key, json::bound(&first.0).map_err(ConfigError)?)
                }
                RequireKind::AtMost => {
                    requires.at_most(key, json::bound(&first.0).map_err(ConfigError)?)
                }
                RequireKind::Within => {
                    let second = values
                        .get(1)
                        .ok_or_else(|| ConfigError("a requirement without its bounds".into()))?;
                    requires.within(
                        key,
                        json::bound(&first.0).map_err(ConfigError)?,
                        json::bound(&second.0).map_err(ConfigError)?,
                    )
                }
                RequireKind::Equals => requires.equals(
                    key,
                    json::prop_value(&program.types, first.1, &first.0).map_err(ConfigError)?,
                ),
                RequireKind::OneOf => {
                    let mut options = Vec::new();
                    for (value, ty) in &values {
                        options.push(
                            json::prop_value(&program.types, ty, value).map_err(ConfigError)?,
                        );
                    }
                    requires.one_of(key, options)
                }
            };
        }
        Ok(Self {
            program,
            config,
            props,
            requires,
            fault: FaultHandle::default(),
        })
    }

    /// The module.
    pub fn program(&self) -> &Arc<Program> {
        &self.program
    }

    /// The configuration as JSON, defaults filled in.
    pub fn config(&self) -> Json {
        json::to_json(
            &self.program.types,
            &crate::types::Ty::Named(self.program.config_ty.clone()),
            &self.config,
        )
    }

    /// The properties the output declares for this configuration.
    pub fn props(&self) -> &Props {
        &self.props
    }

    /// What the input requires of the output it reads, for this configuration.
    pub fn requires(&self) -> &Requires {
        &self.requires
    }

    /// Where the decoder reports a fault of the interpreter, once it runs.
    pub fn fault_handle(&self) -> FaultHandle {
        self.fault.clone()
    }

    /// The input of a logic decoder added as `node`.
    pub fn logic_input(node: NodeId) -> InputRef<Arc<LogicBlock>> {
        InputRef::of_node(node, 0)
    }

    /// The input of a stream or bus decoder added as `node`.
    pub fn bus_input(node: NodeId) -> InputRef<DynEvent> {
        InputRef::of_node(node, 0)
    }

    /// The event output of the decoder added as `node`.
    pub fn events(node: NodeId) -> OutputRef<DynEvent> {
        OutputRef::of_node(node, 0)
    }
}

impl Node for Interpreter {
    fn spec(&self) -> NodeSpec {
        let input = match &self.program.input {
            Input::Logic => InputSpec::new::<Arc<LogicBlock>>("logic", self.requires.clone()),
            Input::Stream { schema, .. } | Input::Bus { schema, .. } => {
                InputSpec::with_schema("bus", *schema, self.requires.clone())
            }
        };
        NodeSpec {
            name: self.program.name.clone(),
            inputs: vec![input],
            outputs: vec![OutputSpec::with_schema::<DynEvent>(
                "events",
                self.program.output,
                self.props.clone(),
            )],
        }
    }

    fn start(self: Box<Self>, mut io: Io) -> NodeFuture {
        let rate = io.time_base().map_or(0, |base| base.rate_hz);
        let program = self.program;
        let mut fields = vec![self.config, Value::Int(rate)];
        fields.resize(2 + program.state.len(), Value::Unit);
        let framer = DynFramer::new(Arc::clone(&program));
        let machine = Rc::new(Machine::new(
            Arc::clone(&program),
            fields,
            framer,
            self.fault,
        ));
        // The fields are initialized in order, each with the earlier ones in reach.
        for (index, field) in program.state.iter().enumerate() {
            match machine.execute_sync(field.init, Vec::new()) {
                Ok(value) => machine.set_field(index + 2, value),
                Err(fault) => {
                    machine.record(fault);
                    break;
                }
            }
        }
        match program.kind {
            Kind::Logic => Box::pin(run_logic(LogicDriver(machine), io.signal(0), io.output(0))),
            Kind::Stream => Box::pin(run_stream(StreamDriver(machine), io.input(0), io.output(0))),
            Kind::Bus => Box::pin(run_bus(BusDriver(machine), io.input(0), io.output(0))),
        }
    }
}

// The drivers: the hooks of a kind, as its main loop calls them. The hooks a wait can reach
// from inside the body (progress, through the operation wrappers) are boxed: their futures
// would otherwise contain the body's, which contains theirs.

/// A logic decoder's hooks.
pub(crate) struct LogicDriver(pub(crate) Rc<Machine>);

impl LogicDecoder for LogicDriver {
    type Event = DynEvent;

    fn hold(&self) -> Option<u64> {
        self.0.hold()
    }

    async fn run(&mut self, io: &mut LogicIo<DynEvent>) -> Result<(), Cut> {
        let machine = Rc::clone(&self.0);
        let hook = machine.program.hooks.run;
        let mut cx = Cx::Logic { decoder: self, io };
        machine.hook(&mut cx, hook, Vec::new()).await
    }

    async fn on_gap(&mut self, io: &mut LogicIo<DynEvent>, range: TickRange) {
        let machine = Rc::clone(&self.0);
        let hook = machine.program.hooks.on_gap;
        let mut cx = Cx::Logic { decoder: self, io };
        let _ = Box::pin(machine.hook(&mut cx, hook, vec![Value::Range(range)])).await;
    }

    async fn on_end(&mut self, io: &mut LogicIo<DynEvent>, tick: u64) {
        let machine = Rc::clone(&self.0);
        let hook = machine.program.hooks.on_end;
        let mut cx = Cx::Logic { decoder: self, io };
        let _ = Box::pin(machine.hook(&mut cx, hook, vec![Value::Int(tick)])).await;
    }

    async fn on_progress(&mut self, io: &mut LogicIo<DynEvent>, tick: u64) {
        let machine = Rc::clone(&self.0);
        let hook = machine.program.hooks.on_progress;
        let mut cx = Cx::Logic { decoder: self, io };
        let _ = Box::pin(machine.hook(&mut cx, hook, vec![Value::Int(tick)])).await;
    }
}

/// A stream decoder's hooks.
pub(crate) struct StreamDriver(pub(crate) Rc<Machine>);

impl StreamDecoder for StreamDriver {
    type Lower = DynEvent;
    type Event = DynEvent;

    fn hold(&self) -> Option<u64> {
        self.0.hold()
    }

    async fn run(&mut self, io: &mut StreamIo<DynEvent, DynEvent>) -> Result<(), Cut> {
        let machine = Rc::clone(&self.0);
        let hook = machine.program.hooks.run;
        let mut cx = Cx::Stream { decoder: self, io };
        machine.hook(&mut cx, hook, Vec::new()).await
    }

    async fn on_gap(&mut self, io: &mut StreamIo<DynEvent, DynEvent>, range: TickRange) {
        let machine = Rc::clone(&self.0);
        let hook = machine.program.hooks.on_gap;
        let mut cx = Cx::Stream { decoder: self, io };
        let _ = Box::pin(machine.hook(&mut cx, hook, vec![Value::Range(range)])).await;
    }

    async fn on_end(&mut self, io: &mut StreamIo<DynEvent, DynEvent>, tick: u64) {
        let machine = Rc::clone(&self.0);
        let hook = machine.program.hooks.on_end;
        let mut cx = Cx::Stream { decoder: self, io };
        let _ = Box::pin(machine.hook(&mut cx, hook, vec![Value::Int(tick)])).await;
    }

    async fn on_progress(&mut self, io: &mut StreamIo<DynEvent, DynEvent>, tick: u64) {
        let machine = Rc::clone(&self.0);
        let hook = machine.program.hooks.on_progress;
        let mut cx = Cx::Stream { decoder: self, io };
        let _ = Box::pin(machine.hook(&mut cx, hook, vec![Value::Int(tick)])).await;
    }
}

/// A bus decoder's hooks.
pub(crate) struct BusDriver(pub(crate) Rc<Machine>);

impl BusDecoder for BusDriver {
    type Lower = DynEvent;
    type Framer = DynFramer;
    type Event = DynEvent;

    // Whether a partial frame is drained or read is the module's to say, so the body decides
    // here, before running, rather than the main loop by this constant.
    const PARTIAL: Partial = Partial::Read;

    fn framer(&self) -> DynFramer {
        self.0
            .framer
            .clone()
            .unwrap_or_else(|| unreachable!("a bus decoder has a framer"))
    }

    fn hold(&self) -> Option<u64> {
        self.0.hold()
    }

    async fn transaction(
        &mut self,
        io: &mut BusIo<DynEvent>,
        tx: &mut Transaction<'_, DynEvent, DynFramer>,
    ) -> Result<(), Cut> {
        let machine = Rc::clone(&self.0);
        if tx.is_partial() && machine.program.partial == Partial::Drain {
            return tx.drain().await.map_err(Cut::from);
        }
        let hook = machine.program.hooks.transaction;
        let mut cx = Cx::Bus { io, tx: Some(tx) };
        // The cursor is in reach through the handles; its parameter is a placeholder.
        machine.hook(&mut cx, hook, vec![Value::Unit]).await
    }

    async fn recover(&mut self, io: &mut BusIo<DynEvent>, cut: CutKind, at: Provenance) {
        let machine = Rc::clone(&self.0);
        let Some(recovery) = machine.program.recovery else {
            return;
        };
        let reason = match cut {
            CutKind::Incomplete => recovery.incomplete,
            CutKind::Truncated => recovery.truncated,
            CutKind::Missing => recovery.missing,
            CutKind::Gap => recovery.gap,
            CutKind::End => recovery.end,
        };
        let hook = machine.program.hooks.recover;
        let mut cx = Cx::Bus { io, tx: None };
        let _ = machine
            .hook(
                &mut cx,
                hook,
                vec![Value::Enum(reason), Value::Provenance(at)],
            )
            .await;
    }

    async fn on_gap(&mut self, io: &mut BusIo<DynEvent>, range: TickRange) {
        let machine = Rc::clone(&self.0);
        let hook = machine.program.hooks.on_gap;
        let mut cx = Cx::Bus { io, tx: None };
        let _ = machine.hook(&mut cx, hook, vec![Value::Range(range)]).await;
    }

    async fn on_end(&mut self, io: &mut BusIo<DynEvent>, tick: u64) {
        let machine = Rc::clone(&self.0);
        let hook = machine.program.hooks.on_end;
        let mut cx = Cx::Bus { io, tx: None };
        let _ = machine.hook(&mut cx, hook, vec![Value::Int(tick)]).await;
    }

    async fn on_progress(&mut self, io: &mut BusIo<DynEvent>, tick: u64) {
        let machine = Rc::clone(&self.0);
        let hook = machine.program.hooks.on_progress;
        let mut cx = Cx::Bus { io, tx: None };
        let _ = machine.hook(&mut cx, hook, vec![Value::Int(tick)]).await;
    }
}
