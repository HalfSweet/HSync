// SPDX-License-Identifier: Apache-2.0

//! Generic JTAG contract declared by the TypeScript chain selector.
use uiharu_core::SchemaId;
pub use uiharu_dsl_protocols::jtag_chain::{EndReason, JtagEvent, Register, ResetBy, TapState};
/// Schema of generic JTAG transport events.
pub const JTAG_EVENTS: SchemaId = SchemaId::new("jtag.events", 1);
/// Bits in the existing physical decoder's largest shift batch.
pub const SHIFT_BATCH_BITS: u16 = 256;
pub(crate) const SHIFT_BATCH_BYTES: usize = SHIFT_BATCH_BITS as usize / 8;
/// Scan identity, serialized as the same unsigned integer as before.
pub type ScanId = u64;
/// IEEE1149.1 state operations, kept separate from the generated event DTO.
pub trait TapStateExt: Sized {
    /// All states in the state-machine table order.
    const ALL: [TapState; 16];
    /// State-table index.
    fn index(self) -> usize;
    /// State reached at the next rising TCK edge.
    fn next(self, tms: bool) -> TapState;
    /// Register selected in the capture-through-update columns.
    fn register(self) -> Option<Register>;
    /// Whether this state shifts data.
    fn is_shift(&self) -> bool;
    /// Whether this state commits shifted data.
    fn is_update(&self) -> bool;
    /// Shift state for a register.
    fn shift(register: Register) -> TapState;
}
impl TapStateExt for TapState {
    /// Every state, in the order of the enumeration.
    const ALL: [TapState; 16] = [
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
    fn index(self) -> usize {
        self as usize
    }

    /// The state the controller moves to on a rising edge of TCK with TMS at `tms`, as the
    /// state diagram of IEEE 1149.1 says.
    fn next(self, tms: bool) -> TapState {
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
    fn register(self) -> Option<Register> {
        use TapState::*;
        match self {
            CaptureDr | ShiftDr | Exit1Dr | PauseDr | Exit2Dr | UpdateDr => Some(Register::Dr),
            CaptureIr | ShiftIr | Exit1Ir | PauseIr | Exit2Ir | UpdateIr => Some(Register::Ir),
            TestLogicReset | RunTestIdle | SelectDrScan | SelectIrScan => None,
        }
    }

    /// Whether the state shifts a bit on every clock: Shift-IR or Shift-DR.
    fn is_shift(&self) -> bool {
        matches!(self, TapState::ShiftDr | TapState::ShiftIr)
    }

    /// Whether the state is Update-IR or Update-DR.
    fn is_update(&self) -> bool {
        matches!(self, TapState::UpdateDr | TapState::UpdateIr)
    }

    /// The shift state of `register`.
    fn shift(register: Register) -> TapState {
        match register {
            Register::Ir => TapState::ShiftIr,
            Register::Dr => TapState::ShiftDr,
        }
    }
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
                scan: 2,
                register: Register::Ir
            }),
            r#"{"kind":"scan-begin","scan":2,"register":"ir"}"#
        );
        assert_eq!(
            json(&JtagEvent::Shift {
                scan: 2,
                offset: 256,
                tdi: Some(vec![0x0f, 0x01]),
                tdo: None,
                bits: 9
            }),
            r#"{"kind":"shift","scan":2,"offset":256,"tdi":[15,1],"tdo":null,"bits":9}"#
        );
        assert_eq!(
            json(&JtagEvent::ScanEnd {
                scan: 2,
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
