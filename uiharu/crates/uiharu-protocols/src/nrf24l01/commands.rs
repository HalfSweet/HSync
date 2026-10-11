// SPDX-License-Identifier: Apache-2.0

//! The command set and the register map: which byte is which command, what each command moves
//! after its command byte, and which address is which register.

use super::events::{Command, Direction, Register};

/// The most bytes a payload command moves.
pub const MAX_PAYLOAD: u8 = 32;

/// The most bytes a register command moves: the five of an address register.
pub const MAX_REGISTER_BYTES: u8 = 5;

/// ACTIVATE's command byte, a command only when [`ACTIVATE_KEY`] follows.
pub const ACTIVATE: u8 = 0x50;

/// The byte after [`ACTIVATE`] that makes it the ACTIVATE command.
pub const ACTIVATE_KEY: u8 = 0x73;

/// What a command byte names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Decoded {
    pub(crate) command: Command,
    /// The register address of R_REGISTER and W_REGISTER.
    pub(crate) register: Option<u8>,
    /// The pipe of W_ACK_PAYLOAD.
    pub(crate) pipe: Option<u8>,
}

/// What a command moves after its command byte: which way, and how many bytes it takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Payload {
    /// Which way the data moves; `None` for a command without data.
    pub(crate) direction: Option<Direction>,
    /// The fewest bytes the command takes.
    pub(crate) min: u8,
    /// The most.
    pub(crate) max: u8,
}

impl Payload {
    const NONE: Payload = Payload {
        direction: None,
        min: 0,
        max: 0,
    };

    const fn of(direction: Direction, max: u8) -> Payload {
        Payload {
            direction: Some(direction),
            min: 1,
            max,
        }
    }
}

/// The command `byte` names, with its register or pipe; `None` for a byte that names no
/// command, ACTIVATE's included, which takes its key to be one.
pub(crate) const fn decode(byte: u8) -> Option<Decoded> {
    let (command, register, pipe) = match byte {
        0x00..=0x1f => (Command::RRegister, Some(byte & 0x1f), None),
        0x20..=0x3f => (Command::WRegister, Some(byte & 0x1f), None),
        0x60 => (Command::RRxPlWid, None, None),
        0x61 => (Command::RRxPayload, None, None),
        0xa0 => (Command::WTxPayload, None, None),
        // Pipes 0 to 5: PPP of 110 and 111 names none.
        0xa8..=0xad => (Command::WAckPayload, None, Some(byte & 0x07)),
        0xb0 => (Command::WTxPayloadNoack, None, None),
        0xe1 => (Command::FlushTx, None, None),
        0xe2 => (Command::FlushRx, None, None),
        0xe3 => (Command::ReuseTxPl, None, None),
        0xff => (Command::Nop, None, None),
        _ => return None,
    };
    Some(Decoded {
        command,
        register,
        pipe,
    })
}

impl Command {
    /// Every command, in the order of the contract.
    pub const ALL: [Command; 12] = [
        Command::RRegister,
        Command::WRegister,
        Command::RRxPayload,
        Command::WTxPayload,
        Command::FlushTx,
        Command::FlushRx,
        Command::ReuseTxPl,
        Command::RRxPlWid,
        Command::WAckPayload,
        Command::WTxPayloadNoack,
        Command::Nop,
        Command::Activate,
    ];

    /// The command's byte; for the register commands the address, and for W_ACK_PAYLOAD the
    /// pipe, go in its low bits.
    pub const fn byte(self) -> u8 {
        match self {
            Command::RRegister => 0x00,
            Command::WRegister => 0x20,
            Command::RRxPayload => 0x61,
            Command::WTxPayload => 0xa0,
            Command::FlushTx => 0xe1,
            Command::FlushRx => 0xe2,
            Command::ReuseTxPl => 0xe3,
            Command::RRxPlWid => 0x60,
            Command::WAckPayload => 0xa8,
            Command::WTxPayloadNoack => 0xb0,
            Command::Nop => 0xff,
            Command::Activate => ACTIVATE,
        }
    }

