//! The `sigrokdecode` module decoders import, written in Rust: the `Decoder` base class whose
//! `wait()` runs on `hsync-decode`'s wait engine and whose `put()` collects annotations or feeds
//! the decoder stacked above.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use hsync_capture::LogicCapture;
use hsync_core::ChannelIndex;
use hsync_decode::Annotation;
use hsync_decode::sampler::{Condition, Pin, Sampler, Term};
use parking_lot::{Mutex, RwLock};
use pyo3::exceptions::{PyBaseException, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyModule, PyTuple};

/// `OUTPUT_ANN` and the other output types of `register()`.
pub const OUTPUT_ANN: i32 = 0;
pub const OUTPUT_PYTHON: i32 = 1;
pub const OUTPUT_BINARY: i32 = 2;
pub const OUTPUT_META: i32 = 3;
/// The metadata key of the sample rate (`SRD_CONF_SAMPLERATE`).
pub const SRD_CONF_SAMPLERATE: i32 = 10_000;
/// What `wait()` returns for a channel the decoder has not been given.
const UNCONNECTED: u8 = 0xff;

pyo3::create_exception!(
    sigrokdecode,
    EndOfData,
    PyBaseException,
    "The samples ran out; ends decode()."
);

/// What a decoder instance works on while it runs.
#[derive(Debug)]
pub(crate) struct Instance {
    pub capture: Arc<RwLock<LogicCapture>>,
    /// Samples to decode.
    pub end: u64,
    /// The capture channel of each decoder channel (required, then optional), if given.
    pub slots: Vec<Option<ChannelIndex>>,
    /// The last match.
    pub current: Option<u64>,
    /// Output types by the id `register()` returned.
    pub outputs: Vec<i32>,
    /// The row of each annotation class.
    pub class_rows: Vec<u32>,
    /// Where this decoder's annotations go.
    pub annotations: Arc<Mutex<Vec<Annotation>>>,
    /// The decoder stacked above, fed by `OUTPUT_PYTHON`.
    pub upper: Option<Py<PyAny>>,
    pub cancel: Arc<AtomicBool>,
}

/// The base class of every decoder.
#[pyclass(subclass, module = "sigrokdecode", name = "Decoder")]
#[derive(Debug, Default)]
pub struct Decoder {
    pub(crate) instance: Mutex<Option<Instance>>,
}

/// `'l'`, `'h'`, … as a pin condition.
fn pin(code: &str) -> PyResult<Pin> {
    Ok(match code {
        "l" => Pin::Low,
        "h" => Pin::High,
        "r" => Pin::Rising,
        "f" => Pin::Falling,
        "e" => Pin::Either,
        "s" => Pin::Stable,
        other => {
            return Err(PyValueError::new_err(format!(
                "unknown wait condition {other:?}"
            )));
        }
    })
}

/// A condition of `wait()`: its terms, each with the decoder channel it reads, if any.
type Wanted = Vec<(Option<usize>, Term)>;

/// The conditions of a `wait()` call: `None`, a dict or a list of dicts.
fn conditions(conds: Option<&Bound<'_, PyAny>>) -> PyResult<Vec<Wanted>> {
    let Some(conds) = conds.filter(|conds| !conds.is_none()) else {
        return Ok(Vec::new());
    };
    let dicts: Vec<Bound<'_, PyDict>> = if let Ok(dict) = conds.cast::<PyDict>() {
        vec![dict.clone()]
    } else if let Ok(list) = conds.cast::<PyList>() {
        list.iter()
            .map(|item| {
                item.cast::<PyDict>()
                    .cloned()
                    .map_err(|_| PyTypeError::new_err("wait() conditions must be dicts"))
            })
            .collect::<PyResult<_>>()?
    } else if let Ok(tuple) = conds.cast::<PyTuple>() {
        tuple
            .iter()
            .map(|item| {
                item.cast::<PyDict>()
                    .cloned()
                    .map_err(|_| PyTypeError::new_err("wait() conditions must be dicts"))
            })
            .collect::<PyResult<_>>()?
    } else {
        return Err(PyTypeError::new_err(
            "wait() takes a dict or a list of dicts",
        ));
    };
    dicts
        .iter()
        .map(|dict| {
            dict.iter()
                .map(|(key, value)| {
                    if let Ok(name) = key.extract::<String>() {
                        if name == "skip" {
                            let count: u64 = value.extract()?;
                            return Ok((None, Term::Skip(count)));
                        }
                        return Err(PyValueError::new_err(format!("unknown wait key {name:?}")));
                    }
                    let channel: usize = key.extract()?;
                    let code: String = value.extract()?;
                    Ok((Some(channel), Term::Pin(channel, pin(&code)?)))
                })
                .collect()
        })
        .collect()
}

