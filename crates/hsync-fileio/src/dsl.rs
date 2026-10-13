//! Reading and writing `.dsl` capture files.
//!
//! A `.dsl` file is a zip archive with a `header` (see [`crate::header`]), `decoders` and
//! `session` JSON documents, and the samples. Versions 2 and 3 store channel `c`'s block `b` as
//! `L-c/b`: `2^24` samples, LSB first, the last block shorter. Version 1 stores all channels
//! interleaved in one `data` entry, two bytes per sample.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Seek, Write};
use std::path::Path;

use hsync_capture::{CHUNK_WORDS, ChunkView, LogicCapture};
use hsync_core::ChannelIndex;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::FileError;
use crate::header::{DslHeader, HEADER_VERSION, LOGIC_MODE};
use crate::progress::Progress;

/// A capture file loaded into memory.
#[derive(Debug)]
pub struct DslDocument {
    /// The parsed `header` entry.
    pub header: DslHeader,
    /// The samples.
    pub capture: LogicCapture,
    /// The `session` entry: channel names, colours, decoders and device settings.
    pub session: Option<serde_json::Value>,
    /// The `decoders` entry: decoder stacks and their options.
    pub decoders: Option<serde_json::Value>,
    /// The `hsync-decoders` entry HSync adds: its own decoder entries, which DSView ignores.
    pub hsync_decoders: Option<serde_json::Value>,
}

/// Reads a `.dsl` file from disk.
pub fn read_dsl_file(path: &Path) -> Result<DslDocument, FileError> {
    read_dsl(BufReader::new(File::open(path)?))
}

/// Writes `capture` to the `.dsl` file at `path`; see [`write_atomically`].
pub fn write_dsl_file(
    path: &Path,
    capture: &LogicCapture,
    request: &DslWriteRequest<'_>,
    progress: Progress<'_>,
) -> Result<(), FileError> {
    write_atomically(path, |file| {
        write_dsl_with(file, capture, request, progress)
    })
}

/// Writes a file next to `path` and moves it over `path` once complete, so a failed or cancelled
/// write leaves an existing file as it was.
pub fn write_atomically(
    path: &Path,
    write: impl FnOnce(BufWriter<File>) -> Result<BufWriter<File>, FileError>,
) -> Result<(), FileError> {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".part");
    let partial = path.with_file_name(name);
    let written = (|| {
        let writer = write(BufWriter::new(File::create(&partial)?))?;
        let file = writer.into_inner().map_err(|error| error.into_error())?;
        file.sync_all()?;
        std::fs::rename(&partial, path)?;
        Ok(())
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&partial);
    }
    written
}

/// Reads a `.dsl` archive.
pub fn read_dsl<R: Read + Seek>(reader: R) -> Result<DslDocument, FileError> {
    let mut archive = ZipArchive::new(reader)?;
    let header = DslHeader::parse(&read_text(&mut archive, "header")?)?;
    if header.device_mode != LOGIC_MODE {
        return Err(FileError::UnsupportedMode(header.device_mode));
    }
    let mut capture = match header.version {
        1 => read_interleaved(&mut archive, &header)?,
        2 | 3 => read_blocks(&mut archive, &header)?,
        version => return Err(FileError::UnsupportedVersion(version)),
    };
    if header.total_samples > 0 {
        capture.truncate(header.total_samples);
    }
    capture.set_trigger(Some(header.trigger_pos), header.trigger_time_ms);

    let session = read_json(&mut archive, "session")?;
    let decoders = read_json(&mut archive, "decoders")?;
    let hsync_decoders = read_json(&mut archive, "hsync-decoders")?;
    Ok(DslDocument {
        header,
        capture,
        session,
        decoders,
        hsync_decoders,
    })
}

fn read_text<R: Read + Seek>(archive: &mut ZipArchive<R>, name: &str) -> Result<String, FileError> {
    let mut entry = archive
        .by_name(name)
        .map_err(|_| FileError::MissingEntry(name.to_owned()))?;
    let mut text = String::new();
    entry.read_to_string(&mut text)?;
    Ok(text)
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

fn read_blocks<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    header: &DslHeader,
) -> Result<LogicCapture, FileError> {
    let channels: Vec<ChannelIndex> = header.probes.iter().map(|(index, _)| *index).collect();
    let mut capture = LogicCapture::new(header.samplerate, channels.iter().copied());
    let mut buffer = Vec::new();
    for &channel in &channels {
        let data = capture
            .channel_mut(channel)
            .ok_or_else(|| FileError::MissingEntry(format!("L-{channel}/0")))?;
        for block in 0.. {
            if header.total_blocks.is_some_and(|blocks| block >= blocks) {
                break;
            }
            let name = format!("L-{channel}/{block}");
            let mut entry = match archive.by_name(&name) {
                Ok(entry) => entry,
                // Without a block count, read until the blocks run out.
                Err(_) if header.total_blocks.is_none() && block > 0 => break,
                Err(_) => return Err(FileError::MissingEntry(name)),
            };
            buffer.clear();
            entry.read_to_end(&mut buffer)?;
            if buffer.len() % 8 != 0 {
                return Err(FileError::MisalignedBlock {
                    entry: name,
                    len: buffer.len() as u64,
                });
            }
            data.append_bytes(&buffer);
        }
    }
    Ok(capture)
}

