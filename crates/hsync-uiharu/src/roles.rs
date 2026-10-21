//! What each event is to someone reading the bus, for the interface to colour it by: the role
//! its kind has in the decoder's manifest, an error when it meets one of the kind's error
//! conditions, and for JTAG the register a scan shifts and the column of a TAP state.

use std::collections::VecDeque;

use serde::Serialize;
use serde_json::Value;
use ts_rs::TS;
use uiharu_registry::{EventKind, Role};

/// What an event is to someone reading the bus, whatever the protocol (uiharu's [`Role`]).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum EventRole {
    /// Where a transaction, frame or scan begins or ends.
    Framing,
    /// Whom the traffic is for.
    Address,
    /// What is asked.
    Command,
    /// What a frame says of itself.
    Control,
    /// What the traffic carries; also what an event is whose kind declares no role.
    #[default]
    Data,
    /// A CRC or a checksum.
    Check,
    /// The other side's answer.
    Handshake,
    /// A condition of the line itself.
    Line,
    /// Something the decoder reports as wrong.
    Error,
}

impl EventRole {
    const COUNT: usize = 9;
    const ALL: [EventRole; Self::COUNT] = [
        EventRole::Framing,
        EventRole::Address,
        EventRole::Command,
        EventRole::Control,
        EventRole::Data,
        EventRole::Check,
        EventRole::Handshake,
        EventRole::Line,
        EventRole::Error,
    ];
}

impl From<Role> for EventRole {
    fn from(role: Role) -> Self {
        match role {
            Role::Framing => EventRole::Framing,
            Role::Address => EventRole::Address,
            Role::Command => EventRole::Command,
            Role::Control => EventRole::Control,
            Role::Data => EventRole::Data,
            Role::Check => EventRole::Check,
            Role::Handshake => EventRole::Handshake,
            Role::Line => EventRole::Line,
            Role::Error => EventRole::Error,
        }
    }
}

/// A stretch of a lane or of the overview strip in one role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct RolePart {
    pub start: u64,
    pub end: u64,
    pub role: EventRole,
}

/// The roles of a stream's kinds of event, from its decoder's manifest.
#[derive(Debug, Default)]
pub(crate) struct KindRoles(Vec<EventKind>);

impl KindRoles {
    pub(crate) fn new(events: Vec<EventKind>) -> Self {
        Self(events)
    }

    /// The role of `payload`, an event of kind `kind`.
    pub(crate) fn of(&self, kind: &str, payload: &Value) -> EventRole {
        self.0
            .iter()
            .find(|event| event.kind == kind)
            .map_or(EventRole::Data, |event| event.role_of(payload).into())
    }
}

/// The register the latest JTAG scan shifts, so that its shifts and the markers of its begin and
/// end take the register's role: an instruction is a command, what a data register holds is
/// data. A marker is then told apart as an instruction or a data scan, however thin.
#[derive(Debug, Default)]
pub(crate) struct ScanRegisters {
    last: Option<(Value, EventRole)>,
}

impl ScanRegisters {
    /// The role of a JTAG event of kind `kind` whose kind's role is `role`.
    pub(crate) fn role(&mut self, kind: &str, payload: &Value, role: EventRole) -> EventRole {
        if role == EventRole::Error {
            return role;
        }
        match kind {
            "scan-begin" => {
                let register = match payload.get("register").and_then(Value::as_str) {
                    Some("ir") => EventRole::Command,
                    _ => EventRole::Data,
                };
                self.last = payload.get("scan").cloned().map(|scan| (scan, register));
                register
            }
            "shift" | "scan-end" => match &self.last {
                Some((scan, register)) if payload.get("scan") == Some(scan) => *register,
                _ => role,
            },
            _ => role,
        }
    }
}

/// The role of a TAP state: the column of the state diagram it lies in. Reset and idle frame
/// the scans; the instruction register's states are commands, the data register's data.
pub(crate) fn tap_role(state: &str) -> EventRole {
    if state.contains("-ir") {
        EventRole::Command
    } else if state.contains("-dr") {
        EventRole::Data
    } else {
        EventRole::Framing
    }
}

/// Paints events onto columns of `unit` ticks and gives each column the role covering most of
/// it, or an error's when one lies there; neighbouring columns of one role run together.
pub(crate) struct RolePainter {
    unit: u64,
    /// The column of `open[0]`.
    first: u64,
    /// The ticks each role covers in the columns events may still reach, in order.
    open: VecDeque<[u64; EventRole::COUNT]>,
    parts: Vec<RolePart>,
}

impl RolePainter {
    pub(crate) fn new(unit: u64) -> Self {
        Self {
            unit: unit.max(1),
            first: 0,
            open: VecDeque::new(),
            parts: Vec::new(),
        }
    }