impl Decoder {
    fn with_instance<T>(&self, work: impl FnOnce(&mut Instance) -> PyResult<T>) -> PyResult<T> {
        let mut guard = self.instance.lock();
        let instance = guard
            .as_mut()
            .ok_or_else(|| PyValueError::new_err("the decoder is not running"))?;
        work(instance)
    }
}

#[pymethods]
impl Decoder {
    #[new]
    #[pyo3(signature = (*_args, **_kwargs))]
    fn new(_args: &Bound<'_, PyTuple>, _kwargs: Option<&Bound<'_, PyDict>>) -> Self {
        Self::default()
    }

    /// Waits until one of the conditions holds and returns every channel's level there.
    #[pyo3(signature = (conds = None))]
    fn wait<'py>(
        slf: &Bound<'py, Self>,
        conds: Option<&Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyTuple>> {
        let py = slf.py();
        let wanted = conditions(conds)?;
        let this = slf.borrow();
        let (sample, matched, pins) = this.with_instance(|instance| {
            if instance.cancel.load(Ordering::Relaxed) {
                return Err(EndOfData::new_err("cancelled"));
            }
            let capture = instance.capture.read();
            // Only the channels the decoder was given take part; a condition on another
            // channel can never hold.
            let mut sampler_slots = Vec::new();
            let mut channels = Vec::new();
            for (slot, channel) in instance.slots.iter().enumerate() {
                if let Some(data) = channel.and_then(|index| capture.channel(index)) {
                    sampler_slots.push(slot);
                    channels.push(data);
                }
            }
            let to_sampler = |slot: usize| sampler_slots.iter().position(|&s| s == slot);
            let mut kept = Vec::new();
            let mut positions = Vec::new();
            for (position, condition) in wanted.iter().enumerate() {
                let terms: Option<Condition> = condition
                    .iter()
                    .map(|(slot, term)| match (slot, term) {
                        (Some(slot), Term::Pin(_, pin)) => {
                            to_sampler(*slot).map(|s| Term::Pin(s, *pin))
                        }
                        (_, term) => Some(*term),
                    })
                    .collect();
                if let Some(terms) = terms {
                    kept.push(terms);
                    positions.push(position);
                }
            }
            let mut sampler = Sampler::new(channels, instance.end).resume(instance.current);
            let found = if !wanted.is_empty() && kept.is_empty() {
                None
            } else {
                sampler.wait(&kept)
            };
            instance.current = Some(sampler.samplenum());
            let Some(found) = found else {
                return Err(EndOfData::new_err("no more samples"));
            };
            // DSView's decoders read `self.matched` as a mask: bit i for condition i.
            let mut matched = 0u64;
            for (index, &position) in positions.iter().enumerate() {
                if found.has(index) {
                    matched |= 1 << position;
                }
            }
            let pins: Vec<u8> = instance
                .slots
                .iter()
                .map(
                    |channel| match channel.and_then(|index| capture.channel(index)) {
                        Some(data) if found.sample < data.len() => {
                            u8::from(data.sample(found.sample))
                        }
                        _ => UNCONNECTED,
                    },
                )
                .collect();
            Ok((found.sample, matched, pins))
        })?;
        drop(this);
        slf.setattr("samplenum", sample)?;
        slf.setattr("matched", matched)?;
        PyTuple::new(py, pins)
    }

