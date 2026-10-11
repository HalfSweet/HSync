// SPDX-License-Identifier: Apache-2.0

//! The `usbpd.events` contract: what the USB Power Delivery decoder emits and what decoders
//! built on USB PD messages read.
//!
//! The names follow the Universal Serial Bus Power Delivery Specification, revision 3.1
//! (usb.org): the ordered sets of §5.4, the message header and the extended message header of
//! §6.2.1, the message types of §6.3 (control), §6.4 (data) and §6.5 (extended), and the power
//! and request data objects of §6.4.1 and §6.4.2.

use serde::{Deserialize, Serialize};
use uiharu_core::{EventSchema, SchemaId};

/// The schema of USB PD event streams.
pub const USB_PD_EVENTS: SchemaId = SchemaId::new("usbpd.events", 1);

/// The most bytes one `data` event carries.
pub const DATA_BATCH: usize = 32;

/// A packet, numbered from 1 by the decoder in the order packets begin on the line.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct PacketId(pub u64);

/// The ordered set that starts a message: which port or cable plug it is addressed to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OrderedSet {
    /// SOP: Sync-1 Sync-1 Sync-1 Sync-2, between the ports at both ends of the cable.
    Sop,
    /// SOP': Sync-1 Sync-1 Sync-3 Sync-3, to or from a cable plug.
    SopPrime,
    /// SOP'': Sync-1 Sync-3 Sync-1 Sync-3, to or from a cable's second cable plug.
    SopDoublePrime,
    /// SOP'_Debug: Sync-1 RST-2 RST-2 Sync-3.
    SopPrimeDebug,
    /// SOP''_Debug: Sync-1 RST-2 Sync-3 Sync-2.
    SopDoublePrimeDebug,
}

/// The message types of control messages (§6.3): messages without data objects.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ControlMessage {
    /// 1: GoodCRC.
    GoodCrc,
    /// 2: GotoMin.
    GotoMin,
    /// 3: Accept.
    Accept,
    /// 4: Reject.
    Reject,
    /// 5: Ping.
    Ping,
    /// 6: PS_RDY.
    PsRdy,
    /// 7: Get_Source_Cap.
    GetSourceCap,
    /// 8: Get_Sink_Cap.
    GetSinkCap,
    /// 9: DR_Swap.
    DrSwap,
    /// 10: PR_Swap.
    PrSwap,
    /// 11: VCONN_Swap.
    VconnSwap,
    /// 12: Wait.
    Wait,
    /// 13: Soft_Reset.
    SoftReset,
    /// 14: Data_Reset.
    DataReset,
    /// 15: Data_Reset_Complete.
    DataResetComplete,
    /// 16: Not_Supported.
    NotSupported,
    /// 17: Get_Source_Cap_Extended.
    GetSourceCapExtended,
    /// 18: Get_Status.
    GetStatus,
    /// 19: FR_Swap.
    FrSwap,
    /// 20: Get_PPS_Status.
    GetPpsStatus,
    /// 21: Get_Country_Codes.
    GetCountryCodes,
    /// 22: Get_Sink_Cap_Extended.
    GetSinkCapExtended,
    /// 23: Get_Source_Info.
    GetSourceInfo,
    /// 24: Get_Revision.
    GetRevision,
}

impl ControlMessage {
    /// Every control message, in the order of their types, 1 to 24.
    pub const ALL: [ControlMessage; 24] = [
        ControlMessage::GoodCrc,
        ControlMessage::GotoMin,
        ControlMessage::Accept,
        ControlMessage::Reject,
        ControlMessage::Ping,
        ControlMessage::PsRdy,
        ControlMessage::GetSourceCap,
        ControlMessage::GetSinkCap,
        ControlMessage::DrSwap,
        ControlMessage::PrSwap,
        ControlMessage::VconnSwap,
        ControlMessage::Wait,
        ControlMessage::SoftReset,
        ControlMessage::DataReset,
        ControlMessage::DataResetComplete,
        ControlMessage::NotSupported,
        ControlMessage::GetSourceCapExtended,
        ControlMessage::GetStatus,
        ControlMessage::FrSwap,
        ControlMessage::GetPpsStatus,
        ControlMessage::GetCountryCodes,
        ControlMessage::GetSinkCapExtended,
        ControlMessage::GetSourceInfo,
        ControlMessage::GetRevision,
    ];

    /// The control message of message type `value`; `None` for a reserved type.
    pub const fn from_type(value: u8) -> Option<ControlMessage> {
        match value {
            1..=24 => Some(Self::ALL[value as usize - 1]),
            _ => None,
        }
    }

    /// The message type.
    pub const fn message_type(self) -> u8 {
        self as u8 + 1
    }
}

