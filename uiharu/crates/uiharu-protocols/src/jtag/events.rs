// SPDX-License-Identifier: Apache-2.0

//! The `jtag.events` contract: what a JTAG decoder emits and what decoders built on JTAG read.

use serde::{Deserialize, Serialize};
use uiharu_core::{EventSchema, SchemaId};

/// The schema of JTAG event streams.
pub const JTAG_EVENTS: SchemaId = SchemaId::new("jtag.events", 1);

/// The most bits one `shift` event carries.
pub const SHIFT_BATCH_BITS: u16 = 256;

/// The bytes a full `shift` batch takes on one line.
pub(crate) const SHIFT_BATCH_BYTES: usize = SHIFT_BATCH_BITS as usize / 8;

/// A scan: one passage through Shift-IR or Shift-DR, numbered from 1 by the decoder.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct ScanId(pub u64);

/// The register a scan shifts through.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Register {
    /// The instruction register, shifted in Shift-IR.
    Ir,
    /// The data register the current instruction selects, shifted in Shift-DR.
    Dr,
}

/// The sixteen states of the TAP controller, named as IEEE 1149.1 names them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TapState {
    /// Test-Logic-Reset: the test logic is disabled.
    TestLogicReset,
    /// Run-Test/Idle.
    RunTestIdle,
    /// Select-DR-Scan.
    SelectDrScan,
    /// Capture-DR: the data register loads its parallel input.
    CaptureDr,
    /// Shift-DR: the data register shifts one bit per clock between TDI and TDO.
    ShiftDr,
    /// Exit1-DR.
    Exit1Dr,
    /// Pause-DR: shifting through the data register halts for a while.
    PauseDr,
    /// Exit2-DR.
    Exit2Dr,
    /// Update-DR: the data register's shifted value is latched.
    UpdateDr,
    /// Select-IR-Scan.
    SelectIrScan,
    /// Capture-IR: the instruction register loads its fixed pattern.
    CaptureIr,
    /// Shift-IR: the instruction register shifts one bit per clock between TDI and TDO.
    ShiftIr,
    /// Exit1-IR.
    Exit1Ir,
    /// Pause-IR: shifting through the instruction register halts for a while.
    PauseIr,
    /// Exit2-IR.
    Exit2Ir,
    /// Update-IR: the shifted instruction becomes the current one.
    UpdateIr,
}

impl TapState {
    /// Every state, in the order of the enumeration.
    pub const ALL: [TapState; 16] = [
        TapState::TestLogicReset,
        TapState::RunTestIdle,
        TapState::SelectDrScan,
        TapState::CaptureDr,
        TapState::ShiftDr,
        TapState::Exit1Dr,
        TapState::PauseDr,
        TapState::Exit2Dr,
        TapState::UpdateDr,
        TapState::SelectIrScan,
        TapState::CaptureIr,
        TapState::ShiftIr,
        TapState::Exit1Ir,
        TapState::PauseIr,
        TapState::Exit2Ir,
        TapState::UpdateIr,
    ];

    /// The state's position in [`TapState::ALL`].
    pub const fn index(self) -> usize {
        self as usize
    }