    /// Asks for an output of `output_type`; returns its id for `put()`.
    #[pyo3(signature = (output_type, proto_id = None, meta = None))]
    fn register(
        &self,
        output_type: i32,
        proto_id: Option<&Bound<'_, PyAny>>,
        meta: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<usize> {
        let _ = (proto_id, meta);
        self.with_instance(|instance| {
            instance.outputs.push(output_type);
            Ok(instance.outputs.len() - 1)
        })
    }

    /// Whether decoder channel `index` was given a capture channel.
    fn has_channel(&self, index: usize) -> PyResult<bool> {
        self.with_instance(|instance| Ok(instance.slots.get(index).is_some_and(Option::is_some)))
    }

    /// Puts out `data` for samples `ss..es` on `output_id`.
    fn put(&self, ss: u64, es: u64, output_id: usize, data: &Bound<'_, PyAny>) -> PyResult<()> {
        let (kind, upper) = self.with_instance(|instance| {
            let kind = *instance
                .outputs
                .get(output_id)
                .ok_or_else(|| PyValueError::new_err(format!("unknown output {output_id}")))?;
            Ok((
                kind,
                instance
                    .upper
                    .as_ref()
                    .map(|upper| upper.clone_ref(data.py())),
            ))
        })?;
        match kind {
            OUTPUT_ANN => {
                let list = data
                    .cast::<PyList>()
                    .map_err(|_| PyTypeError::new_err("annotations are [class, [texts]]"))?;
                if list.len() != 2 {
                    return Err(PyTypeError::new_err("annotations are [class, [texts]]"));
                }
                let class: u32 = list.get_item(0)?.extract()?;
                let texts: Vec<String> = list
                    .get_item(1)?
                    .try_iter()?
                    .map(|item| {
                        let item = item?;
                        Ok(match item.extract::<String>() {
                            Ok(text) => text,
                            Err(_) => item.str()?.to_string(),
                        })
                    })
                    .collect::<PyResult<_>>()?;
                self.with_instance(|instance| {
                    let row = instance
                        .class_rows
                        .get(class as usize)
                        .copied()
                        .unwrap_or(0);
                    instance.annotations.lock().push(Annotation {
                        start: ss,
                        end: es,
                        class,
                        row,
                        texts,
                    });
                    Ok(())
                })
            }
            OUTPUT_PYTHON => {
                if let Some(upper) = upper {
                    upper.call_method1(data.py(), "decode", (ss, es, data))?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

/// Puts `sigrokdecode` in `sys.modules` and `dir` on the import path, once.
pub fn install(py: Python<'_>, dir: &Path) -> PyResult<()> {
    let sys = py.import("sys")?;
    // The decoders live among the application's resources: leave no bytecode beside them, and
    // keep the old escape-sequence warnings of some of them quiet.
    sys.setattr("dont_write_bytecode", true)?;
    let filter = PyDict::new(py);
    filter.set_item(
        "category",
        py.get_type::<pyo3::exceptions::PySyntaxWarning>(),
    )?;
    py.import("warnings")?
        .call_method("filterwarnings", ("ignore",), Some(&filter))?;
    let modules = sys.getattr("modules")?;
    if !modules.contains("sigrokdecode")? {
        let module = PyModule::new(py, "sigrokdecode")?;
        module.add_class::<Decoder>()?;
        module.add("EndOfData", py.get_type::<EndOfData>())?;
        module.add("OUTPUT_ANN", OUTPUT_ANN)?;
        module.add("OUTPUT_PYTHON", OUTPUT_PYTHON)?;
        module.add("OUTPUT_BINARY", OUTPUT_BINARY)?;
        module.add("OUTPUT_META", OUTPUT_META)?;
        module.add("SRD_CONF_SAMPLERATE", SRD_CONF_SAMPLERATE)?;
        modules.set_item("sigrokdecode", module)?;
    }
    let path = sys.getattr("path")?;
    let dir = dir.to_string_lossy().into_owned();
    if !path.contains(&dir)? {
        path.call_method1("insert", (0, dir))?;
    }
    Ok(())
}
