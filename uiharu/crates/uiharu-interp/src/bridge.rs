// SPDX-License-Identifier: Apache-2.0

//! Bridges between a stream of a contract's Rust type and the same contract as an interpreted
//! decoder reads or writes it ([`DynEvent`]): a node each way, passing every item on and
//! converting the events through their JSON form.

use std::marker::PhantomData;

use serde::Serialize;
use serde::de::DeserializeOwned;
use uiharu_core::{
    EventSchema, InputSpec, Io, Item, Node, NodeFuture, NodeSpec, OutputSpec, Props, Provenance,
    Requires,
};

use crate::event::DynEvent;
use crate::vm::FaultHandle;

/// From a contract's Rust type to [`DynEvent`]: for a compiled decoder's output read by an
/// interpreted decoder. The output declares `props`, the properties of the output bridged.
pub struct ToDyn<T> {
    props: Props,
    _payload: PhantomData<fn() -> T>,
}

impl<T> std::fmt::Debug for ToDyn<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToDyn").field("props", &self.props).finish()
    }
}

impl<T> ToDyn<T> {
    /// A bridge declaring `props` on its output.
    pub fn new(props: Props) -> Self {
        Self {
            props,
            _payload: PhantomData,
        }
    }
}

impl<T: Serialize + EventSchema + Clone + 'static> Node for ToDyn<T> {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            name: "to-dyn".into(),
            inputs: vec![InputSpec::new::<T>("in", Requires::new())],
            outputs: vec![OutputSpec::with_schema::<DynEvent>(
                "out",
                T::SCHEMA,
                self.props.clone(),
            )],
        }
    }

    fn start(self: Box<Self>, mut io: Io) -> NodeFuture {
        let mut input = io.input::<T>(0);
        let mut out = io.output::<DynEvent>(0);
        Box::pin(async move {
            loop {
                match input.recv().await {
                    Item::Event(event) => {
                        // A contract's events are plain data; serializing them cannot fail.
                        let payload =
                            serde_json::to_value(&event.payload).unwrap_or(serde_json::Value::Null);
                        out.emit(
                            Provenance::over(event.span, event.settled),
                            DynEvent(payload),
                        )
                        .await;
                    }
                    Item::Gap(range) => out.gap(range).await,
                    Item::Progress(tick) => out.progress(tick).await,
                    Item::End(tick) => {
                        out.end(tick).await;
                        return;
                    }
                }
            }
        })
    }
}

/// From [`DynEvent`] to a contract's Rust type: for an interpreted decoder's output read by a
/// compiled decoder. An event that does not fit the type is dropped and reported through the
/// bridge's [`FaultHandle`].
pub struct FromDyn<T> {
    props: Props,
    fault: FaultHandle,
    _payload: PhantomData<fn() -> T>,
}

impl<T> std::fmt::Debug for FromDyn<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FromDyn")
            .field("props", &self.props)
            .finish()
    }
}

impl<T> FromDyn<T> {
    /// A bridge declaring `props` on its output.
    pub fn new(props: Props) -> Self {
        Self {
            props,
            fault: FaultHandle::default(),
            _payload: PhantomData,
        }
    }

    /// Where the bridge reports an event that did not fit the type.
    pub fn fault_handle(&self) -> FaultHandle {
        self.fault.clone()
    }
}

impl<T: DeserializeOwned + EventSchema + 'static> Node for FromDyn<T> {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            name: "from-dyn".into(),
            inputs: vec![InputSpec::with_schema("in", T::SCHEMA, Requires::new())],
            outputs: vec![OutputSpec::new::<T>("out", self.props.clone())],
        }
    }

    fn start(self: Box<Self>, mut io: Io) -> NodeFuture {
        let mut input = io.input::<DynEvent>(0);
        let mut out = io.output::<T>(0);
        let fault = self.fault;
        Box::pin(async move {
            loop {
                match input.recv().await {
                    Item::Event(event) => match serde_json::from_value::<T>(event.payload.0) {
                        Ok(payload) => {
                            out.emit(Provenance::over(event.span, event.settled), payload)
                                .await;
                        }
                        Err(error) => {
                            fault.report(format!("an event does not fit {}: {error}", T::SCHEMA))
                        }
                    },
                    Item::Gap(range) => out.gap(range).await,
                    Item::Progress(tick) => out.progress(tick).await,
                    Item::End(tick) => {
                        out.end(tick).await;
                        return;
                    }
                }
            }
        })
    }
}