/// The message types of data messages (§6.4): messages with data objects that are not extended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DataMessage {
    /// 1: Source_Capabilities.
    SourceCapabilities,
    /// 2: Request.
    Request,
    /// 3: BIST.
    Bist,
    /// 4: Sink_Capabilities.
    SinkCapabilities,
    /// 5: Battery_Status.
    BatteryStatus,
    /// 6: Alert.
    Alert,
    /// 7: Get_Country_Info.
    GetCountryInfo,
    /// 8: Enter_USB.
    EnterUsb,
    /// 9: EPR_Request.
    EprRequest,
    /// 10: EPR_Mode.
    EprMode,
    /// 11: Source_Info.
    SourceInfo,
    /// 12: Revision.
    Revision,
    /// 15: Vendor_Defined.
    VendorDefined,
}

impl DataMessage {
    /// Every data message, in the order of their types.
    pub const ALL: [DataMessage; 13] = [
        DataMessage::SourceCapabilities,
        DataMessage::Request,
        DataMessage::Bist,
        DataMessage::SinkCapabilities,
        DataMessage::BatteryStatus,
        DataMessage::Alert,
        DataMessage::GetCountryInfo,
        DataMessage::EnterUsb,
        DataMessage::EprRequest,
        DataMessage::EprMode,
        DataMessage::SourceInfo,
        DataMessage::Revision,
        DataMessage::VendorDefined,
    ];

    /// The data message of message type `value`; `None` for a reserved type.
    pub const fn from_type(value: u8) -> Option<DataMessage> {
        match value {
            1..=12 => Some(Self::ALL[value as usize - 1]),
            15 => Some(DataMessage::VendorDefined),
            _ => None,
        }
    }

    /// The message type.
    pub const fn message_type(self) -> u8 {
        match self {
            DataMessage::VendorDefined => 15,
            other => other as u8 + 1,
        }
    }
}

/// The message types of extended messages (§6.5): messages whose header has the Extended bit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExtendedMessage {
    /// 1: Source_Capabilities_Extended.
    SourceCapabilitiesExtended,
    /// 2: Status.
    Status,
    /// 3: Get_Battery_Cap.
    GetBatteryCap,
    /// 4: Get_Battery_Status.
    GetBatteryStatus,
    /// 5: Battery_Capabilities.
    BatteryCapabilities,
    /// 6: Get_Manufacturer_Info.
    GetManufacturerInfo,
    /// 7: Manufacturer_Info.
    ManufacturerInfo,
    /// 8: Security_Request.
    SecurityRequest,
    /// 9: Security_Response.
    SecurityResponse,
    /// 10: Firmware_Update_Request.
    FirmwareUpdateRequest,
    /// 11: Firmware_Update_Response.
    FirmwareUpdateResponse,
    /// 12: PPS_Status.
    PpsStatus,
    /// 13: Country_Info.
    CountryInfo,
    /// 14: Country_Codes.
    CountryCodes,
    /// 15: Sink_Capabilities_Extended.
    SinkCapabilitiesExtended,
    /// 16: Extended_Control.
    ExtendedControl,
    /// 17: EPR_Source_Capabilities.
    EprSourceCapabilities,
    /// 18: EPR_Sink_Capabilities.
    EprSinkCapabilities,
    /// 30: Vendor_Defined_Extended.
    VendorDefinedExtended,
}

impl ExtendedMessage {
    /// Every extended message, in the order of their types.
    pub const ALL: [ExtendedMessage; 19] = [
        ExtendedMessage::SourceCapabilitiesExtended,
        ExtendedMessage::Status,
        ExtendedMessage::GetBatteryCap,
        ExtendedMessage::GetBatteryStatus,
        ExtendedMessage::BatteryCapabilities,
        ExtendedMessage::GetManufacturerInfo,
        ExtendedMessage::ManufacturerInfo,
        ExtendedMessage::SecurityRequest,
        ExtendedMessage::SecurityResponse,
        ExtendedMessage::FirmwareUpdateRequest,
        ExtendedMessage::FirmwareUpdateResponse,
        ExtendedMessage::PpsStatus,
        ExtendedMessage::CountryInfo,
        ExtendedMessage::CountryCodes,
        ExtendedMessage::SinkCapabilitiesExtended,
        ExtendedMessage::ExtendedControl,
        ExtendedMessage::EprSourceCapabilities,
        ExtendedMessage::EprSinkCapabilities,
        ExtendedMessage::VendorDefinedExtended,
    ];

    /// The extended message of message type `value`; `None` for a reserved type.
    pub const fn from_type(value: u8) -> Option<ExtendedMessage> {
        match value {
            1..=18 => Some(Self::ALL[value as usize - 1]),
            30 => Some(ExtendedMessage::VendorDefinedExtended),
            _ => None,
        }
    }

    /// The message type.
    pub const fn message_type(self) -> u8 {
        match self {
            ExtendedMessage::VendorDefinedExtended => 30,
            other => other as u8 + 1,
        }
    }
}