fn read_interleaved<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    header: &DslHeader,
) -> Result<LogicCapture, FileError> {
    let mut entry = archive
        .by_name("data")
        .map_err(|_| FileError::MissingEntry("data".to_owned()))?;
    let mut bytes = Vec::new();
    entry.read_to_end(&mut bytes)?;

    // Version 1 always stores two bytes per sample; bit `c` is channel `c` (DSView's
    // `trans_data`).
    let channels: Vec<ChannelIndex> = header
        .probes
        .iter()
        .map(|(index, _)| *index)
        .filter(|index| *index < 16)
        .collect();
    let mut capture = LogicCapture::new(header.samplerate, channels.iter().copied());
    let (samples, _) = bytes.as_chunks::<2>();
    for &channel in &channels {
        let words: Vec<u64> = samples
            .chunks(64)
            .map(|group| {
                group.iter().enumerate().fold(0u64, |word, (bit, sample)| {
                    let level = u16::from_le_bytes(*sample) >> channel & 1;
                    word | u64::from(level) << bit
                })
            })
            .collect();
        if let Some(data) = capture.channel_mut(channel) {
            data.append_words(&words);
        }
    }
    let samples = samples.len() as u64;
    capture.truncate(samples);
    Ok(capture)
}

/// What [`write_dsl`] stores besides the samples.
#[derive(Debug)]
pub struct DslWriteRequest<'a> {
    /// Driver name recorded in the header (`DSLogic`, `virtual-demo`, ...).
    pub driver: &'a str,
    /// Name of every channel of the capture.
    pub channel_names: &'a BTreeMap<ChannelIndex, String>,
    /// The `session` document.
    pub session: &'a serde_json::Value,
    /// The `decoders` document, as DSView reads it.
    pub decoders: &'a serde_json::Value,
    /// HSync's own decoder entries, written as `hsync-decoders` for HSync to read back.
    pub hsync_decoders: Option<&'a serde_json::Value>,
}

/// Writes `capture` as a version 3 `.dsl` archive that DSView can open.
pub fn write_dsl<W: Write + Seek>(
    writer: W,
    capture: &LogicCapture,
    request: &DslWriteRequest<'_>,
) -> Result<W, FileError> {
    write_dsl_with(writer, capture, request, &mut |_| true)
}

/// [`write_dsl`], telling `progress` how far it got and stopping when it says so.
pub fn write_dsl_with<W: Write + Seek>(
    writer: W,
    capture: &LogicCapture,
    request: &DslWriteRequest<'_>,
    progress: Progress<'_>,
) -> Result<W, FileError> {
    let samples = capture.len();
    let words = usize::try_from(samples.div_ceil(64)).unwrap_or(usize::MAX);
    let blocks = words.div_ceil(CHUNK_WORDS);
    let info = capture.info();
    let header = DslHeader {
        version: HEADER_VERSION,
        driver: request.driver.to_owned(),
        device_mode: LOGIC_MODE,
        total_samples: samples,
        total_probes: u32::try_from(info.channels.len()).unwrap_or(u32::MAX),
        total_blocks: Some(u32::try_from(blocks).unwrap_or(u32::MAX)),
        samplerate: capture.samplerate(),
        trigger_time_ms: info.trigger_time_ms,
        trigger_pos: info.trigger_sample.unwrap_or(0),
        probes: info
            .channels
            .iter()
            .map(|index| {
                let name = request
                    .channel_names
                    .get(index)
                    .cloned()
                    .unwrap_or_else(|| index.to_string());
                (*index, name)
            })
            .collect(),
    };

    // DSView compresses with Z_BEST_SPEED; sample blocks are large and repetitive.
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .compression_level(Some(1))
        .large_file(words.saturating_mul(8) > u32::MAX as usize);
    let mut zip = ZipWriter::new(writer);
    zip.start_file("header", options)?;
    zip.write_all(header.to_text().as_bytes())?;
    zip.start_file("decoders", options)?;
    zip.write_all(json_text(request.decoders, "decoders")?.as_bytes())?;
    if let Some(decoders) = request.hsync_decoders {
        zip.start_file("hsync-decoders", options)?;
        zip.write_all(json_text(decoders, "hsync-decoders")?.as_bytes())?;
    }
    zip.start_file("session", options)?;
    zip.write_all(json_text(request.session, "session")?.as_bytes())?;

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
            zip.start_file(format!("L-{index}/{block}"), options)?;
            zip.write_all(&bytes)?;
        }
    }
    progress(1.0);
    Ok(zip.finish()?)
}

fn json_text(value: &serde_json::Value, entry: &str) -> Result<String, FileError> {
    serde_json::to_string_pretty(value).map_err(|source| FileError::Json {
        entry: entry.to_owned(),
        source,
    })
}
