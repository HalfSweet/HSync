//! Running a stack of Python decoders over a capture: the decoder a DSView entry names, fed by
//! the capture, and the decoders stacked on it, each fed by the one below.

use std::collections::BTreeMap;
use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use hsync_capture::LogicCapture;
use hsync_core::ChannelIndex;
use hsync_decode::Annotation;
use parking_lot::{Mutex, RwLock};
use pyo3::prelude::*;
use pyo3::types::PyDict;
use serde_json::Value;

use crate::catalog::decoder_class;
use crate::module::{self, Decoder, EndOfData, Instance, SRD_CONF_SAMPLERATE};
use crate::{DecoderHost, DecoderInfo, SrdError};

/// One decoder of a stack and its options as stored.
#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    pub id: String,
    pub options: serde_json::Map<String, Value>,
}

/// A decoder entry as DSView stores it: the decoder, its channels by role, and the decoders
/// stacked on it.
#[derive(Debug, Clone, PartialEq)]
pub struct Stack {
    /// From the bottom (fed by the capture) up.
    pub layers: Vec<Layer>,
    pub channels: BTreeMap<String, ChannelIndex>,
}

impl Stack {
    /// The stack of a DSView `decoders` entry; `None` without an id.
    pub fn from_dsview(entry: &Value) -> Option<Self> {
        let layer = |value: &Value| -> Option<Layer> {
            Some(Layer {
                id: value.get("id")?.as_str()?.to_owned(),
                options: value
                    .get("options")
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_default(),
            })
        };
        let mut layers = vec![layer(entry)?];
        layers.extend(
            entry
                .get("stacked decoders")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(layer),
        );
        let mut channels = BTreeMap::new();
        for item in entry
            .get("channel")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            for (role, index) in item.as_object().into_iter().flatten() {
                if let Some(index) = index
                    .as_u64()
                    .and_then(|index| ChannelIndex::try_from(index).ok())
                {
                    channels.insert(role.clone(), index);
                }
            }
        }
        Some(Self { layers, channels })
    }
}

/// What one decoder of a stack put out.
#[derive(Debug, Clone, PartialEq)]
pub struct LayerResult {
    pub id: String,
    /// Its annotation rows, `(id, description)`; one row for all its classes when it declares
    /// none.
    pub rows: Vec<(String, String)>,
    pub annotations: Vec<Annotation>,
    /// The error a stacked decoder raised; the decoder below it carried on (DSView).
    pub error: Option<String>,
}

/// The rows of a decoder and the row of each of its annotation classes, as DSView's decoder
/// stack sorts them (`decoderstack.cpp`): without annotation rows a single row named after the
/// decoder takes every class; with rows, a class goes to the last row listing it, and a class
/// no row lists is not shown.
fn rows_of(info: &DecoderInfo) -> (Vec<(String, String)>, Vec<Option<u32>>) {
    if info.annotation_rows.is_empty() {
        let rows = vec![(info.id.clone(), info.name.clone())];
        return (rows, vec![Some(0); info.annotations.len()]);
    }
    let rows = info
        .annotation_rows
        .iter()
        .map(|row| (row.id.clone(), row.desc.clone()))
        .collect();
    let mut classes = vec![None; info.annotations.len()];
    for (row, entry) in info.annotation_rows.iter().enumerate() {
        for &class in &entry.classes {
            if let Some(slot) = classes.get_mut(class as usize) {
                *slot = Some(row as u32);
            }
        }
    }
    (rows, classes)
}

/// `self.options`: each option's default, replaced by the stored value converted to the
/// default's type.
fn options<'py>(
    py: Python<'py>,
    info: &DecoderInfo,
    stored: &serde_json::Map<String, Value>,
) -> PyResult<Bound<'py, PyDict>> {
    let dict = PyDict::new(py);
    for option in &info.options {
        let value = stored.get(&option.id).unwrap_or(&option.default);
        match &option.default {
            Value::Number(default) if default.is_i64() || default.is_u64() => {
                let number = match value {
                    Value::Number(number) => number.as_f64(),
                    Value::String(text) => text.trim().parse().ok(),
                    _ => None,
                }
                .unwrap_or_else(|| default.as_f64().unwrap_or(0.0));
                dict.set_item(&option.id, number.round() as i64)?;
            }
            Value::Number(default) => {
                let number = match value {
                    Value::Number(number) => number.as_f64(),
                    Value::String(text) => text.trim().parse().ok(),
                    _ => None,
                }
                .unwrap_or_else(|| default.as_f64().unwrap_or(0.0));
                dict.set_item(&option.id, number)?;
            }
            _ => {
                let text = match value {
                    Value::String(text) => text.clone(),
                    Value::Null => String::new(),
                    other => other.to_string(),
                };
                dict.set_item(&option.id, text)?;
            }
        }
    }
    Ok(dict)
}

/// What a run of a stack gave: every decoder's annotations, and why it stopped early if it did.
#[derive(Debug)]
pub struct StackOutcome {
    pub layers: Vec<LayerResult>,
    /// The error a decoder raised; the annotations up to it are kept.
    pub error: Option<SrdError>,
}

/// The message of a Python error, with its traceback for the log.
fn describe_error(py: Python<'_>, error: &PyErr) -> (String, String) {
    let message = error.to_string();
    let detail = match error.traceback(py).map(|trace| trace.format()) {
        Some(Ok(trace)) => format!("{trace}{message}"),
        _ => message.clone(),
    };
    (message, detail)
}