    /// Adds an event from `start` to `reach`, events coming in order of their start.
    pub(crate) fn add(&mut self, start: u64, reach: u64, role: EventRole) {
        let reach = reach.max(start.saturating_add(1));
        let from = start / self.unit;
        let to = (reach - 1) / self.unit;
        // Nothing reaches back before the start of this event.
        while self.first < from
            && let Some(cover) = self.open.pop_front()
        {
            self.close(self.first, cover);
            self.first += 1;
        }
        if self.open.is_empty() {
            self.first = from;
        }
        for column in from.max(self.first)..=to {
            let index = (column - self.first) as usize;
            if index == self.open.len() {
                self.open.push_back([0; EventRole::COUNT]);
            }
            let low = (column * self.unit).max(start);
            let high = ((column + 1) * self.unit).min(reach);
            self.open[index][role as usize] += high.saturating_sub(low).max(1);
        }
    }

    fn close(&mut self, column: u64, cover: [u64; EventRole::COUNT]) {
        let role = if cover[EventRole::Error as usize] > 0 {
            Some(EventRole::Error)
        } else {
            EventRole::ALL
                .into_iter()
                .zip(cover)
                .filter(|&(_, ticks)| ticks > 0)
                .max_by_key(|&(_, ticks)| ticks)
                .map(|(role, _)| role)
        };
        let Some(role) = role else {
            return;
        };
        let start = column * self.unit;
        let end = start + self.unit;
        match self.parts.last_mut() {
            Some(last) if last.role == role && last.end == start => last.end = end,
            _ => self.parts.push(RolePart { start, end, role }),
        }
    }

    /// The parts painted, cut to `start..end`.
    pub(crate) fn finish(mut self, start: u64, end: u64) -> Vec<RolePart> {
        while let Some(cover) = self.open.pop_front() {
            self.close(self.first, cover);
            self.first += 1;
        }
        let mut parts = self.parts;
        for part in &mut parts {
            part.start = part.start.max(start);
            part.end = part.end.min(end);
        }
        parts.retain(|part| part.end > part.start);
        parts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(start: u64, end: u64, role: EventRole) -> RolePart {
        RolePart { start, end, role }
    }

    #[test]
    fn each_column_takes_the_role_covering_most_of_it() {
        let mut painter = RolePainter::new(10);
        painter.add(0, 7, EventRole::Address);
        painter.add(7, 10, EventRole::Data);
        painter.add(10, 18, EventRole::Data);
        painter.add(20, 22, EventRole::Data);
        painter.add(40, 41, EventRole::Check);
        assert_eq!(
            painter.finish(0, 45),
            [
                part(0, 10, EventRole::Address),
                part(10, 30, EventRole::Data),
                part(40, 45, EventRole::Check),
            ]
        );
    }

    #[test]
    fn an_error_takes_its_column_and_a_wide_event_every_column_it_crosses() {
        let mut painter = RolePainter::new(10);
        painter.add(0, 9, EventRole::Data);
        painter.add(9, 10, EventRole::Error);
        painter.add(12, 45, EventRole::Line);
        assert_eq!(
            painter.finish(5, 100),
            [part(5, 10, EventRole::Error), part(10, 50, EventRole::Line)]
        );
    }

    #[test]
    fn tap_states_take_the_role_of_their_column() {
        assert_eq!(tap_role("run-test-idle"), EventRole::Framing);
        assert_eq!(tap_role("test-logic-reset"), EventRole::Framing);
        assert_eq!(tap_role("select-ir-scan"), EventRole::Command);
        assert_eq!(tap_role("shift-ir"), EventRole::Command);
        assert_eq!(tap_role("select-dr-scan"), EventRole::Data);
        assert_eq!(tap_role("exit2-dr"), EventRole::Data);
    }

    #[test]
    fn a_scan_takes_the_role_of_its_register_from_begin_to_end() {
        let mut scans = ScanRegisters::default();
        let begin = |scan: u64, register: &str| serde_json::json!({ "kind": "scan-begin", "scan": scan, "register": register });
        let shift = |scan: u64| serde_json::json!({ "kind": "shift", "scan": scan });
        let end = |scan: u64| serde_json::json!({ "kind": "scan-end", "scan": scan });
        let mut roles = |kind: &str, payload: serde_json::Value, role: EventRole| {
            scans.role(kind, &payload, role)
        };
        assert_eq!(
            [
                roles("scan-begin", begin(1, "ir"), EventRole::Framing),
                roles("shift", shift(1), EventRole::Data),
                roles("scan-end", end(1), EventRole::Framing),
                roles("scan-begin", begin(2, "dr"), EventRole::Framing),
                roles("shift", shift(2), EventRole::Data),
                roles("scan-end", end(2), EventRole::Framing),
            ],
            [
                EventRole::Command,
                EventRole::Command,
                EventRole::Command,
                EventRole::Data,
                EventRole::Data,
                EventRole::Data,
            ]
        );
        // Events of a scan other than the latest keep the role of their kind; so do errors.
        assert_eq!(
            roles("scan-end", end(1), EventRole::Framing),
            EventRole::Framing
        );
        assert_eq!(roles("shift", shift(2), EventRole::Error), EventRole::Error);
    }
}