/// The data role a port states in the headers of its SOP messages (bit 5).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DataRole {
    /// 0: Upstream Facing Port.
    Ufp,
    /// 1: Downstream Facing Port.
    Dfp,
}

/// The power role a port states in the headers of its SOP messages (bit 8).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PowerRole {
    /// 0: Sink.
    Sink,
    /// 1: Source.
    Source,
}

/// A power data object of a Source_Capabilities or Sink_Capabilities message, by its type in
/// bits 31–30, with its fields in physical units: the field times its unit, exactly.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(
    tag = "supply",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum PowerObject {
    /// `00`: a fixed supply.
    Fixed {
        /// The voltage, bits 19–10 in 50 mV units.
        millivolts: u16,
        /// The maximum current of a source, the operational current of a sink: bits 9–0 in
        /// 10 mA units.
        milliamps: u16,
        /// Bits 29–20 as they are, bit 29 in bit 9: flags whose meanings differ between source
        /// and sink objects and between revisions of the specification.
        flags: u16,
    },
    /// `01`: a battery.
    Battery {
        /// The maximum voltage, bits 29–20 in 50 mV units.
        max_millivolts: u16,
        /// The minimum voltage, bits 19–10 in 50 mV units.
        min_millivolts: u16,
        /// The maximum allowable power of a source, the operational power of a sink: bits 9–0
        /// in 250 mW units.
        milliwatts: u32,
    },
    /// `10`: a variable supply (not a battery).
    Variable {
        /// The maximum voltage, bits 29–20 in 50 mV units.
        max_millivolts: u16,
        /// The minimum voltage, bits 19–10 in 50 mV units.
        min_millivolts: u16,
        /// The maximum current of a source, the operational current of a sink: bits 9–0 in
        /// 10 mA units.
        milliamps: u16,
    },
    /// `11`: an augmented power data object (APDO).
    Augmented {
        /// The APDO type, bits 29–28: 0 is a programmable power supply (PPS); the others are
        /// reported by number only.
        apdo_type: u8,
        /// A PPS's maximum voltage, bits 24–17 in 100 mV units; `None` for other APDO types.
        max_millivolts: Option<u16>,
        /// A PPS's minimum voltage, bits 15–8 in 100 mV units; `None` for other APDO types.
        min_millivolts: Option<u16>,
        /// A PPS's maximum current, bits 6–0 in 50 mA units; `None` for other APDO types.
        milliamps: Option<u16>,
    },
}

impl PowerObject {
    /// The power data object `raw`, as it is laid out by its type.
    pub const fn from_raw(raw: u32) -> PowerObject {
        let high = ((raw >> 20) & 0x3ff) as u16;
        let middle = ((raw >> 10) & 0x3ff) as u16;
        let low = (raw & 0x3ff) as u16;
        match raw >> 30 {
            0 => PowerObject::Fixed {
                millivolts: middle * 50,
                milliamps: low * 10,
                flags: high,
            },
            1 => PowerObject::Battery {
                max_millivolts: high * 50,
                min_millivolts: middle * 50,
                milliwatts: low as u32 * 250,
            },
            2 => PowerObject::Variable {
                max_millivolts: high * 50,
                min_millivolts: middle * 50,
                milliamps: low * 10,
            },
            _ => {
                let apdo_type = ((raw >> 28) & 0x3) as u8;
                if apdo_type == 0 {
                    PowerObject::Augmented {
                        apdo_type,
                        max_millivolts: Some(((raw >> 17) & 0xff) as u16 * 100),
                        min_millivolts: Some(((raw >> 8) & 0xff) as u16 * 100),
                        milliamps: Some((raw & 0x7f) as u16 * 50),
                    }
                } else {
                    PowerObject::Augmented {
                        apdo_type,
                        max_millivolts: None,
                        min_millivolts: None,
                        milliamps: None,
                    }
                }
            }
        }
    }
}

/// A message header (§6.2.1.1), its bits 5 and 8 read as the ordered set before it says.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageHeader {
    /// Message Type, bits 4–0.
    pub message_type: u8,
    /// The control message's name: no data objects, not extended; `None` for other messages
    /// and reserved types.
    pub control: Option<ControlMessage>,
    /// The data message's name: data objects, not extended; `None` for other messages and
    /// reserved types.
    pub data: Option<DataMessage>,
    /// The extended message's name; `None` for other messages and reserved types.
    pub extended_message: Option<ExtendedMessage>,
    /// Port Data Role, bit 5, in SOP messages; `None` in the others, where the bit is
    /// reserved.
    pub data_role: Option<DataRole>,
    /// Port Power Role, bit 8, in SOP messages; `None` in the others.
    pub power_role: Option<PowerRole>,
    /// Cable Plug, bit 8, in the messages of every other set (SOP', SOP'' and their debug
    /// variants): whether a cable plug or VPD sent it; `None` in SOP messages.
    pub cable_plug: Option<bool>,
    /// Specification Revision, bits 7–6: 0 for revision 1.0, 1 for 2.0, 2 for 3.x.
    pub revision: u8,
    /// MessageID, bits 11–9.
    pub message_id: u8,
    /// Number of Data Objects, bits 14–12.
    pub objects: u8,
    /// Extended, bit 15.
    pub extended: bool,
}