/// Runs `stack` over `samples` of `capture` (DSView's decode start and end), as
/// `srd_session_send` does from the start sample on. `cancel` stops it at the next wait.
pub fn run_stack(
    host: &DecoderHost,
    capture: &Arc<RwLock<LogicCapture>>,
    samples: Range<u64>,
    stack: &Stack,
    cancel: &Arc<AtomicBool>,
) -> Result<StackOutcome, SrdError> {
    let infos: Vec<DecoderInfo> = stack
        .layers
        .iter()
        .map(|layer| host.info(&layer.id).cloned())
        .collect::<Result<_, _>>()?;
    let Some(bottom_info) = infos.first() else {
        return Ok(StackOutcome {
            layers: Vec::new(),
            error: None,
        });
    };
    if let Some(missing) = bottom_info
        .channels
        .iter()
        .find(|channel| !stack.channels.contains_key(&channel.id))
    {
        return Err(SrdError::InvalidDecoder {
            decoder: bottom_info.id.clone(),
            detail: format!("a required channel is not given: {}", missing.id),
        });
    }
    let samplerate = capture.read().samplerate();
    Python::attach(|py| {
        module::install(py, host.dir())?;
        let sinks: Vec<Arc<Mutex<Vec<Annotation>>>> = infos
            .iter()
            .map(|_| Arc::new(Mutex::new(Vec::new())))
            .collect();
        // Build from the top down so each decoder knows the one above it.
        let mut upper: Option<Py<PyAny>> = None;
        let mut instances = Vec::new();
        for (position, (layer, info)) in stack.layers.iter().zip(&infos).enumerate().rev() {
            let class = decoder_class(py, &info.package)?;
            let object = class.call0()?;
            let slots = if position == 0 {
                info.channels
                    .iter()
                    .chain(&info.optional_channels)
                    .map(|channel| stack.channels.get(&channel.id).copied())
                    .collect()
            } else {
                Vec::new()
            };
            let (_, class_rows) = rows_of(info);
            {
                let base = object
                    .cast::<Decoder>()
                    .map_err(|_| SrdError::InvalidDecoder {
                        decoder: info.id.clone(),
                        detail: "its Decoder does not derive from sigrokdecode.Decoder".into(),
                    })?;
                *base.borrow().instance.lock() = Some(Instance {
                    capture: Arc::clone(capture),
                    first: samples.start,
                    end: samples.end,
                    slots,
                    current: None,
                    outputs: Vec::new(),
                    class_rows,
                    annotations: Arc::clone(&sinks[position]),
                    upper: upper.take(),
                    upper_error: None,
                    cancel: Arc::clone(cancel),
                });
            }
            object.setattr("options", options(py, info, &layer.options)?)?;
            upper = Some(object.clone().unbind());
            instances.push(object);
        }
        // `instances` runs from the top down; DSView starts them from the bottom up and sets
        // samplenum and matched after start().
        instances.reverse();
        for object in &instances {
            if object.hasattr("metadata")? {
                object.call_method1("metadata", (SRD_CONF_SAMPLERATE, samplerate))?;
            }
            object.call_method0("start")?;
            object.setattr("samplenum", 0)?;
            object.setattr("matched", 0)?;
        }
        let bottom = &instances[0];
        let mut error = match bottom.call_method0("decode") {
            // decode() is meant to run until the samples run out; returning is an error.
            Ok(returned) => Some(
                returned
                    .extract::<String>()
                    .unwrap_or_else(|_| "decode() returned".into()),
            ),
            Err(error) if error.is_instance_of::<EndOfData>(py) => None,
            Err(error) => {
                let (message, detail) = describe_error(py, &error);
                log::warn!("decoder {} failed: {detail}", bottom_info.id);
                Some(message)
            }
        };
        // DSView's end() hooks, after a complete run: the bottom decoder learns where the
        // samples ended, then each decoder above it in turn.
        if error.is_none() && !cancel.load(std::sync::atomic::Ordering::Relaxed) {
            bottom.setattr("last_samplenum", samples.end)?;
            for object in &instances {
                if !object.hasattr("end")? {
                    continue;
                }
                if let Err(failure) = object.call_method0("end") {
                    let (message, detail) = describe_error(py, &failure);
                    log::warn!("end() of a decoder failed: {detail}");
                    error = Some(message);
                    break;
                }
            }
        }
        // A decoder's upper_error tells of the decoder above it.
        let mut upper_errors = vec![None; instances.len()];
        for (position, object) in instances.iter().enumerate() {
            if let Ok(base) = object.cast::<Decoder>()
                && let Some(instance) = base.borrow().instance.lock().as_mut()
                && position + 1 < upper_errors.len()
            {
                upper_errors[position + 1] = instance.upper_error.take();
            }
        }
        let layers = stack
            .layers
            .iter()
            .zip(&infos)
            .zip(&sinks)
            .zip(upper_errors)
            .map(|(((layer, info), sink), error)| LayerResult {
                id: layer.id.clone(),
                rows: rows_of(info).0,
                annotations: std::mem::take(&mut *sink.lock()),
                error,
            })
            .collect();
        Ok(StackOutcome {
            layers,
            error: error.map(SrdError::Python),
        })
    })
}
