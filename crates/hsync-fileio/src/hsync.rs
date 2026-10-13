//! HSync's own capture file, `.hsync`: a zip archive holding
//!
//! - `manifest.json`: the capture's timing, channels, trigger and origin (see [`HsyncManifest`]);
//! - `decoders.json`: the decoder entries, as the session stores them;
//! - `session.json`: the interface's session document (device settings, triggers, colours,
//!   the view), kept as it is;
//! - `channels/<index>/<block>`: the samples of each channel, 64 per word, little-endian words
//!   in blocks of `manifest.blocks.words` words, each block LZ4-compressed on its own (the
//!   block's size in front, as `lz4_flex` writes it) and stored uncompressed in the archive.
//!
//! A reader takes any file whose `manifest.version` it knows; later versions may add entries
//! and fields, which readers ignore.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Seek, Write};
use std::path::Path;

use hsync_capture::{CHUNK_WORDS, ChunkView, LogicCapture};
use hsync_core::ChannelIndex;
use serde::{Deserialize, Serialize};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::dsl::write_atomically;
use crate::error::FileError;
use crate::progress::Progress;

/// What `manifest.format` says.
pub const HSYNC_FORMAT: &str = "hsync-capture";

/// The version this crate writes, and the latest it reads.
pub const HSYNC_VERSION: u32 = 1;

/// The file name extension, without the dot.
pub const HSYNC_EXTENSION: &str = "hsync";

/// Whether `path` names an HSync capture file, by its extension.
pub fn is_hsync_path(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case(HSYNC_EXTENSION))
}

/// A channel of the capture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HsyncChannel {
    /// Hardware channel number.
    pub index: ChannelIndex,
    /// Its name; the number as text when it has none.
    pub name: String,
}

/// Where the trigger fired.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HsyncTrigger {
    /// The sample the capture is timed from.
    pub sample: u64,
    /// When it fired, in milliseconds since the Unix epoch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_ms: Option<i64>,
}

/// The device the capture came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HsyncSource {
    pub driver: String,
    pub model: String,
}

/// How the samples are cut into blocks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HsyncBlocks {
    /// Words (of 64 samples) per block, the last block of a channel aside.
    pub words: usize,
    /// How each block is compressed: `lz4`.
    pub compression: String,
}

/// The `manifest.json` entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HsyncManifest {
    /// Always [`HSYNC_FORMAT`].
    pub format: String,
    pub version: u32,
    /// The HSync that wrote the file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_version: Option<String>,
    pub samplerate: u64,
    /// Samples per channel.
    pub samples: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger: Option<HsyncTrigger>,
    /// The channels, ascending by index.
    pub channels: Vec<HsyncChannel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<HsyncSource>,
    pub blocks: HsyncBlocks,
}

/// A capture file as read.
#[derive(Debug)]
pub struct HsyncDocument {
    pub manifest: HsyncManifest,
    pub capture: LogicCapture,
    /// The `decoders.json` entry.
    pub decoders: Option<serde_json::Value>,
    /// The `session.json` entry.
    pub session: Option<serde_json::Value>,
}

/// What [`write_hsync`] stores besides the samples.
#[derive(Debug)]
pub struct HsyncWriteRequest<'a> {
    /// Name of every channel of the capture; channels left out are named by their number.
    pub channel_names: &'a BTreeMap<ChannelIndex, String>,
    pub source: Option<HsyncSource>,
    /// The HSync writing the file.
    pub app_version: &'a str,
    pub decoders: Option<&'a serde_json::Value>,
    pub session: Option<&'a serde_json::Value>,
}

const MANIFEST: &str = "manifest.json";
const DECODERS: &str = "decoders.json";
const SESSION: &str = "session.json";
const LZ4: &str = "lz4";

fn block_name(channel: ChannelIndex, block: usize) -> String {
    format!("channels/{channel}/{block}")
}

fn read_json<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    name: &str,
) -> Result<Option<serde_json::Value>, FileError> {
    let Ok(mut entry) = archive.by_name(name) else {
        return Ok(None);
    };
    let mut text = String::new();
    entry.read_to_string(&mut text)?;
    if text.trim().is_empty() {
        return Ok(None);
    }
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|source| FileError::Json {
            entry: name.to_owned(),
            source,
        })
}