impl MessageHeader {
    /// The header `raw` of a message that `set` starts.
    pub fn from_raw(raw: u16, set: OrderedSet) -> MessageHeader {
        let message_type = (raw & 0x1f) as u8;
        let objects = ((raw >> 12) & 0x7) as u8;
        let extended = raw & 0x8000 != 0;
        let (control, data, extended_message) = match (extended, objects) {
            (true, _) => (None, None, ExtendedMessage::from_type(message_type)),
            (false, 0) => (ControlMessage::from_type(message_type), None, None),
            (false, _) => (None, DataMessage::from_type(message_type), None),
        };
        // Bits 5 and 8 are the port's roles in SOP messages; in messages to and from cable
        // plugs bit 8 says whether a cable plug sent it and bit 5 is reserved.
        let sop = set == OrderedSet::Sop;
        let bit5 = raw & (1 << 5) != 0;
        let bit8 = raw & (1 << 8) != 0;
        MessageHeader {
            message_type,
            control,
            data,
            extended_message,
            data_role: sop.then_some(if bit5 { DataRole::Dfp } else { DataRole::Ufp }),
            power_role: sop.then_some(if bit8 {
                PowerRole::Source
            } else {
                PowerRole::Sink
            }),
            cable_plug: (!sop).then_some(bit8),
            revision: ((raw >> 6) & 0x3) as u8,
            message_id: ((raw >> 9) & 0x7) as u8,
            objects,
            extended,
        }
    }
}

/// An extended message header (§6.2.1.2), at the start of an extended message's data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtendedMessageHeader {
    /// Data Size, bits 8–0: the bytes of the whole message's data block.
    pub data_size: u16,
    /// Request Chunk, bit 10.
    pub request_chunk: bool,
    /// Chunk Number, bits 14–11.
    pub chunk_number: u8,
    /// Chunked, bit 15.
    pub chunked: bool,
}

impl ExtendedMessageHeader {
    /// The extended message header `raw`.
    pub const fn from_raw(raw: u16) -> ExtendedMessageHeader {
        ExtendedMessageHeader {
            data_size: raw & 0x1ff,
            request_chunk: raw & (1 << 10) != 0,
            chunk_number: ((raw >> 11) & 0xf) as u8,
            chunked: raw & 0x8000 != 0,
        }
    }
}

/// The request data object of a Request message (§6.4.2), as a request for a fixed supply, a
/// variable supply or a battery lays it out; a request for a programmable power supply lays
/// out bits 22–0 differently.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestObject {
    /// Object Position, bits 31–28: the power data object requested, from 1.
    pub position: u8,
    /// GiveBack flag, bit 27.
    pub give_back: bool,
    /// Capability Mismatch, bit 26.
    pub capability_mismatch: bool,
    /// USB Communications Capable, bit 25.
    pub usb_communications_capable: bool,
    /// No USB Suspend, bit 24.
    pub no_usb_suspend: bool,
    /// Unchunked Extended Messages Supported, bit 23.
    pub unchunked_extended: bool,
    /// Bits 19–10: the operating current in 10 mA units for a fixed or variable supply, the
    /// operating power in 250 mW units for a battery.
    pub operating: u16,
    /// Bits 9–0: the maximum operating current or power in the same units, or the minimum when
    /// `give_back` is set.
    pub maximum: u16,
}

impl RequestObject {
    /// The request data object `raw`.
    pub const fn from_raw(raw: u32) -> RequestObject {
        RequestObject {
            position: (raw >> 28) as u8,
            give_back: raw & (1 << 27) != 0,
            capability_mismatch: raw & (1 << 26) != 0,
            usb_communications_capable: raw & (1 << 25) != 0,
            no_usb_suspend: raw & (1 << 24) != 0,
            unchunked_extended: raw & (1 << 23) != 0,
            operating: ((raw >> 10) & 0x3ff) as u16,
            maximum: (raw & 0x3ff) as u16,
        }
    }
}

/// How a packet ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EndReason {
    /// The EOP K-code and the final transition: the packet is whole.
    Eop,
    /// The packet was Hard Reset signalling, which ends after its ordered set.
    HardReset,
    /// The packet was Cable Reset signalling, which ends after its ordered set.
    CableReset,
    /// An error ended it; the `error` event before says which.
    Error,
    /// The samples ended or were lost while the packet was under way.
    Aborted,
}

