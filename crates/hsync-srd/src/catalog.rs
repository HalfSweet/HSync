//! What the decoders are: their channels, options and annotations, read from the Python classes
//! for the decoder dialog and for running them.

use std::path::Path;

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyTuple};
use serde::Serialize;
use ts_rs::TS;

use crate::SrdError;
use crate::module;

/// A channel a decoder takes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct ChannelSpec {
    pub id: String,
    pub name: String,
    pub desc: String,
}

/// An option of a decoder: its default and, for choices, the values allowed.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct OptionSpec {
    pub id: String,
    pub desc: String,
    pub default: serde_json::Value,
    /// The values to choose from; empty for free entry of the default's type.
    pub values: Vec<serde_json::Value>,
}

/// A class of annotations a decoder puts out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct AnnotationClass {
    pub id: String,
    pub desc: String,
    /// DSView's type code, which hints at the colour (`('7', 'start', 'Start condition')`).
    pub kind: Option<u32>,
}

/// A row of annotations and the classes it shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct AnnotationRow {
    pub id: String,
    pub desc: String,
    pub classes: Vec<u32>,
}

/// A decoder as its class describes it.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DecoderInfo {
    /// The id `.dsl` files store, e.g. `1:i2c`.
    pub id: String,
    /// The Python package the decoder lives in, e.g. `1-i2c`.
    #[serde(skip)]
    #[ts(skip)]
    pub package: String,
    pub name: String,
    pub longname: String,
    pub desc: String,
    pub license: String,
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    pub tags: Vec<String>,
    pub channels: Vec<ChannelSpec>,
    pub optional_channels: Vec<ChannelSpec>,
    pub options: Vec<OptionSpec>,
    pub annotations: Vec<AnnotationClass>,
    pub annotation_rows: Vec<AnnotationRow>,
}

fn text(object: &Bound<'_, PyAny>, name: &str) -> String {
    object
        .getattr(name)
        .ok()
        .and_then(|value| value.extract::<String>().ok())
        .unwrap_or_default()
}

fn texts(object: &Bound<'_, PyAny>, name: &str) -> Vec<String> {
    object
        .getattr(name)
        .ok()
        .and_then(|value| value.extract::<Vec<String>>().ok())
        .unwrap_or_default()
}

/// A Python value as JSON: the option values decoders use are numbers and strings.
pub(crate) fn json_of(value: &Bound<'_, PyAny>) -> serde_json::Value {
    if let Ok(flag) = value.extract::<bool>()
        && value.is_instance_of::<pyo3::types::PyBool>()
    {
        return serde_json::Value::Bool(flag);
    }
    if let Ok(number) = value.extract::<i64>()
        && value.is_instance_of::<pyo3::types::PyInt>()
    {
        return number.into();
    }
    if let Ok(number) = value.extract::<f64>() {
        return serde_json::Number::from_f64(number).map_or(serde_json::Value::Null, Into::into);
    }
    if let Ok(string) = value.extract::<String>() {
        return string.into();
    }
    serde_json::Value::Null
}

fn channel_specs(class: &Bound<'_, PyAny>, name: &str) -> Result<Vec<ChannelSpec>, SrdError> {
    let Ok(list) = class.getattr(name) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for item in list.try_iter()? {
        let item = item?;
        let dict = item
            .cast::<PyDict>()
            .map_err(|_| SrdError::InvalidDecoder {
                decoder: text(class, "id"),
                detail: format!("{name} entries must be dicts"),
            })?;
        let field = |key: &str| -> String {
            dict.get_item(key)
                .ok()
                .flatten()
                .and_then(|value| value.extract::<String>().ok())
                .unwrap_or_default()
        };
        out.push(ChannelSpec {
            id: field("id"),
            name: field("name"),
            desc: field("desc"),
        });
    }
    Ok(out)
}

fn option_specs(class: &Bound<'_, PyAny>) -> Result<Vec<OptionSpec>, SrdError> {
    let Ok(list) = class.getattr("options") else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for item in list.try_iter()? {
        let item = item?;
        let Ok(dict) = item.cast::<PyDict>() else {
            continue;
        };
        let id = dict
            .get_item("id")?
            .and_then(|value| value.extract::<String>().ok())
            .unwrap_or_default();
        let desc = dict
            .get_item("desc")?
            .and_then(|value| value.extract::<String>().ok())
            .unwrap_or_default();
        let default = dict
            .get_item("default")?
            .map_or(serde_json::Value::Null, |value| json_of(&value));
        let values = match dict.get_item("values")? {
            Some(values) => values
                .try_iter()?
                .filter_map(Result::ok)
                .map(|value| json_of(&value))
                .collect(),
            None => Vec::new(),
        };
        out.push(OptionSpec {
            id,
            desc,
            default,
            values,
        });
    }
    Ok(out)
}

