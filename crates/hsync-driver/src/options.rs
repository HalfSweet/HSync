//! Device options described for the UI. Drivers never produce text:
//! choices carry an i18n key and parameters.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A setting of a device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum OptionKey {
    /// Buffer or stream acquisition.
    OperationMode,
    /// Number of usable channels and their maximum sample rate.
    ChannelMode,
    /// Samples per second.
    Samplerate,
    /// Samples per channel to capture.
    SampleLimit,
    /// Fixed input threshold level (1.8/2.5/3.3 V or 5 V).
    Threshold,
    /// Adjustable input threshold voltage.
    Vth,
    /// Glitch filter.
    Filter,
    /// Hardware run-length compression.
    Rle,
    /// Sample on an external clock.
    ExternalClock,
    /// Sample on the falling edge of the external clock.
    ClockFallingEdge,
    /// What on the external trigger input fires the trigger.
    ExternalTrigger,
    /// What a stop in buffer mode does with the samples already captured.
    BufferOption,
    /// Signal pattern of the demo device.
    Pattern,
}

/// The value of an option.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", content = "value", rename_all = "camelCase")]
#[ts(export)]
pub enum OptionValue {
    Bool(bool),
    Integer(u64),
    Number(f64),
    Choice(String),
}

impl OptionValue {
    /// The value of a [`OptionValue::Bool`].
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(value) => Some(*value),
            _ => None,
        }
    }

    /// The value of an [`OptionValue::Integer`].
    pub fn as_integer(&self) -> Option<u64> {
        match self {
            Self::Integer(value) => Some(*value),
            _ => None,
        }
    }

    /// The value of a [`OptionValue::Number`] (or an integer).
    pub fn as_number(&self) -> Option<f64> {
        match self {
            Self::Number(value) => Some(*value),
            Self::Integer(value) => Some(*value as f64),
            _ => None,
        }
    }

    /// The id of a [`OptionValue::Choice`].
    pub fn as_choice(&self) -> Option<&str> {
        match self {
            Self::Choice(id) => Some(id),
            _ => None,
        }
    }
}

/// Text the UI translates: an i18n key under `device.` and its parameters.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct Label {
    pub key: String,
    #[ts(type = "Record<string, string | number>")]
    pub params: BTreeMap<String, serde_json::Value>,
}

impl Label {
    pub fn new(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            params: BTreeMap::new(),
        }
    }

    #[must_use]
    pub fn with(mut self, name: &str, value: impl Into<serde_json::Value>) -> Self {
        self.params.insert(name.to_owned(), value.into());
        self
    }
}

/// One choice of a [`OptionKind::Choice`] option.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct Choice {
    pub id: String,
    pub label: Label,
}

/// What values an option accepts.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
#[ts(export)]
pub enum OptionKind {
    Bool,
    Choice {
        choices: Vec<Choice>,
    },
    /// One of these integers, ascending (sample rates).
    Integers {
        values: Vec<u64>,
    },
    /// A number in a range (threshold voltage, sample limit).
    Range {
        min: f64,
        max: f64,
        step: f64,
    },
}

/// An option as the UI shows it.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct OptionState {
    pub key: OptionKey,
    pub kind: OptionKind,
    pub value: OptionValue,
    /// Options that do not apply in the current mode are shown disabled.
    pub enabled: bool,
}