    /// What the command moves after its byte; `register` is the address of a register
    /// command.
    pub(crate) const fn payload(self, register: Option<u8>) -> Payload {
        match self {
            Command::RRegister => Payload::of(Direction::Out, register_bytes(register)),
            Command::WRegister => Payload::of(Direction::In, register_bytes(register)),
            Command::RRxPayload => Payload::of(Direction::Out, MAX_PAYLOAD),
            Command::WTxPayload | Command::WAckPayload | Command::WTxPayloadNoack => {
                Payload::of(Direction::In, MAX_PAYLOAD)
            }
            Command::RRxPlWid => Payload::of(Direction::Out, 1),
            // The key is its one byte.
            Command::Activate => Payload::of(Direction::In, 1),
            Command::FlushTx | Command::FlushRx | Command::ReuseTxPl | Command::Nop => {
                Payload::NONE
            }
        }
    }
}

/// The most bytes a register command moves for the register at `address`: the register's
/// width, or the commands' five when the address is not in the map.
const fn register_bytes(address: Option<u8>) -> u8 {
    match address {
        Some(address) => match Register::from_address(address) {
            Some(register) => register.width(),
            None => MAX_REGISTER_BYTES,
        },
        None => MAX_REGISTER_BYTES,
    }
}

impl Register {
    /// Every register, in address order.
    pub const ALL: [Register; 26] = [
        Register::Config,
        Register::EnAa,
        Register::EnRxaddr,
        Register::SetupAw,
        Register::SetupRetr,
        Register::RfCh,
        Register::RfSetup,
        Register::Status,
        Register::ObserveTx,
        Register::Rpd,
        Register::RxAddrP0,
        Register::RxAddrP1,
        Register::RxAddrP2,
        Register::RxAddrP3,
        Register::RxAddrP4,
        Register::RxAddrP5,
        Register::TxAddr,
        Register::RxPwP0,
        Register::RxPwP1,
        Register::RxPwP2,
        Register::RxPwP3,
        Register::RxPwP4,
        Register::RxPwP5,
        Register::FifoStatus,
        Register::Dynpd,
        Register::Feature,
    ];

    /// The register's address.
    pub const fn address(self) -> u8 {
        match self {
            Register::Config => 0x00,
            Register::EnAa => 0x01,
            Register::EnRxaddr => 0x02,
            Register::SetupAw => 0x03,
            Register::SetupRetr => 0x04,
            Register::RfCh => 0x05,
            Register::RfSetup => 0x06,
            Register::Status => 0x07,
            Register::ObserveTx => 0x08,
            Register::Rpd => 0x09,
            Register::RxAddrP0 => 0x0a,
            Register::RxAddrP1 => 0x0b,
            Register::RxAddrP2 => 0x0c,
            Register::RxAddrP3 => 0x0d,
            Register::RxAddrP4 => 0x0e,
            Register::RxAddrP5 => 0x0f,
            Register::TxAddr => 0x10,
            Register::RxPwP0 => 0x11,
            Register::RxPwP1 => 0x12,
            Register::RxPwP2 => 0x13,
            Register::RxPwP3 => 0x14,
            Register::RxPwP4 => 0x15,
            Register::RxPwP5 => 0x16,
            Register::FifoStatus => 0x17,
            Register::Dynpd => 0x1c,
            Register::Feature => 0x1d,
        }
    }

    /// The register at `address`, if the map has one there.
    pub const fn from_address(address: u8) -> Option<Register> {
        Some(match address {
            0x00 => Register::Config,
            0x01 => Register::EnAa,
            0x02 => Register::EnRxaddr,
            0x03 => Register::SetupAw,
            0x04 => Register::SetupRetr,
            0x05 => Register::RfCh,
            0x06 => Register::RfSetup,
            0x07 => Register::Status,
            0x08 => Register::ObserveTx,
            0x09 => Register::Rpd,
            0x0a => Register::RxAddrP0,
            0x0b => Register::RxAddrP1,
            0x0c => Register::RxAddrP2,
            0x0d => Register::RxAddrP3,
            0x0e => Register::RxAddrP4,
            0x0f => Register::RxAddrP5,
            0x10 => Register::TxAddr,
            0x11 => Register::RxPwP0,
            0x12 => Register::RxPwP1,
            0x13 => Register::RxPwP2,
            0x14 => Register::RxPwP3,
            0x15 => Register::RxPwP4,
            0x16 => Register::RxPwP5,
            0x17 => Register::FifoStatus,
            0x1c => Register::Dynpd,
            0x1d => Register::Feature,
            _ => return None,
        })
    }