/// The text of a tuple element: a string, or an integer in decimal (DSView takes both).
fn element(item: &Bound<'_, PyAny>) -> Option<String> {
    item.extract::<String>()
        .ok()
        .or_else(|| item.extract::<i64>().ok().map(|number| number.to_string()))
}

/// The annotation classes with DSView's type codes (`decoder.c`, `get_annotations`): a
/// three-element class carries its code; two-element ones count up from 7, continuing after
/// the last code given.
fn annotation_classes(class: &Bound<'_, PyAny>) -> Result<Vec<AnnotationClass>, SrdError> {
    let Ok(list) = class.getattr("annotations") else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    let mut kind: u32 = 7;
    for item in list.try_iter()? {
        let item = item?;
        let fields: Vec<String> = item
            .try_iter()
            .map(|items| {
                items
                    .filter_map(Result::ok)
                    .filter_map(|one| element(&one))
                    .collect()
            })
            .unwrap_or_default();
        out.push(match fields.as_slice() {
            [code, id, desc] => {
                kind = code
                    .bytes()
                    .filter(u8::is_ascii_digit)
                    .fold(0u32, |sum, digit| {
                        sum.wrapping_mul(10).wrapping_add(u32::from(digit - b'0'))
                    });
                AnnotationClass {
                    id: id.clone(),
                    desc: desc.clone(),
                    kind: Some(kind),
                }
            }
            [id, desc] => {
                let class = AnnotationClass {
                    id: id.clone(),
                    desc: desc.clone(),
                    kind: Some(kind),
                };
                kind += 1;
                class
            }
            _ => {
                return Err(SrdError::InvalidDecoder {
                    decoder: text(class, "id"),
                    detail: "annotations need two or three elements".into(),
                });
            }
        });
    }
    Ok(out)
}

fn annotation_rows(class: &Bound<'_, PyAny>) -> Result<Vec<AnnotationRow>, SrdError> {
    let Ok(list) = class.getattr("annotation_rows") else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for item in list.try_iter()? {
        let item = item?;
        let Ok(tuple) = item.cast::<PyTuple>() else {
            continue;
        };
        if tuple.len() != 3 {
            continue;
        }
        out.push(AnnotationRow {
            id: tuple.get_item(0)?.extract().unwrap_or_default(),
            desc: tuple.get_item(1)?.extract().unwrap_or_default(),
            classes: tuple.get_item(2)?.extract().unwrap_or_default(),
        });
    }
    Ok(out)
}

/// Reads a decoder class.
pub(crate) fn describe(class: &Bound<'_, PyAny>, package: &str) -> Result<DecoderInfo, SrdError> {
    Ok(DecoderInfo {
        id: text(class, "id"),
        package: package.to_owned(),
        name: text(class, "name"),
        longname: text(class, "longname"),
        desc: text(class, "desc"),
        license: text(class, "license"),
        inputs: texts(class, "inputs"),
        outputs: texts(class, "outputs"),
        tags: texts(class, "tags"),
        channels: channel_specs(class, "channels")?,
        optional_channels: channel_specs(class, "optional_channels")?,
        options: option_specs(class)?,
        annotations: annotation_classes(class)?,
        annotation_rows: annotation_rows(class)?,
    })
}

/// Imports the decoder package in `dir` named `name` and returns its `Decoder` class.
pub(crate) fn decoder_class<'py>(py: Python<'py>, name: &str) -> PyResult<Bound<'py, PyAny>> {
    let importlib = py.import("importlib")?;
    let package = importlib.call_method1("import_module", (name,))?;
    package.getattr("Decoder")
}

/// Every decoder in `dir`, sorted by id. Decoders that fail to load are left out with a warning.
pub fn load_catalog(dir: &Path) -> Result<Vec<DecoderInfo>, SrdError> {
    Python::attach(|py| {
        module::install(py, dir)?;
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .map_err(|error| SrdError::Python(format!("{}: {error}", dir.display())))?
            .filter_map(Result::ok)
            .filter(|entry| entry.path().join("pd.py").is_file())
            .filter_map(|entry| entry.file_name().into_string().ok())
            .collect();
        names.sort();
        let mut out = Vec::new();
        for name in names {
            match decoder_class(py, &name)
                .map_err(SrdError::from)
                .and_then(|class| describe(&class, &name))
            {
                Ok(info) => out.push(info),
                Err(error) => log::warn!("decoder {name} did not load: {error}"),
            }
        }
        out.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(out)
    })
}