    /// The state the controller moves to on a rising edge of TCK with TMS at `tms`, as the
    /// state diagram of IEEE 1149.1 says.
    pub const fn next(self, tms: bool) -> TapState {
        use TapState::*;
        match (self, tms) {
            (TestLogicReset, true) => TestLogicReset,
            (TestLogicReset, false) => RunTestIdle,
            (RunTestIdle, true) => SelectDrScan,
            (RunTestIdle, false) => RunTestIdle,
            (SelectDrScan, true) => SelectIrScan,
            (SelectDrScan, false) => CaptureDr,
            (CaptureDr, true) => Exit1Dr,
            (CaptureDr, false) => ShiftDr,
            (ShiftDr, true) => Exit1Dr,
            (ShiftDr, false) => ShiftDr,
            (Exit1Dr, true) => UpdateDr,
            (Exit1Dr, false) => PauseDr,
            (PauseDr, true) => Exit2Dr,
            (PauseDr, false) => PauseDr,
            (Exit2Dr, true) => UpdateDr,
            (Exit2Dr, false) => ShiftDr,
            (UpdateDr, true) => SelectDrScan,
            (UpdateDr, false) => RunTestIdle,
            (SelectIrScan, true) => TestLogicReset,
            (SelectIrScan, false) => CaptureIr,
            (CaptureIr, true) => Exit1Ir,
            (CaptureIr, false) => ShiftIr,
            (ShiftIr, true) => Exit1Ir,
            (ShiftIr, false) => ShiftIr,
            (Exit1Ir, true) => UpdateIr,
            (Exit1Ir, false) => PauseIr,
            (PauseIr, true) => Exit2Ir,
            (PauseIr, false) => PauseIr,
            (Exit2Ir, true) => UpdateIr,
            (Exit2Ir, false) => ShiftIr,
            (UpdateIr, true) => SelectDrScan,
            (UpdateIr, false) => RunTestIdle,
        }
    }

    /// The register whose column of the state diagram the state is in, Capture to Update;
    /// `None` for Test-Logic-Reset, Run-Test/Idle and the two Select states.
    pub const fn register(self) -> Option<Register> {
        use TapState::*;
        match self {
            CaptureDr | ShiftDr | Exit1Dr | PauseDr | Exit2Dr | UpdateDr => Some(Register::Dr),
            CaptureIr | ShiftIr | Exit1Ir | PauseIr | Exit2Ir | UpdateIr => Some(Register::Ir),
            TestLogicReset | RunTestIdle | SelectDrScan | SelectIrScan => None,
        }
    }

    /// Whether the state shifts a bit on every clock: Shift-IR or Shift-DR.
    pub const fn is_shift(self) -> bool {
        matches!(self, TapState::ShiftDr | TapState::ShiftIr)
    }

    /// Whether the state is Update-IR or Update-DR.
    pub const fn is_update(self) -> bool {
        matches!(self, TapState::UpdateDr | TapState::UpdateIr)
    }

    /// The shift state of `register`.
    pub const fn shift(register: Register) -> TapState {
        match register {
            Register::Ir => TapState::ShiftIr,
            Register::Dr => TapState::ShiftDr,
        }
    }
}

/// What reset the TAP controller.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ResetBy {
    /// TMS: the controller entered Test-Logic-Reset through the state diagram, or five clocks
    /// with TMS high put it there from a state the decoder did not know.
    Tms,
    /// The TRST line was asserted.
    Trst,
}

/// Why a scan ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EndReason {
    /// The controller went on to Update-IR or Update-DR: the shifted value took effect.
    Complete,
    /// The scan never reached its update: TRST reset the controller, or the samples ended or
    /// were lost while it was under way.
    Aborted,
}

/// What travels on a JTAG event stream.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum JtagEvent {
    /// The controller moved to another state.
    State {
        /// The state it is in from this clock on.
        state: TapState,
    },
    /// The controller was reset to Test-Logic-Reset.
    Reset {
        /// What reset it.
        by: ResetBy,
    },
    /// The controller entered a shift state with no scan under way: a scan begins.
    ScanBegin {
        /// The scan that begins.
        scan: ScanId,
        /// The register it shifts through.
        register: Register,
    },
    /// Bits shifted through the register, in the order they were shifted; a line that is not
    /// connected reads `None`.
    Shift {
        /// The scan the bits belong to.
        scan: ScanId,
        /// The position of the batch's first bit in the scan, in bits.
        offset: u32,
        /// The bits shifted in on TDI, packed least significant bit first: bit `i` of the
        /// batch is bit `i % 8` of byte `i / 8`.
        tdi: Option<Vec<u8>>,
        /// The bits shifted out on TDO, packed the same way.
        tdo: Option<Vec<u8>>,
        /// How many bits the batch holds, 1 to [`SHIFT_BATCH_BITS`].
        bits: u16,
    },
    /// The scan ended.
    ScanEnd {
        /// The scan that ends.
        scan: ScanId,
        /// How many bits it shifted in all.
        bits: u32,
        /// Why it ended.
        reason: EndReason,
    },
}

