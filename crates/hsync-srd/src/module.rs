//! The `sigrokdecode` module decoders import, written in Rust after libsigrokdecode4DSL: the
//! `Decoder` base class whose `wait()` runs on `hsync-decode`'s wait engine and whose `put()`
//! collects annotations or feeds the decoder stacked above.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use hsync_capture::LogicCapture;
use hsync_core::ChannelIndex;
use hsync_decode::Annotation;
use hsync_decode::sampler::{Condition, Pin, Sampler, Term};
use parking_lot::{Mutex, RwLock};
use pyo3::exceptions::{PyBaseException, PyIndexError, PyTypeError, PyValueError};
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
    "The samples ran out; ends decode(). A BaseException, so `except Exception` lets it pass."
);

/// What a decoder instance works on while it runs.
#[derive(Debug)]
pub(crate) struct Instance {
    pub capture: Arc<RwLock<LogicCapture>>,
    /// Samples to decode.
    pub end: u64,
    /// The capture channel of each decoder channel (required, then optional), if given; empty
    /// for a stacked decoder.
    pub slots: Vec<Option<ChannelIndex>>,
    /// The last match.
    pub current: Option<u64>,
    /// Output types and protocol ids by the id `register()` returned.
    pub outputs: Vec<(i32, String)>,
    /// The row of each annotation class; classes in no row are not shown.
    pub class_rows: Vec<Option<u32>>,
    /// Where this decoder's annotations go.
    pub annotations: Arc<Mutex<Vec<Annotation>>>,
    /// The decoder stacked above, fed by `OUTPUT_PYTHON`.
    pub upper: Option<Py<PyAny>>,
    /// The first error of the decoder above, which does not stop this one.
    pub upper_error: Option<String>,
    pub cancel: Arc<AtomicBool>,
}

/// The base class of every decoder.
#[pyclass(subclass, module = "sigrokdecode", name = "Decoder")]
#[derive(Debug, Default)]
pub struct Decoder {
    pub(crate) instance: Mutex<Option<Instance>>,
}

/// DSView's condition letters (only the first character counts): `h`, `l`, `r`, `f`, `e`, and
/// `n` for no edge. Anything else never holds.
fn pin(code: &str) -> Option<Pin> {
    Some(match code.chars().next()? {
        'l' => Pin::Low,
        'h' => Pin::High,
        'r' => Pin::Rising,
        'f' => Pin::Falling,
        'e' => Pin::Either,
        'n' => Pin::Stable,
        _ => return None,
    })
}

/// A condition of `wait()`: its terms, each with the decoder channel it reads, if any.
type Wanted = Vec<(Option<usize>, Term)>;

fn dicts<'py>(conds: &Bound<'py, PyAny>) -> PyResult<Vec<Bound<'py, PyDict>>> {
    let all = |items: Vec<Bound<'py, PyAny>>| {
        items
            .into_iter()
            .map(|item| {
                item.cast::<PyDict>()
                    .cloned()
                    .map_err(|_| PyTypeError::new_err("wait() conditions must be dicts"))
            })
            .collect()
    };
    if let Ok(dict) = conds.cast::<PyDict>() {
        Ok(vec![dict.clone()])
    } else if let Ok(list) = conds.cast::<PyList>() {
        all(list.iter().collect())
    } else if let Ok(tuple) = conds.cast::<PyTuple>() {
        all(tuple.iter().collect())
    } else {
        Err(PyTypeError::new_err(
            "wait() takes a dict or a list of dicts",
        ))
    }
}

/// The conditions of a `wait()` call: `None`, a dict or a list of dicts; `None`, `[]` and `{}`
/// wait for nothing.
fn conditions(conds: Option<&Bound<'_, PyAny>>) -> PyResult<Vec<Wanted>> {
    let Some(conds) = conds.filter(|conds| !conds.is_none()) else {
        return Ok(Vec::new());
    };
    let dicts = dicts(conds)?;
    if dicts.len() == 1 && dicts[0].is_empty() {
        return Ok(Vec::new());
    }
    dicts
        .iter()
        .map(|dict| {
            dict.iter()
                .map(|(key, value)| {
                    // DSView takes any string key as a skip.
                    if key.extract::<String>().is_ok() {
                        let count: u64 = value
                            .extract()
                            .map_err(|_| PyValueError::new_err("skip takes a count of samples"))?;
                        return Ok((None, Term::Skip(count)));
                    }
                    let channel: usize = key.extract()?;
                    let code: String = value.extract()?;
                    let term = pin(&code).map_or(Term::Const(false), |pin| Term::Pin(channel, pin));
                    Ok((Some(channel), term))
                })
                .collect()
        })
        .collect()
}