/// What went wrong in a packet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ErrorReason {
    /// A bit, or the bit rate the preamble shows, outside the tolerance: a transition too
    /// early, too late or missing.
    Timing,
    /// A five-bit code the 4b5b table does not have, or a K-code other than EOP among the
    /// data.
    Symbol,
    /// The first four symbols are no ordered set, not even with one of them corrupted.
    OrderedSet,
    /// An EOP before the fields the header announces.
    EarlyEop,
    /// No EOP after the CRC, where the header says the packet ends.
    MissingEop,
}

/// What travels on a USB PD event stream.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum UsbPdEvent {
    /// A packet begins: its preamble.
    Begin {
        /// The packet that begins.
        pkt: PacketId,
    },
    /// The ordered set of a message.
    Sop {
        /// The packet.
        pkt: PacketId,
        /// Which ordered set it is.
        set: OrderedSet,
        /// The position, 1 to 4, of a K-code that did not match the set, which was recognised
        /// from the other three; `None` when all four matched.
        corrupt: Option<u8>,
    },
    /// The message header.
    Header {
        /// The packet.
        pkt: PacketId,
        /// Its fields.
        #[serde(flatten)]
        header: MessageHeader,
    },
    /// The extended message header that starts an extended message's data.
    ExtendedHeader {
        /// The packet.
        pkt: PacketId,
        /// Its fields.
        #[serde(flatten)]
        header: ExtendedMessageHeader,
    },
    /// A data object of a data message, as it is sent.
    Object {
        /// The packet.
        pkt: PacketId,
        /// The object's position in the message, from 1, as Object Position counts them.
        index: u8,
        /// The 32 bits.
        raw: u32,
    },
    /// The object before, read as a power data object: in Source_Capabilities and
    /// Sink_Capabilities messages.
    Pdo {
        /// The packet.
        pkt: PacketId,
        /// The object's position in the message, from 1.
        index: u8,
        /// The object's type and fields.
        #[serde(flatten)]
        object: PowerObject,
    },
    /// The object before, read as the request data object of a Request message.
    Rdo {
        /// The packet.
        pkt: PacketId,
        /// Its fields.
        #[serde(flatten)]
        request: RequestObject,
    },
    /// Data bytes of an extended message, after its extended header, as the packet carries
    /// them.
    Data {
        /// The packet.
        pkt: PacketId,
        /// The position of the first byte after the extended header.
        offset: u16,
        /// At most [`DATA_BATCH`] bytes.
        bytes: Vec<u8>,
    },
    /// The CRC, and whether it matches.
    Crc {
        /// The packet.
        pkt: PacketId,
        /// The 32 bits as they are sent, the first in bit 0.
        value: u32,
        /// Whether the CRC matches the header and the data before it.
        ok: bool,
    },
    /// Hard Reset signalling.
    HardReset {
        /// The packet.
        pkt: PacketId,
        /// The position, 1 to 4, of a K-code that did not match; `None` when all four did.
        corrupt: Option<u8>,
    },
    /// Cable Reset signalling.
    CableReset {
        /// The packet.
        pkt: PacketId,
        /// The position, 1 to 4, of a K-code that did not match; `None` when all four did.
        corrupt: Option<u8>,
    },
    /// What went wrong; the packet ends with `error` right after.
    Error {
        /// The packet.
        pkt: PacketId,
        /// What it was.
        reason: ErrorReason,
    },
    /// The packet ended.
    End {
        /// The packet.
        pkt: PacketId,
        /// How it ended.
        reason: EndReason,
    },
}

impl UsbPdEvent {
    /// The packet the event belongs to.
    pub fn pkt(&self) -> PacketId {
        match self {
            UsbPdEvent::Begin { pkt }
            | UsbPdEvent::Sop { pkt, .. }
            | UsbPdEvent::Header { pkt, .. }
            | UsbPdEvent::ExtendedHeader { pkt, .. }
            | UsbPdEvent::Object { pkt, .. }
            | UsbPdEvent::Pdo { pkt, .. }
            | UsbPdEvent::Rdo { pkt, .. }
            | UsbPdEvent::Data { pkt, .. }
            | UsbPdEvent::Crc { pkt, .. }
            | UsbPdEvent::HardReset { pkt, .. }
            | UsbPdEvent::CableReset { pkt, .. }
            | UsbPdEvent::Error { pkt, .. }
            | UsbPdEvent::End { pkt, .. } => *pkt,
        }
    }
}