impl EventSchema for JtagEvent {
    const SCHEMA: SchemaId = JTAG_EVENTS;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json(event: &JtagEvent) -> String {
        serde_json::to_string(event).expect("serializes")
    }

    #[test]
    fn serializes_as_the_contract_spells_it() {
        assert_eq!(
            json(&JtagEvent::State {
                state: TapState::Exit1Dr
            }),
            r#"{"kind":"state","state":"exit1-dr"}"#
        );
        assert_eq!(
            json(&JtagEvent::Reset { by: ResetBy::Trst }),
            r#"{"kind":"reset","by":"trst"}"#
        );
        assert_eq!(
            json(&JtagEvent::ScanBegin {
                scan: ScanId(2),
                register: Register::Ir
            }),
            r#"{"kind":"scan-begin","scan":2,"register":"ir"}"#
        );
        assert_eq!(
            json(&JtagEvent::Shift {
                scan: ScanId(2),
                offset: 256,
                tdi: Some(vec![0x0f, 0x01]),
                tdo: None,
                bits: 9
            }),
            r#"{"kind":"shift","scan":2,"offset":256,"tdi":[15,1],"tdo":null,"bits":9}"#
        );
        assert_eq!(
            json(&JtagEvent::ScanEnd {
                scan: ScanId(2),
                bits: 265,
                reason: EndReason::Complete
            }),
            r#"{"kind":"scan-end","scan":2,"bits":265,"reason":"complete"}"#
        );
    }

    #[test]
    fn states_are_named_as_the_standard_names_them() {
        let expected = [
            "test-logic-reset",
            "run-test-idle",
            "select-dr-scan",
            "capture-dr",
            "shift-dr",
            "exit1-dr",
            "pause-dr",
            "exit2-dr",
            "update-dr",
            "select-ir-scan",
            "capture-ir",
            "shift-ir",
            "exit1-ir",
            "pause-ir",
            "exit2-ir",
            "update-ir",
        ];
        for (index, state) in TapState::ALL.iter().enumerate() {
            assert_eq!(state.index(), index);
            assert_eq!(
                json(&JtagEvent::State { state: *state }),
                format!(r#"{{"kind":"state","state":"{}"}}"#, expected[index])
            );
        }
    }

    #[test]
    fn five_clocks_with_tms_high_reset_from_every_state() {
        for state in TapState::ALL {
            let mut at = state;
            for _ in 0..5 {
                at = at.next(true);
            }
            assert_eq!(at, TapState::TestLogicReset, "from {state:?}");
        }
        // Four are not always enough.
        let mut at = TapState::ShiftDr;
        for _ in 0..4 {
            at = at.next(true);
        }
        assert_eq!(at, TapState::SelectIrScan);
    }

    #[test]
    fn the_two_columns_mirror_each_other() {
        // The IR column is the DR column seven places further on in `ALL`.
        let mirror = |state: TapState| match state.register() {
            Some(Register::Dr) => TapState::ALL[state.index() + 7],
            Some(Register::Ir) => TapState::ALL[state.index() - 7],
            None => state,
        };
        for state in TapState::ALL {
            if state.register().is_none() {
                continue;
            }
            for tms in [false, true] {
                let next = state.next(tms);
                if state.is_update() {
                    // Both updates lead out of the columns to the same state.
                    assert_eq!(next, mirror(state).next(tms), "{state:?}");
                } else {
                    assert_eq!(mirror(next), mirror(state).next(tms), "{state:?}, {tms}");
                }
            }
        }
        assert_eq!(TapState::shift(Register::Ir), TapState::ShiftIr);
        assert!(TapState::ShiftDr.is_shift() && !TapState::PauseDr.is_shift());
        assert!(TapState::UpdateIr.is_update() && !TapState::Exit2Ir.is_update());
        assert_eq!(TapState::SelectDrScan.register(), None);
        assert_eq!(TapState::Exit2Ir.register(), Some(Register::Ir));
    }
}
