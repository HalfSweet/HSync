//! Saving and exporting captures (DESIGN.md §8): what to write is taken from the workspace
//! under its lock, and the writing goes on without it, reporting progress and stopping when
//! asked.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use hsync_capture::LogicCapture;
use hsync_core::{ChannelIndex, SampleIndex};
use hsync_fileio::{DslWriteRequest, ExportFormat, Progress};
use parking_lot::RwLock;
use serde::Deserialize;
use ts_rs::TS;

use crate::SessionError;

/// The formats a capture can be written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ExportKind {
    /// DSView's capture file.
    Dsl,
    Csv,
    Vcd,
    /// sigrok's session archive.
    Sigrok,
    /// Interleaved sample bytes.
    Binary,
}

/// What to export, as the export dialog asks for it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ExportRequest {
    pub kind: ExportKind,
    /// CSV: a row for every sample rather than for every change.
    pub every_sample: bool,
    /// First and last sample to write; the whole capture without one.
    pub range: Option<(SampleIndex, SampleIndex)>,
    /// Channels to write, in this order; all of them when empty.
    pub channels: Vec<ChannelIndex>,
}

/// A capture's samples and the names and documents written with them.
#[derive(Debug, Clone)]
pub struct ExportSource {
    pub(crate) capture: Arc<RwLock<LogicCapture>>,
    pub(crate) channel_names: BTreeMap<ChannelIndex, String>,
    pub(crate) decoders: Option<serde_json::Value>,
    pub(crate) driver: String,
    pub(crate) session: Option<serde_json::Value>,
}

impl ExportSource {
    fn driver(&self) -> &str {
        if self.driver.is_empty() {
            "HSync"
        } else {
            &self.driver
        }
    }

    fn name_of(&self, index: ChannelIndex) -> String {
        self.channel_names
            .get(&index)
            .cloned()
            .unwrap_or_else(|| index.to_string())
    }

    /// Writes the whole capture to `path` as a `.dsl` file; returns the `session` document
    /// written, for the workspace to keep.
    pub fn save(
        &self,
        path: &Path,
        progress: Progress<'_>,
    ) -> Result<serde_json::Value, SessionError> {
        let capture = self.capture.read();
        let channels = capture.info().channels;
        let decoders = self
            .decoders
            .clone()
            .unwrap_or_else(|| serde_json::Value::Array(Vec::new()));
        self.write_dsl(path, &capture, &channels, &decoders, progress)
    }

    fn write_dsl(
        &self,
        path: &Path,
        capture: &LogicCapture,
        channels: &[ChannelIndex],
        decoders: &serde_json::Value,
        progress: Progress<'_>,
    ) -> Result<serde_json::Value, SessionError> {
        let names: BTreeMap<ChannelIndex, String> = channels
            .iter()
            .map(|&index| (index, self.name_of(index)))
            .collect();
        let info = capture.info();
        let session = session_document(
            self.session.as_ref(),
            self.driver(),
            &info,
            &names,
            decoders,
        );
        let request = DslWriteRequest {
            driver: self.driver(),
            channel_names: &names,
            session: &session,
            decoders,
        };
        hsync_fileio::write_dsl_file(path, capture, &request, progress)?;
        Ok(session)
    }