impl EventSchema for UsbPdEvent {
    const SCHEMA: SchemaId = USB_PD_EVENTS;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_types_round_trip() {
        for value in 0..32u8 {
            if let Some(message) = ControlMessage::from_type(value) {
                assert_eq!(message.message_type(), value);
            }
            if let Some(message) = DataMessage::from_type(value) {
                assert_eq!(message.message_type(), value);
            }
            if let Some(message) = ExtendedMessage::from_type(value) {
                assert_eq!(message.message_type(), value);
            }
        }
        let count = |known: fn(u8) -> bool| (0..32u8).filter(|value| known(*value)).count();
        assert_eq!(
            count(|value| ControlMessage::from_type(value).is_some()),
            24
        );
        assert_eq!(count(|value| DataMessage::from_type(value).is_some()), 13);
        assert_eq!(
            count(|value| ExtendedMessage::from_type(value).is_some()),
            19
        );
        assert_eq!(ControlMessage::from_type(0), None);
        assert_eq!(ControlMessage::from_type(25), None);
        assert_eq!(DataMessage::from_type(13), None);
        assert_eq!(DataMessage::from_type(15), Some(DataMessage::VendorDefined));
        assert_eq!(ExtendedMessage::from_type(19), None);
        assert_eq!(
            ExtendedMessage::from_type(30),
            Some(ExtendedMessage::VendorDefinedExtended)
        );
    }

    fn json<T: Serialize>(value: &T) -> String {
        serde_json::to_string(value).expect("serializes")
    }

    #[test]
    fn names_are_the_specification_names_in_kebab_case() {
        let names: Vec<String> = ControlMessage::ALL.iter().map(json).collect();
        assert_eq!(
            names.join(","),
            concat!(
                r#""good-crc","goto-min","accept","reject","ping","ps-rdy","get-source-cap","#,
                r#""get-sink-cap","dr-swap","pr-swap","vconn-swap","wait","soft-reset","#,
                r#""data-reset","data-reset-complete","not-supported","get-source-cap-extended","#,
                r#""get-status","fr-swap","get-pps-status","get-country-codes","#,
                r#""get-sink-cap-extended","get-source-info","get-revision""#
            )
        );
        let names: Vec<String> = DataMessage::ALL.iter().map(json).collect();
        assert_eq!(
            names.join(","),
            concat!(
                r#""source-capabilities","request","bist","sink-capabilities","battery-status","#,
                r#""alert","get-country-info","enter-usb","epr-request","epr-mode","source-info","#,
                r#""revision","vendor-defined""#
            )
        );
        let names: Vec<String> = ExtendedMessage::ALL.iter().map(json).collect();
        assert_eq!(
            names.join(","),
            concat!(
                r#""source-capabilities-extended","status","get-battery-cap","#,
                r#""get-battery-status","battery-capabilities","get-manufacturer-info","#,
                r#""manufacturer-info","security-request","security-response","#,
                r#""firmware-update-request","firmware-update-response","pps-status","#,
                r#""country-info","country-codes","sink-capabilities-extended","#,
                r#""extended-control","epr-source-capabilities","epr-sink-capabilities","#,
                r#""vendor-defined-extended""#
            )
        );
        let sets = [
            OrderedSet::Sop,
            OrderedSet::SopPrime,
            OrderedSet::SopDoublePrime,
            OrderedSet::SopPrimeDebug,
            OrderedSet::SopDoublePrimeDebug,
        ];
        let names: Vec<String> = sets.iter().map(json).collect();
        assert_eq!(
            names.join(","),
            r#""sop","sop-prime","sop-double-prime","sop-prime-debug","sop-double-prime-debug""#
        );
    }

    #[test]
    fn power_objects_read_their_fields_by_type() {
        // 5 V at 3 A, with Dual-Role Power and USB Communications Capable (bits 29 and 26).
        let fixed = (1 << 29) | (1 << 26) | (100 << 10) | 300;
        assert_eq!(
            PowerObject::from_raw(fixed),
            PowerObject::Fixed {
                millivolts: 5000,
                milliamps: 3000,
                flags: 0b10_0100_0000,
            }
        );
        let battery = (1 << 30) | (420 << 20) | (240 << 10) | 400;
        assert_eq!(
            PowerObject::from_raw(battery),
            PowerObject::Battery {
                max_millivolts: 21_000,
                min_millivolts: 12_000,
                milliwatts: 100_000,
            }
        );
        let variable = (2 << 30) | (1023 << 20) | (1 << 10) | 1023;
        assert_eq!(
            PowerObject::from_raw(variable),
            PowerObject::Variable {
                max_millivolts: 51_150,
                min_millivolts: 50,
                milliamps: 10_230,
            }
        );
        // A PPS from 3.3 V to 21 V at 3 A.
        let pps = (3 << 30) | (210 << 17) | (33 << 8) | 60;
        assert_eq!(
            PowerObject::from_raw(pps),
            PowerObject::Augmented {
                apdo_type: 0,
                max_millivolts: Some(21_000),
                min_millivolts: Some(3_300),
                milliamps: Some(3_000),
            }
        );
        assert_eq!(
            PowerObject::from_raw(0xd000_0000 | 0x1234),
            PowerObject::Augmented {
                apdo_type: 1,
                max_millivolts: None,
                min_millivolts: None,
                milliamps: None,
            }
        );
    }

