// SPDX-License-Identifier: Apache-2.0

//! Names the runtime wants for the whole program: schema names, port properties and port
//! names are `&'static str` in `uiharu-core`, so a name read from a module at run time is
//! kept once, for good, and handed out from here every time it is met again.

use std::collections::BTreeSet;
use std::sync::{Mutex, PoisonError};

static NAMES: Mutex<BTreeSet<&'static str>> = Mutex::new(BTreeSet::new());

/// `name` as a string that lives as long as the program: the one copy kept for every name
/// interned so far.
pub fn intern(name: &str) -> &'static str {
    let mut names = NAMES.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(kept) = names.get(name) {
        return kept;
    }
    let kept: &'static str = Box::leak(name.to_owned().into_boxed_str());
    names.insert(kept);
    kept
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_kept_once() {
        let first = intern("interp.test.events");
        let again = intern(&String::from("interp.test.events"));
        assert!(std::ptr::eq(first, again));
        assert_eq!(first, "interp.test.events");
    }
}
