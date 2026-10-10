//! Progress of long writes.

/// Told how far a long write got, from 0 to 1; returning `false` cancels the write, which then
/// fails with [`crate::FileError::Cancelled`].
pub type Progress<'a> = &'a mut dyn FnMut(f64) -> bool;