    /// Writes the part of the capture `request` asks for to `path`.
    pub fn export(
        &self,
        path: &Path,
        request: &ExportRequest,
        progress: Progress<'_>,
    ) -> Result<(), SessionError> {
        let capture = self.capture.read();
        let all = capture.info().channels;
        let channels: Vec<ChannelIndex> = if request.channels.is_empty() {
            all.clone()
        } else {
            request
                .channels
                .iter()
                .copied()
                .filter(|index| all.contains(index))
                .collect()
        };
        if channels.is_empty() {
            return Err(SessionError::NothingToExport);
        }
        let (from, to) = match request.range {
            Some((first, last)) => (first.min(last), first.max(last).saturating_add(1)),
            None => (0, capture.len()),
        };
        let whole = from == 0 && to >= capture.len() && channels == all;
        let sliced;
        let part: &LogicCapture = if whole {
            &capture
        } else {
            sliced = capture.slice(from, to, &channels);
            &sliced
        };
        if request.kind == ExportKind::Dsl {
            // Decoders go along when every channel they use is written.
            let decoders = serde_json::Value::Array(
                self.decoders
                    .as_ref()
                    .and_then(serde_json::Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter(|entry| {
                        decoder_channels(entry)
                            .iter()
                            .all(|index| channels.contains(index))
                    })
                    .cloned()
                    .collect(),
            );
            self.write_dsl(path, part, &channels, &decoders, progress)?;
            return Ok(());
        }
        let format = match request.kind {
            ExportKind::Csv => ExportFormat::Csv {
                every_sample: request.every_sample,
            },
            ExportKind::Vcd => ExportFormat::Vcd,
            ExportKind::Sigrok => ExportFormat::Sigrok,
            _ => ExportFormat::Binary,
        };
        let names: Vec<(ChannelIndex, String)> = channels
            .iter()
            .map(|&index| (index, self.name_of(index)))
            .collect();
        let date = utc_now();
        let generator = format!("HSync {}", env!("CARGO_PKG_VERSION"));
        let export = hsync_fileio::ExportRequest {
            format,
            channels: &names,
            total_channels: all.len(),
            first_sample: from,
            date: &date,
            generator: &generator,
        };
        hsync_fileio::write_atomically(path, |file| {
            hsync_fileio::export(file, part, &export, progress)
        })?;
        Ok(())
    }
}

/// Writes `text` to `path`, such as the data stream as the panel lists it.
pub fn write_text(path: &Path, text: &str) -> Result<(), SessionError> {
    hsync_fileio::write_atomically(path, |mut file| {
        std::io::Write::write_all(&mut file, text.as_bytes())?;
        Ok(file)
    })?;
    Ok(())
}

/// Channels a DSView decoder entry uses.
fn decoder_channels(entry: &serde_json::Value) -> Vec<ChannelIndex> {
    entry
        .get("channel")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_object)
        .flat_map(|roles| roles.values())
        .filter_map(serde_json::Value::as_u64)
        .filter_map(|index| ChannelIndex::try_from(index).ok())
        .collect()
}

/// The current time in UTC, e.g. `2026-10-10 06:30:00 UTC`.
fn utc_now() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    let days = (seconds / 86_400) as i64;
    let rest = seconds % 86_400;
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02} UTC",
        rest / 3_600,
        rest % 3_600 / 60,
        rest % 60
    )
}

/// DSView's channel type of logic channels (`SR_CHANNEL_LOGIC`).
const LOGIC_CHANNEL: u32 = 10000;

/// DSView's `session` document for a capture: the one it was read with, brought up to date, or
/// a new one with the fields DSView needs to open the file.
fn session_document(
    previous: Option<&serde_json::Value>,
    driver: &str,
    info: &hsync_capture::CaptureInfo,
    names: &BTreeMap<ChannelIndex, String>,
    decoders: &serde_json::Value,
) -> serde_json::Value {
    let mut session = previous.cloned().unwrap_or_else(|| {
        serde_json::json!({
            "Version": 3,
            "Title": "HSync",
            "DeviceMode": 0,
            "Max Height": "1X",
        })
    });
    let Some(fields) = session.as_object_mut() else {
        return session;
    };
    // DSView disables every channel of the file that `channel` does not list.
    let listed = fields
        .get("channel")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    let channels = names
        .iter()
        .enumerate()
        .map(|(position, (&index, name))| {
            let previous = listed.iter().find(|entry| {
                entry.get("index").and_then(serde_json::Value::as_u64) == Some(u64::from(index))
            });
            let mut entry = previous.cloned().unwrap_or_else(|| {
                serde_json::json!({ "colour": "default", "strigger": 0, "view_index": position })
            });
            if let Some(entry) = entry.as_object_mut() {
                entry.insert("index".into(), index.into());
                entry.insert("name".into(), name.as_str().into());
                entry.insert("enabled".into(), true.into());
                entry.insert("type".into(), LOGIC_CHANNEL.into());
            }
            entry
        })
        .collect();
    fields.insert("Device".into(), driver.into());
    fields.insert("Sample rate".into(), info.samplerate.to_string().into());
    fields.insert("Sample count".into(), info.sample_count.to_string().into());
    fields.insert("channel".into(), serde_json::Value::Array(channels));
    fields.insert("decoder".into(), decoders.clone());
    session
}