    /// The register's width in bytes: five for the addresses of pipes 0 and 1 and TX_ADDR,
    /// as many as SETUP_AW sets (3 to 5) being used; one for the others.
    pub const fn width(self) -> u8 {
        match self {
            Register::RxAddrP0 | Register::RxAddrP1 | Register::TxAddr => MAX_REGISTER_BYTES,
            _ => 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_bytes_map_both_ways() {
        for command in Command::ALL {
            if command == Command::Activate {
                // A command only with its key.
                assert_eq!(decode(command.byte()), None);
                continue;
            }
            let decoded = decode(command.byte()).expect("a command");
            assert_eq!(decoded.command, command);
        }
        let known = (0..=255u8).filter(|byte| decode(*byte).is_some()).count();
        // 32 register reads, 32 register writes, six ack payload pipes, eight others.
        assert_eq!(known, 32 + 32 + 6 + 8);
    }

    #[test]
    fn register_commands_carry_their_address_and_ack_payloads_their_pipe() {
        let read = decode(0x07).expect("R_REGISTER");
        assert_eq!((read.command, read.register), (Command::RRegister, Some(7)));
        let write = decode(0x3d).expect("W_REGISTER");
        assert_eq!(
            (write.command, write.register),
            (Command::WRegister, Some(0x1d))
        );
        for pipe in 0..6 {
            let ack = decode(0xa8 | pipe).expect("W_ACK_PAYLOAD");
            assert_eq!((ack.command, ack.pipe), (Command::WAckPayload, Some(pipe)));
        }
        for byte in [0xae, 0xaf, 0x40, 0x50, 0x62, 0xe0, 0xe4, 0xfe, 0xa1, 0xb1] {
            assert_eq!(decode(byte), None, "{byte:#04x}");
        }
    }

    #[test]
    fn the_register_map_has_26_registers() {
        for register in Register::ALL {
            assert_eq!(Register::from_address(register.address()), Some(register));
        }
        let mapped = (0..32u8)
            .filter(|address| Register::from_address(*address).is_some())
            .count();
        assert_eq!(mapped, 26);
        for unmapped in [0x18, 0x19, 0x1a, 0x1b, 0x1e, 0x1f] {
            assert_eq!(Register::from_address(unmapped), None);
        }
    }

    #[test]
    fn payloads_follow_the_command_and_the_register() {
        let payload = |command: Command, register| command.payload(register);
        assert_eq!(payload(Command::WRegister, Some(0x00)).max, 1);
        assert_eq!(payload(Command::WRegister, Some(0x10)).max, 5);
        assert_eq!(payload(Command::RRegister, Some(0x0b)).max, 5);
        assert_eq!(payload(Command::RRegister, Some(0x0c)).max, 1);
        assert_eq!(payload(Command::RRegister, Some(0x1e)).max, 5);
        assert_eq!(
            payload(Command::RRegister, Some(0x07)).direction,
            Some(Direction::Out)
        );
        assert_eq!(
            payload(Command::WTxPayloadNoack, None),
            Payload {
                direction: Some(Direction::In),
                min: 1,
                max: 32
            }
        );
        assert_eq!(payload(Command::RRxPlWid, None).max, 1);
        for command in [
            Command::FlushTx,
            Command::FlushRx,
            Command::ReuseTxPl,
            Command::Nop,
        ] {
            assert_eq!(payload(command, None), Payload::NONE);
        }
    }
}