/// Runs one wait over the capture: the matched sample, DSView's `matched` mask and the levels
/// there, or `None` when the samples ran out or the run was cancelled.
fn scan(instance: &mut Instance, wanted: &[Wanted]) -> Option<(u64, u64, Vec<u8>)> {
    if instance.cancel.load(Ordering::Relaxed) {
        return None;
    }
    let capture = instance.capture.read();
    // The channels the decoder was given take part; the others read as constant low.
    let mut sampler_slots = Vec::new();
    let mut channels = Vec::new();
    for (slot, channel) in instance.slots.iter().enumerate() {
        if let Some(data) = channel.and_then(|index| capture.channel(index)) {
            sampler_slots.push(slot);
            channels.push(data);
        }
    }
    let to_sampler = |slot: usize| sampler_slots.iter().position(|&s| s == slot);
    let terms: Vec<Condition> = wanted
        .iter()
        .map(|condition| {
            condition
                .iter()
                .map(|(slot, term)| match (slot, term) {
                    (Some(slot), Term::Pin(_, pin)) => match to_sampler(*slot) {
                        Some(sampler) => Term::Pin(sampler, *pin),
                        None => Term::Const(matches!(pin, Pin::Low | Pin::Stable)),
                    },
                    (_, term) => *term,
                })
                .collect()
        })
        .collect();
    let mut sampler = Sampler::new(channels, instance.end).resume(instance.current);
    let found = sampler.wait(&terms);
    instance.current = Some(sampler.samplenum());
    let found = found?;
    let matched = (0..wanted.len())
        .filter(|&index| found.has(index))
        .fold(0u64, |mask, index| mask | 1 << index);
    let pins = instance
        .slots
        .iter()
        .map(
            |channel| match channel.and_then(|index| capture.channel(index)) {
                Some(data) if found.sample < data.len() => u8::from(data.sample(found.sample)),
                _ => UNCONNECTED,
            },
        )
        .collect();
    Some((found.sample, matched, pins))
}

/// A text of DSView's annotations: `'@'` and hex digits stand for a number shown in the user's
/// format; such texts are no lines of their own.
fn hex_text(text: &str) -> Option<String> {
    let digits = text.strip_prefix('@')?;
    (!digits.is_empty() && digits.len() < 256).then(|| digits.to_uppercase())
}