    #[test]
    fn serializes_as_the_contract_spells_it() {
        let pkt = PacketId(3);
        assert_eq!(
            json(&UsbPdEvent::Begin { pkt }),
            r#"{"kind":"begin","pkt":3}"#
        );
        assert_eq!(
            json(&UsbPdEvent::Sop {
                pkt,
                set: OrderedSet::SopPrime,
                corrupt: None
            }),
            r#"{"kind":"sop","pkt":3,"set":"sop-prime","corrupt":null}"#
        );
        assert_eq!(
            json(&UsbPdEvent::Header {
                pkt,
                header: MessageHeader {
                    message_type: 1,
                    control: None,
                    data: Some(DataMessage::SourceCapabilities),
                    extended_message: None,
                    data_role: Some(DataRole::Dfp),
                    power_role: Some(PowerRole::Source),
                    cable_plug: None,
                    revision: 2,
                    message_id: 5,
                    objects: 2,
                    extended: false,
                },
            }),
            concat!(
                r#"{"kind":"header","pkt":3,"messageType":1,"control":null,"#,
                r#""data":"source-capabilities","extendedMessage":null,"dataRole":"dfp","#,
                r#""powerRole":"source","cablePlug":null,"revision":2,"messageId":5,"#,
                r#""objects":2,"extended":false}"#
            )
        );
        assert_eq!(
            json(&UsbPdEvent::ExtendedHeader {
                pkt,
                header: ExtendedMessageHeader {
                    data_size: 30,
                    request_chunk: false,
                    chunk_number: 1,
                    chunked: true
                },
            }),
            concat!(
                r#"{"kind":"extended-header","pkt":3,"dataSize":30,"requestChunk":false,"#,
                r#""chunkNumber":1,"chunked":true}"#
            )
        );
        assert_eq!(
            json(&UsbPdEvent::Object {
                pkt,
                index: 1,
                raw: 0x0001_912c
            }),
            r#"{"kind":"object","pkt":3,"index":1,"raw":102700}"#
        );
        assert_eq!(
            json(&UsbPdEvent::Pdo {
                pkt,
                index: 1,
                object: PowerObject::Fixed {
                    millivolts: 5000,
                    milliamps: 3000,
                    flags: 0
                }
            }),
            r#"{"kind":"pdo","pkt":3,"index":1,"supply":"fixed","millivolts":5000,"milliamps":3000,"flags":0}"#
        );
        assert_eq!(
            json(&UsbPdEvent::Pdo {
                pkt,
                index: 2,
                object: PowerObject::Augmented {
                    apdo_type: 0,
                    max_millivolts: Some(21_000),
                    min_millivolts: Some(3_300),
                    milliamps: Some(3_000)
                }
            }),
            concat!(
                r#"{"kind":"pdo","pkt":3,"index":2,"supply":"augmented","apdoType":0,"#,
                r#""maxMillivolts":21000,"minMillivolts":3300,"milliamps":3000}"#
            )
        );
        assert_eq!(
            json(&UsbPdEvent::Rdo {
                pkt,
                request: RequestObject {
                    position: 2,
                    give_back: false,
                    capability_mismatch: true,
                    usb_communications_capable: true,
                    no_usb_suspend: false,
                    unchunked_extended: false,
                    operating: 200,
                    maximum: 300
                },
            }),
            concat!(
                r#"{"kind":"rdo","pkt":3,"position":2,"giveBack":false,"#,
                r#""capabilityMismatch":true,"usbCommunicationsCapable":true,"#,
                r#""noUsbSuspend":false,"unchunkedExtended":false,"operating":200,"maximum":300}"#
            )
        );
        assert_eq!(
            json(&UsbPdEvent::Data {
                pkt,
                offset: 0,
                bytes: vec![1, 2]
            }),
            r#"{"kind":"data","pkt":3,"offset":0,"bytes":[1,2]}"#
        );
        assert_eq!(
            json(&UsbPdEvent::Crc {
                pkt,
                value: 0xcbf4_3926,
                ok: true
            }),
            r#"{"kind":"crc","pkt":3,"value":3421780262,"ok":true}"#
        );
        assert_eq!(
            json(&UsbPdEvent::HardReset {
                pkt,
                corrupt: Some(2)
            }),
            r#"{"kind":"hard-reset","pkt":3,"corrupt":2}"#
        );
        assert_eq!(
            json(&UsbPdEvent::CableReset { pkt, corrupt: None }),
            r#"{"kind":"cable-reset","pkt":3,"corrupt":null}"#
        );
        assert_eq!(
            json(&UsbPdEvent::Error {
                pkt,
                reason: ErrorReason::MissingEop
            }),
            r#"{"kind":"error","pkt":3,"reason":"missing-eop"}"#
        );
        assert_eq!(
            json(&UsbPdEvent::End {
                pkt,
                reason: EndReason::HardReset
            }),
            r#"{"kind":"end","pkt":3,"reason":"hard-reset"}"#
        );
    }

