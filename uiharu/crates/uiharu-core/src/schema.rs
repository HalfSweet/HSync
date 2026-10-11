// SPDX-License-Identifier: Apache-2.0

//! What ports carry and what they require of each other: the identity of an event schema, the
//! properties an output declares (word width, bit order, which lines are present …) and the
//! requirements an input places on them. The graph checks them when ports are connected.

use std::collections::BTreeMap;
use std::fmt;

use serde::Serialize;

use crate::stream::{QueueHandles, make_queue};

/// The name and version of an event schema, such as `spi.events` version 1. Two ports are
/// compatible only when both agree.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub struct SchemaId {
    /// The schema's name, dotted and lower case.
    pub name: &'static str,
    /// Its version; a new version is a different contract.
    pub version: u32,
}

impl SchemaId {
    /// The schema `name` at `version`.
    pub const fn new(name: &'static str, version: u32) -> Self {
        Self { name, version }
    }
}

impl fmt::Display for SchemaId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.name, self.version)
    }
}

/// The payload type of an event stream, tied to its schema.
pub trait EventSchema: 'static {
    /// The schema this type implements.
    const SCHEMA: SchemaId;
}

/// A property of an output port.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PropValue {
    /// A yes or no, such as whether a chip select line is present.
    Bool(bool),
    /// A count or width, such as the bits per word.
    Int(i64),
    /// A name, such as a bit order.
    Text(String),
}

impl fmt::Display for PropValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PropValue::Bool(value) => write!(f, "{value}"),
            PropValue::Int(value) => write!(f, "{value}"),
            PropValue::Text(value) => write!(f, "{value:?}"),
        }
    }
}

impl From<bool> for PropValue {
    fn from(value: bool) -> Self {
        PropValue::Bool(value)
    }
}

impl From<i64> for PropValue {
    fn from(value: i64) -> Self {
        PropValue::Int(value)
    }
}

impl From<&str> for PropValue {
    fn from(value: &str) -> Self {
        PropValue::Text(value.to_owned())
    }
}

/// The properties of an output port, by name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Props(BTreeMap<&'static str, PropValue>);

impl Props {
    /// No properties.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets `key` to `value`, replacing an earlier value.
    pub fn with(mut self, key: &'static str, value: impl Into<PropValue>) -> Self {
        self.0.insert(key, value.into());
        self
    }

    /// The value of `key`, if declared.
    pub fn get(&self, key: &str) -> Option<&PropValue> {
        self.0.get(key)
    }

    /// The properties in name order.
    pub fn iter(&self) -> impl Iterator<Item = (&'static str, &PropValue)> {
        self.0.iter().map(|(key, value)| (*key, value))
    }
}

/// What an input requires of one property of the output it is connected to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Requirement {
    /// The property must be declared with exactly this value.
    Equals(PropValue),
    /// The property must be an integer of at least this value.
    AtLeast(i64),
}

impl Requirement {
    /// Whether `value` (the output's property, if declared) satisfies the requirement.
    pub fn accepts(&self, value: Option<&PropValue>) -> bool {
        match (self, value) {
            (Requirement::Equals(wanted), Some(found)) => wanted == found,
            (Requirement::AtLeast(least), Some(PropValue::Int(found))) => found >= least,
            _ => false,
        }
    }
}

impl fmt::Display for Requirement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Requirement::Equals(value) => write!(f, "= {value}"),
            Requirement::AtLeast(least) => write!(f, ">= {least}"),
        }
    }
}

/// The requirements of an input port, by property name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Requires(BTreeMap<&'static str, Requirement>);

impl Requires {
    /// No requirements.
    pub fn new() -> Self {
        Self::default()
    }

    /// Requires `key` to equal `value`.
    pub fn equals(mut self, key: &'static str, value: impl Into<PropValue>) -> Self {
        self.0.insert(key, Requirement::Equals(value.into()));
        self
    }

    /// Requires `key` to be an integer of at least `least`.
    pub fn at_least(mut self, key: &'static str, least: i64) -> Self {
        self.0.insert(key, Requirement::AtLeast(least));
        self
    }

    /// The requirements in name order.
    pub fn iter(&self) -> impl Iterator<Item = (&'static str, &Requirement)> {
        self.0.iter().map(|(key, requirement)| (*key, requirement))
    }

    /// The first requirement `props` does not satisfy, if any.
    pub fn unmet<'a>(&'a self, props: &Props) -> Option<(&'static str, &'a Requirement)> {
        self.iter()
            .find(|(key, requirement)| !requirement.accepts(props.get(key)))
    }
}

/// An output port as a node declares it.
#[derive(Clone, Debug)]
pub struct OutputSpec {
    /// The port's name within its node.
    pub name: &'static str,
    /// The schema of the events it carries.
    pub schema: SchemaId,
    /// What the node promises about the events.
    pub props: Props,
    /// Makes the queue behind the port, typed for its payload.
    factory: fn(usize) -> QueueHandles,
}

impl OutputSpec {
    /// A port named `name` carrying `T` events with `props`.
    pub fn new<T: EventSchema>(name: &'static str, props: Props) -> Self {
        Self {
            name,
            schema: T::SCHEMA,
            props,
            factory: make_queue::<T>,
        }
    }

    pub(crate) fn make_queue(&self, capacity: usize) -> QueueHandles {
        (self.factory)(capacity)
    }
}

/// An input port as a node declares it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputSpec {
    /// The port's name within its node.
    pub name: &'static str,
    /// The schema of the events it reads.
    pub schema: SchemaId,
    /// What it requires of the output connected to it.
    pub requires: Requires,
}

impl InputSpec {
    /// A port named `name` reading `T` events from an output that satisfies `requires`.
    pub fn new<T: EventSchema>(name: &'static str, requires: Requires) -> Self {
        Self {
            name,
            schema: T::SCHEMA,
            requires,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requirements_check_values() {
        let props = Props::new()
            .with("wordBits", 8)
            .with("cs", true)
            .with("bitOrder", "msb-first");
        let requires = Requires::new()
            .equals("wordBits", 8)
            .equals("cs", true)
            .equals("bitOrder", "msb-first")
            .at_least("channels", 1);
        assert_eq!(requires.unmet(&props).map(|(key, _)| key), Some("channels"));
        let props = props.with("channels", 4);
        assert!(requires.unmet(&props).is_none());
        let narrower = Requires::new().equals("wordBits", 16);
        assert_eq!(narrower.unmet(&props).map(|(key, _)| key), Some("wordBits"));
    }

    #[test]
    fn at_least_needs_an_integer() {
        assert!(!Requirement::AtLeast(1).accepts(Some(&PropValue::Bool(true))));
        assert!(!Requirement::AtLeast(1).accepts(None));
        assert!(Requirement::AtLeast(1).accepts(Some(&PropValue::Int(1))));
    }
}