/// Reads a capture file from `reader`.
pub fn read_hsync<R: Read + Seek>(reader: R) -> Result<HsyncDocument, FileError> {
    let mut archive = ZipArchive::new(reader)?;
    let manifest: HsyncManifest = {
        let Ok(mut entry) = archive.by_name(MANIFEST) else {
            return Err(FileError::NotHsync);
        };
        let mut text = String::new();
        entry.read_to_string(&mut text)?;
        serde_json::from_str(&text).map_err(|source| FileError::Json {
            entry: MANIFEST.to_owned(),
            source,
        })?
    };
    if manifest.format != HSYNC_FORMAT {
        return Err(FileError::NotHsync);
    }
    if manifest.version > HSYNC_VERSION || manifest.version == 0 {
        return Err(FileError::UnsupportedVersion(manifest.version));
    }
    if manifest.blocks.compression != LZ4 {
        return Err(FileError::Block {
            entry: MANIFEST.to_owned(),
            detail: format!("unknown compression {}", manifest.blocks.compression),
        });
    }
    let channels: Vec<ChannelIndex> = manifest
        .channels
        .iter()
        .map(|channel| channel.index)
        .collect();
    let mut capture = LogicCapture::new(manifest.samplerate, channels.iter().copied());
    let words = usize::try_from(manifest.samples.div_ceil(64)).unwrap_or(usize::MAX);
    let block_words = manifest.blocks.words.max(1);
    let blocks = words.div_ceil(block_words);
    let mut compressed = Vec::new();
    for &channel in &channels {
        let data = capture
            .channel_mut(channel)
            .ok_or_else(|| FileError::MissingEntry(block_name(channel, 0)))?;
        for block in 0..blocks {
            let name = block_name(channel, block);
            let mut entry = archive
                .by_name(&name)
                .map_err(|_| FileError::MissingEntry(name.clone()))?;
            compressed.clear();
            entry.read_to_end(&mut compressed)?;
            let bytes =
                lz4_flex::block::decompress_size_prepended(&compressed).map_err(|error| {
                    FileError::Block {
                        entry: name.clone(),
                        detail: error.to_string(),
                    }
                })?;
            if bytes.len() % 8 != 0 {
                return Err(FileError::MisalignedBlock {
                    entry: name,
                    len: bytes.len() as u64,
                });
            }
            data.append_bytes(&bytes);
        }
    }
    capture.truncate(manifest.samples);
    if let Some(trigger) = &manifest.trigger {
        capture.set_trigger(Some(trigger.sample), trigger.time_ms);
    }
    let decoders = read_json(&mut archive, DECODERS)?;
    let session = read_json(&mut archive, SESSION)?;
    Ok(HsyncDocument {
        manifest,
        capture,
        decoders,
        session,
    })
}

/// Reads a capture file from disk.
pub fn read_hsync_file(path: &Path) -> Result<HsyncDocument, FileError> {
    read_hsync(BufReader::new(File::open(path)?))
}

/// Writes `capture` as a capture file.
pub fn write_hsync<W: Write + Seek>(
    writer: W,
    capture: &LogicCapture,
    request: &HsyncWriteRequest<'_>,
) -> Result<W, FileError> {
    write_hsync_with(writer, capture, request, &mut |_| true)
}

/// [`write_hsync`], telling `progress` how far it got and stopping when it says so.
pub fn write_hsync_with<W: Write + Seek>(
    writer: W,
    capture: &LogicCapture,
    request: &HsyncWriteRequest<'_>,
    progress: Progress<'_>,
) -> Result<W, FileError> {
    let samples = capture.len();
    let words = usize::try_from(samples.div_ceil(64)).unwrap_or(usize::MAX);
    let blocks = words.div_ceil(CHUNK_WORDS);
    let info = capture.info();
    let manifest = HsyncManifest {
        format: HSYNC_FORMAT.to_owned(),
        version: HSYNC_VERSION,
        app_version: Some(request.app_version.to_owned()),
        samplerate: capture.samplerate(),
        samples,
        trigger: info.trigger_sample.map(|sample| HsyncTrigger {
            sample,
            time_ms: info.trigger_time_ms,
        }),
        channels: info
            .channels
            .iter()
            .map(|&index| HsyncChannel {
                index,
                name: request
                    .channel_names
                    .get(&index)
                    .cloned()
                    .unwrap_or_else(|| index.to_string()),
            })
            .collect(),
        source: request.source.clone(),
        blocks: HsyncBlocks {
            words: CHUNK_WORDS,
            compression: LZ4.to_owned(),
        },
    };
    let json = |value: &serde_json::Value, entry: &str| -> Result<String, FileError> {
        serde_json::to_string_pretty(value).map_err(|source| FileError::Json {
            entry: entry.to_owned(),
            source,
        })
    };
    let text = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    // Blocks are compressed already; the archive just holds them.
    let stored = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Stored)
        .large_file(words.saturating_mul(8) > u32::MAX as usize);
    let mut zip = ZipWriter::new(writer);
    zip.start_file(MANIFEST, text)?;
    zip.write_all(
        json(
            &serde_json::to_value(&manifest).unwrap_or_default(),
            MANIFEST,
        )?
        .as_bytes(),
    )?;
    if let Some(decoders) = request.decoders {
        zip.start_file(DECODERS, text)?;
        zip.write_all(json(decoders, DECODERS)?.as_bytes())?;
    }
    if let Some(session) = request.session {
        zip.start_file(SESSION, text)?;
        zip.write_all(json(session, SESSION)?.as_bytes())?;
    }

    let mut bytes = Vec::new();
    let total = (blocks * info.channels.len()).max(1);
    let mut done = 0;
    for (index, data) in capture.channels() {
        for block in 0..blocks {
            if !progress(done as f64 / total as f64) {
                return Err(FileError::Cancelled);
            }
            done += 1;
            let block_words = (words - block * CHUNK_WORDS).min(CHUNK_WORDS);
            bytes.clear();
            match data.chunk(block) {
                ChunkView::Constant(level) => {
                    bytes.resize(block_words * 8, if level { 0xFF } else { 0x00 });
                }
                ChunkView::Words(chunk) => {
                    bytes.extend(
                        chunk[..block_words]
                            .iter()
                            .flat_map(|word| word.to_le_bytes()),
                    );
                }
            }
            zip.start_file(block_name(index, block), stored)?;
            zip.write_all(&lz4_flex::block::compress_prepend_size(&bytes))?;
        }
    }
    progress(1.0);
    Ok(zip.finish()?)
}

/// Writes `capture` to `path`, leaving an existing file as it was when the write fails.
pub fn write_hsync_file(
    path: &Path,
    capture: &LogicCapture,
    request: &HsyncWriteRequest<'_>,
    progress: Progress<'_>,
) -> Result<(), FileError> {
    write_atomically(path, |file: BufWriter<File>| {
        write_hsync_with(file, capture, request, progress)
    })
}