    #[test]
    fn events_read_back_as_they_were_written() {
        let events = [
            UsbPdEvent::Pdo {
                pkt: PacketId(1),
                index: 3,
                object: PowerObject::Battery {
                    max_millivolts: 20_000,
                    min_millivolts: 5_000,
                    milliwatts: 60_000,
                },
            },
            UsbPdEvent::Header {
                pkt: PacketId(2),
                header: MessageHeader::from_raw(0x0341, OrderedSet::SopPrime),
            },
            UsbPdEvent::ExtendedHeader {
                pkt: PacketId(2),
                header: ExtendedMessageHeader::from_raw(0x8a1a),
            },
            UsbPdEvent::Rdo {
                pkt: PacketId(2),
                request: RequestObject::from_raw(0x1204_b12c),
            },
            UsbPdEvent::Error {
                pkt: PacketId(3),
                reason: ErrorReason::OrderedSet,
            },
        ];
        for event in events {
            let back: UsbPdEvent = serde_json::from_str(&json(&event)).expect("parses");
            assert_eq!(back, event);
        }
    }

    #[test]
    fn a_sop_header_carries_the_port_roles() {
        // Source_Capabilities, two objects, message ID 3, revision 3.x, from a DFP source.
        let raw = (2 << 12) | (3 << 9) | (1 << 8) | (2 << 6) | (1 << 5) | 1;
        assert_eq!(
            MessageHeader::from_raw(raw, OrderedSet::Sop),
            MessageHeader {
                message_type: 1,
                control: None,
                data: Some(DataMessage::SourceCapabilities),
                extended_message: None,
                data_role: Some(DataRole::Dfp),
                power_role: Some(PowerRole::Source),
                cable_plug: None,
                revision: 2,
                message_id: 3,
                objects: 2,
                extended: false,
            }
        );
    }

    #[test]
    fn a_cable_header_says_whether_a_plug_sent_it() {
        // GoodCRC from a cable plug: bit 8 set, bit 5 reserved and set anyway.
        let raw = (1 << 8) | (1 << 6) | (1 << 5) | 1;
        for set in [
            OrderedSet::SopPrime,
            OrderedSet::SopDoublePrime,
            OrderedSet::SopPrimeDebug,
            OrderedSet::SopDoublePrimeDebug,
        ] {
            let header = MessageHeader::from_raw(raw, set);
            assert_eq!(header.control, Some(ControlMessage::GoodCrc));
            assert_eq!(
                (header.data_role, header.power_role, header.cable_plug),
                (None, None, Some(true))
            );
        }
    }

    #[test]
    fn extended_and_reserved_types_are_named_by_their_table() {
        let header = MessageHeader::from_raw(0x8000 | (7 << 12) | 2, OrderedSet::Sop);
        assert_eq!(
            (header.control, header.data, header.extended_message),
            (None, None, Some(ExtendedMessage::Status))
        );
        assert!(header.extended);
        let header = MessageHeader::from_raw(25, OrderedSet::Sop);
        assert_eq!((header.control, header.message_type), (None, 25));
        let header = MessageHeader::from_raw((1 << 12) | 13, OrderedSet::Sop);
        assert_eq!((header.data, header.message_type), (None, 13));
    }

    #[test]
    fn extended_headers_and_requests_read_their_fields() {
        assert_eq!(
            ExtendedMessageHeader::from_raw(0x8000 | (2 << 11) | (1 << 10) | 260),
            ExtendedMessageHeader {
                data_size: 260,
                request_chunk: true,
                chunk_number: 2,
                chunked: true,
            }
        );
        // Object 2, capability mismatch, USB communications capable, 1.5 A of 3 A.
        let raw = (2 << 28) | (1 << 26) | (1 << 25) | (150 << 10) | 300;
        assert_eq!(
            RequestObject::from_raw(raw),
            RequestObject {
                position: 2,
                give_back: false,
                capability_mismatch: true,
                usb_communications_capable: true,
                no_usb_suspend: false,
                unchunked_extended: false,
                operating: 150,
                maximum: 300,
            }
        );
        // Positions past 7, as EPR uses them, and every flag.
        let raw = (11 << 28) | (0b11111 << 23);
        let request = RequestObject::from_raw(raw);
        assert_eq!(request.position, 11);
        assert!(
            request.give_back
                && request.capability_mismatch
                && request.usb_communications_capable
                && request.no_usb_suspend
                && request.unchunked_extended
        );
    }
}