/// An annotation `[class, [texts…]]`: its class, its lines, and its number in hex digits from
/// an integer or an `'@'` text (the last of them wins, `'@'` texts over integers).
fn annotation(data: &Bound<'_, PyAny>) -> Option<(u32, Vec<String>, Option<String>)> {
    let list = data.cast::<PyList>().ok()?;
    if list.len() != 2 {
        return None;
    }
    let class: u32 = list.get_item(0).ok()?.extract().ok()?;
    let items = list.get_item(1).ok()?;
    let items = items.cast::<PyList>().ok()?;
    if items.is_empty() {
        return None;
    }
    let mut texts = Vec::new();
    let mut number = None;
    let mut hex = None;
    for item in items.iter() {
        if let Ok(text) = item.extract::<String>() {
            match hex_text(&text) {
                Some(digits) => hex = Some(digits),
                None => texts.push(text),
            }
        } else if let Ok(value) = item.extract::<i64>() {
            number = Some(format!("{value:02X}"));
        }
    }
    if texts.is_empty() && number.is_none() && hex.is_none() {
        return None;
    }
    Some((class, texts, hex.or(number)))
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

    /// Waits until one of the conditions holds and returns every channel's level there: 0 or 1,
    /// or 255 for a channel the decoder was not given.
    #[pyo3(signature = (conds = None))]
    fn wait<'py>(
        slf: &Bound<'py, Self>,
        conds: Option<&Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyTuple>> {
        let py = slf.py();
        let wanted = conditions(conds)?;
        let outcome = {
            let this = slf.borrow();
            let mut guard = this.instance.lock();
            let instance = guard
                .as_mut()
                .ok_or_else(|| PyValueError::new_err("the decoder is not running"))?;
            if instance.slots.is_empty() {
                return Err(PyValueError::new_err("a stacked decoder cannot wait()"));
            }
            if let Some(bad) = wanted
                .iter()
                .flatten()
                .filter_map(|(slot, _)| *slot)
                .find(|&slot| slot >= instance.slots.len())
            {
                return Err(PyIndexError::new_err(format!(
                    "invalid channel index {bad}"
                )));
            }
            py.detach(|| scan(instance, &wanted))
        };
        let Some((sample, matched, pins)) = outcome else {
            return Err(EndOfData::new_err("no more samples"));
        };
        slf.setattr("samplenum", sample)?;
        slf.setattr("matched", matched)?;
        PyTuple::new(py, pins)
    }

    /// Asks for an output of `output_type`; returns its id for `put()`.
    #[pyo3(signature = (output_type, proto_id = None, meta = None))]
    fn register(
        &self,
        output_type: i32,
        proto_id: Option<String>,
        meta: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<usize> {
        let _ = meta;
        self.with_instance(|instance| {
            // The same output asked for again gets its id again.
            let key = (output_type, proto_id.unwrap_or_default());
            if let Some(id) = instance.outputs.iter().position(|output| *output == key) {
                return Ok(id);
            }
            instance.outputs.push(key);
            Ok(instance.outputs.len() - 1)
        })
    }

    /// Whether decoder channel `index` was given a capture channel.
    fn has_channel(&self, index: usize) -> PyResult<bool> {
        self.with_instance(|instance| match instance.slots.get(index) {
            Some(slot) => Ok(slot.is_some()),
            None => Err(PyIndexError::new_err("invalid channel index")),
        })
    }

    /// Writes a decoder's message to the log (DSView's extension).
    fn printlog(&self, text: &str) {
        log::warn!("decoder: {}", text.trim_end());
    }

    /// Puts out `data` for samples `ss..es` on `output_id`.
    fn put(&self, ss: u64, es: u64, output_id: usize, data: &Bound<'_, PyAny>) -> PyResult<()> {
        let (kind, upper) = self.with_instance(|instance| {
            let (kind, _) = instance
                .outputs
                .get(output_id)
                .ok_or_else(|| PyValueError::new_err(format!("unknown output {output_id}")))?;
            Ok((
                *kind,
                instance
                    .upper
                    .as_ref()
                    .map(|upper| upper.clone_ref(data.py())),
            ))
        })?;
        match kind {
            OUTPUT_ANN => {
                // DSView drops a malformed annotation with a log line rather than failing.
                let Some((class, texts, value)) = annotation(data) else {
                    log::debug!("dropped a malformed annotation: {data}");
                    return Ok(());
                };
                self.with_instance(|instance| {
                    // Classes in no row are not shown.
                    if let Some(row) = instance.class_rows.get(class as usize).copied().flatten() {
                        instance.annotations.lock().push(Annotation {
                            start: ss,
                            end: es,
                            class,
                            row,
                            texts,
                            value,
                        });
                    }
                    Ok(())
                })
            }
            OUTPUT_PYTHON => {
                let Some(upper) = upper else {
                    return Ok(());
                };
                // A stacked decoder that fails does not stop the one below (DSView).
                if let Err(error) = upper.call_method1(data.py(), "decode", (ss, es, data)) {
                    let message = error.to_string();
                    log::warn!("a stacked decoder failed: {message}");
                    self.with_instance(|instance| {
                        instance.upper_error.get_or_insert(message);
                        Ok(())
                    })?;
                }
                Ok(())
            }
            // DSView takes no binary or meta outputs.
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
