// SPDX-License-Identifier: Apache-2.0

//! The node types by name: how to build each from a JSON configuration, its defaults, its
//! manifest and how to read its output without the event type.

use serde::Serialize;
use serde::de::DeserializeOwned;
use uiharu_core::{DynExposedRef, EventSchema, Node, Pipeline, Props, SchemaId};
use uiharu_interp::{FromDyn, ToDyn};
use uiharu_protocols::am230x::{Am230x, Am230xConfig, Am230xEvent};
use uiharu_protocols::apa102::{Apa102, Apa102Event};
use uiharu_protocols::avr_isp::{AvrIsp, IspEvent};
use uiharu_protocols::can::{Can, CanConfig, CanEvent};
use uiharu_protocols::dmx512::{Dmx512, Dmx512Event};
use uiharu_protocols::ds18b20::{Ds18b20, Ds18b20Event};
use uiharu_protocols::eeprom24xx::{Eeprom24xx, Eeprom24xxConfig, EepromEvent};
use uiharu_protocols::eeprom93xx::{Eeprom93xx, Eeprom93xxConfig, Eeprom93xxEvent};
use uiharu_protocols::i2c::{I2c, I2cConfig, I2cEvent};
use uiharu_protocols::i2s::{I2s, I2sConfig, I2sEvent};
use uiharu_protocols::ir_nec::{Nec, NecConfig, NecEvent};
use uiharu_protocols::ir_rc5::{Rc5, Rc5Config, Rc5Event};
use uiharu_protocols::ir_rc6::{Rc6, Rc6Config, Rc6Event};
use uiharu_protocols::ir_sirc::{Sirc, SircConfig, SircEvent};
use uiharu_protocols::jtag::{Jtag, JtagConfig, JtagEvent};
use uiharu_protocols::jtag_dp::{JtagDp, JtagDpConfig, JtagDpEvent};
use uiharu_protocols::lin::{Lin, LinConfig, LinEvent};
use uiharu_protocols::mdio::{Mdio, MdioConfig, MdioEvent};
use uiharu_protocols::microwire::{Microwire, MicrowireConfig, MicrowireEvent};
use uiharu_protocols::midi::{Midi, MidiEvent};
use uiharu_protocols::modbus_rtu::{ModbusEvent, ModbusRtu};
use uiharu_protocols::nrf24l01::{Nrf24Event, Nrf24l01};
use uiharu_protocols::onewire::{LinkConfig, LinkEvent, OneWireEvent, OneWireLink, OneWireNetwork};
use uiharu_protocols::parallel::{Parallel, ParallelConfig, ParallelEvent};
use uiharu_protocols::ps2::{Ps2, Ps2Config, Ps2Event};
use uiharu_protocols::pwm::{Pwm, PwmConfig, PwmEvent};
use uiharu_protocols::qspi::{Qspi, QspiConfig, QspiEvent};
use uiharu_protocols::sdcard_sd::{SdcardSd, SdcardSdConfig, SdcardSdEvent};
use uiharu_protocols::sdcard_spi::{SdCardSpi, SdCardSpiConfig, SdEvent};
use uiharu_protocols::spi::{Spi, SpiConfig, SpiEvent};
use uiharu_protocols::spi_nor::{NorEvent, SpiNor, SpiNorConfig};
use uiharu_protocols::swd::{Swd, SwdConfig, SwdEvent};
use uiharu_protocols::swim::{Swim, SwimConfig, SwimEvent};
use uiharu_protocols::timing::{Timing, TimingConfig, TimingEvent};
use uiharu_protocols::uart::{Uart, UartConfig, UartEvent};
use uiharu_protocols::usb::{TransactionEvent, UsbConfig, UsbEvent, UsbPackets, UsbTransactions};
use uiharu_protocols::usb_pd::{UsbPd, UsbPdConfig, UsbPdEvent};
use uiharu_protocols::wiegand::{Wiegand, WiegandConfig, WiegandEvent};
use uiharu_protocols::ws281x::{Ws281x, Ws281xConfig, Ws281xEvent};

use crate::dynamic::{DynReader, TypedReader};

/// A node type.
pub struct Kind {
    /// The name a description uses.
    pub name: &'static str,
    /// The manifest, as JSON; see [`crate::Manifest`].
    pub manifest: &'static str,
    /// Builds the node from its configuration, which it validates.
    pub build: fn(&serde_json::Value) -> Result<Box<dyn Node>, String>,
    /// The default configuration, as JSON (`{}` for a node without one).
    pub defaults: fn() -> serde_json::Value,
    /// Opens a reader on the node's output, or `None` when the output carries another type.
    pub open: fn(&mut Pipeline, DynExposedRef, String) -> Option<Box<dyn DynReader>>,
    /// The `kind` tags of the events the node's output carries, in the order its contract
    /// declares them: the kinds its manifest describes.
    pub kinds: fn() -> Vec<String>,
    /// How the node's contract is bridged to and from the streams of interpreted decoders.
    pub bridge: fn() -> Bridge,
}

/// The bridges of a contract's Rust type: between its typed streams and the streams of
/// decoders interpreted from their IR, which carry the contract as JSON.
#[derive(Clone, Copy)]
pub struct Bridge {
    /// The schema the contract implements.
    pub schema: SchemaId,
    /// A node reading the typed stream and writing it as JSON events, declaring `props`.
    pub to_dyn: fn(Props) -> Box<dyn Node>,
    /// A node reading JSON events and writing the typed stream, declaring `props`.
    pub from_dyn: fn(Props) -> Box<dyn Node>,
}

impl std::fmt::Debug for Bridge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Bridge")
            .field("schema", &self.schema)
            .finish_non_exhaustive()
    }
}

/// The bridges of the contract `T` implements.
fn bridge_of<T>() -> Bridge
where
    T: Serialize + DeserializeOwned + EventSchema + Clone + 'static,
{
    Bridge {
        schema: T::SCHEMA,
        to_dyn: |props| Box::new(ToDyn::<T>::new(props)),
        from_dyn: |props| Box::new(FromDyn::<T>::new(props)),
    }
}

impl std::fmt::Debug for Kind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Kind")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

/// The node type named `name`, if there is one.
pub fn kind(name: &str) -> Option<&'static Kind> {
    KINDS.iter().find(|kind| kind.name == name)
}

/// Opens a reader on an output carrying `T` events.
fn open_as<T: Serialize + EventSchema + Clone + 'static>(
    pipeline: &mut Pipeline,
    exposed: DynExposedRef,
    name: String,
) -> Option<Box<dyn DynReader>> {
    Some(Box::new(TypedReader::new(
        name,
        pipeline.exposed_dyn::<T>(exposed)?,
    )))
}

/// The `kind` tags of `T`'s events, in the order the contract declares them.
fn kinds_of<T: DeserializeOwned>() -> Vec<String> {
    // Serde knows them: asked for an event of a kind no contract has, it names those there are.
    let unknown = serde_json::json!({ "kind": "\u{0}" });
    match serde_json::from_value::<T>(unknown) {
        Ok(_) => Vec::new(),
        Err(error) => expected_variants(&error.to_string()),
    }
}

/// The names a serde error about an unknown variant gives as expected, in order; none for any
/// other error. Serde words them "expected `a`", "expected `a` or `b`" or "expected one of `a`,
/// `b`, `c`".
pub(crate) fn expected_variants(message: &str) -> Vec<String> {
    let Some((_, expected)) = message
        .strip_prefix("unknown variant ")
        .and_then(|rest| rest.split_once(", expected "))
    else {
        return Vec::new();
    };
    expected
        .split('`')
        .skip(1)
        .step_by(2)
        .map(str::to_owned)
        .collect()
}

/// The default configuration of a node configured by `C`, as JSON.
fn defaults_of<C: Default + Serialize>() -> serde_json::Value {
    // A configuration is plain data; serializing it cannot fail.
    serde_json::to_value(C::default()).unwrap_or(serde_json::Value::Null)
}

/// The configuration of a node that takes none.
fn no_defaults() -> serde_json::Value {
    serde_json::Value::Object(serde_json::Map::new())
}

fn config<T: DeserializeOwned>(value: &serde_json::Value) -> Result<T, String> {
    let value = match value {
        serde_json::Value::Null => serde_json::Value::Object(serde_json::Map::new()),
        other => other.clone(),
    };
    serde_json::from_value(value).map_err(|error| error.to_string())
}

/// Builds a node from its configuration with `make`, which validates it.
fn build_node<C, N, E>(
    value: &serde_json::Value,
    make: fn(C) -> Result<N, E>,
) -> Result<Box<dyn Node>, String>
where
    C: DeserializeOwned,
    N: Node,
    E: std::fmt::Display,
{
    let config: C = config(value)?;
    make(config)
        .map(|node| Box::new(node) as Box<dyn Node>)
        .map_err(|error| error.to_string())
}

fn build_spi(value: &serde_json::Value) -> Result<Box<dyn Node>, String> {
    build_node(value, Spi::new)
}

/// Checks that a node without configuration was given none.
fn no_config(value: &serde_json::Value) -> Result<(), String> {
    if !matches!(value, serde_json::Value::Null) && value != &serde_json::json!({}) {
        return Err("takes no configuration".to_owned());
    }
    Ok(())
}

fn build_spi_nor(value: &serde_json::Value) -> Result<Box<dyn Node>, String> {
    build_node(value, SpiNor::new)
}

fn build_sdcard_spi(value: &serde_json::Value) -> Result<Box<dyn Node>, String> {
    build_node(value, SdCardSpi::new)
}

fn build_i2s(value: &serde_json::Value) -> Result<Box<dyn Node>, String> {
    build_node(value, I2s::new)
}

fn build_pwm(value: &serde_json::Value) -> Result<Box<dyn Node>, String> {
    build_node(value, Pwm::new)
}

fn build_timing(value: &serde_json::Value) -> Result<Box<dyn Node>, String> {
    build_node(value, Timing::new)
}

fn build_parallel(value: &serde_json::Value) -> Result<Box<dyn Node>, String> {
    build_node(value, Parallel::new)
}

fn build_onewire_link(value: &serde_json::Value) -> Result<Box<dyn Node>, String> {
    build_node(value, OneWireLink::new)
}

fn build_onewire(value: &serde_json::Value) -> Result<Box<dyn Node>, String> {
    no_config(value)?;
    Ok(Box::new(OneWireNetwork::new()))
}

fn build_ds18b20(value: &serde_json::Value) -> Result<Box<dyn Node>, String> {
    no_config(value)?;
    Ok(Box::new(Ds18b20::new()))
}

fn build_usb(value: &serde_json::Value) -> Result<Box<dyn Node>, String> {
    build_node(value, UsbPackets::new)
}

fn build_usb_transactions(value: &serde_json::Value) -> Result<Box<dyn Node>, String> {
    no_config(value)?;
    Ok(Box::new(UsbTransactions::new()))
}

fn build_can(value: &serde_json::Value) -> Result<Box<dyn Node>, String> {
    build_node(value, Can::new)
}

fn build_i2c(value: &serde_json::Value) -> Result<Box<dyn Node>, String> {
    build_node(value, I2c::new)
}

fn build_eeprom24xx(value: &serde_json::Value) -> Result<Box<dyn Node>, String> {
    build_node(value, Eeprom24xx::new)
}

fn build_jtag(value: &serde_json::Value) -> Result<Box<dyn Node>, String> {
    build_node(value, Jtag::new)
}

fn build_swd(value: &serde_json::Value) -> Result<Box<dyn Node>, String> {
    build_node(value, Swd::new)
}

fn build_uart(value: &serde_json::Value) -> Result<Box<dyn Node>, String> {
    build_node(value, Uart::new)
}

fn build_modbus_rtu(value: &serde_json::Value) -> Result<Box<dyn Node>, String> {
    no_config(value)?;
    Ok(Box::new(ModbusRtu::new()))
}

fn build_lin(value: &serde_json::Value) -> Result<Box<dyn Node>, String> {
    build_node(value, Lin::new)
}

fn build_midi(value: &serde_json::Value) -> Result<Box<dyn Node>, String> {
    no_config(value)?;
    Ok(Box::new(Midi::new()))
}

fn build_ws281x(value: &serde_json::Value) -> Result<Box<dyn Node>, String> {
    build_node(value, Ws281x::new)
}

fn build_dmx512(value: &serde_json::Value) -> Result<Box<dyn Node>, String> {
    no_config(value)?;
    Ok(Box::new(Dmx512::new()))
}

fn build_apa102(value: &serde_json::Value) -> Result<Box<dyn Node>, String> {
    no_config(value)?;
    Ok(Box::new(Apa102::new()))
}

fn build_mdio(value: &serde_json::Value) -> Result<Box<dyn Node>, String> {
    build_node(value, Mdio::new)
}

fn build_avr_isp(value: &serde_json::Value) -> Result<Box<dyn Node>, String> {
    no_config(value)?;
    Ok(Box::new(AvrIsp::new()))
}

fn build_nrf24l01(value: &serde_json::Value) -> Result<Box<dyn Node>, String> {
    no_config(value)?;
    Ok(Box::new(Nrf24l01::new()))
}

fn build_usb_pd(value: &serde_json::Value) -> Result<Box<dyn Node>, String> {
    build_node(value, UsbPd::new)
}

fn build_ir_nec(value: &serde_json::Value) -> Result<Box<dyn Node>, String> {
    build_node(value, Nec::new)
}

fn build_ir_rc5(value: &serde_json::Value) -> Result<Box<dyn Node>, String> {
    build_node(value, Rc5::new)
}

fn build_ir_rc6(value: &serde_json::Value) -> Result<Box<dyn Node>, String> {
    build_node(value, Rc6::new)
}

fn build_ir_sirc(value: &serde_json::Value) -> Result<Box<dyn Node>, String> {
    build_node(value, Sirc::new)
}

fn build_wiegand(value: &serde_json::Value) -> Result<Box<dyn Node>, String> {
    build_node(value, Wiegand::new)
}

fn build_ps2(value: &serde_json::Value) -> Result<Box<dyn Node>, String> {
    build_node(value, Ps2::new)
}

fn build_am230x(value: &serde_json::Value) -> Result<Box<dyn Node>, String> {
    build_node(value, Am230x::new)
}

fn build_microwire(value: &serde_json::Value) -> Result<Box<dyn Node>, String> {
    build_node(value, Microwire::new)
}

fn build_eeprom93xx(value: &serde_json::Value) -> Result<Box<dyn Node>, String> {
    build_node(value, Eeprom93xx::new)
}

fn build_qspi(value: &serde_json::Value) -> Result<Box<dyn Node>, String> {
    build_node(value, Qspi::new)
}
fn build_sdcard_sd(value: &serde_json::Value) -> Result<Box<dyn Node>, String> {
    build_node(value, SdcardSd::new)
}
fn build_swim(value: &serde_json::Value) -> Result<Box<dyn Node>, String> {
    build_node(value, Swim::new)
}
fn build_jtag_dp(value: &serde_json::Value) -> Result<Box<dyn Node>, String> {
    let config: JtagDpConfig = config(value)?;
    Ok(Box::new(JtagDp::new(config)))
}

// Generated nodes use the same typed readers and dynamic bridges as handwritten nodes.
// The metadata comes from the TS declaration through `export_decoder_manifests`.
macro_rules! dsl_kind {
    ($name:literal, $module:ident, $node:ident, $config:ident, $event:ident) => {
        Kind {
            name: $name,
            manifest: include_str!(concat!("../manifests/", $name, ".json")),
            build: |value| build_node(value, uiharu_protocols::$module::$node::new),
            defaults: defaults_of::<uiharu_protocols::$module::$config>,
            open: open_as::<uiharu_protocols::$module::$event>,
            kinds: kinds_of::<uiharu_protocols::$module::$event>,
            bridge: bridge_of::<uiharu_protocols::$module::$event>,
        }
    };
}

/// Every node type, by the name a description uses.
pub const KINDS: &[Kind] = &[
    Kind {
        name: "cjtag",
        manifest: include_str!("../manifests/cjtag.json"),
        build: |value| build_node(value, uiharu_protocols::cjtag::Cjtag::new),
        defaults: defaults_of::<uiharu_protocols::cjtag::CjtagConfig>,
        open: open_as::<JtagEvent>,
        kinds: kinds_of::<JtagEvent>,
        bridge: bridge_of::<JtagEvent>,
    },
    dsl_kind!("swd-dp", swd_dp, SwdDp, SwdDpConfig, Event),
    dsl_kind!(
        "avr-isp-device",
        avr_isp_device,
        AvrIspDevice,
        AvrIspDeviceConfig,
        Event
    ),
    dsl_kind!("am230x-values", am230x_values, Am230xValues, Config, Event),
    dsl_kind!("cfp", cfp, Cfp, Config, Event),
    dsl_kind!("ds2408", ds2408, Ds2408, Config, Event),
    dsl_kind!("ds243x", ds243x, Ds243x, Config, Event),
    dsl_kind!("ds28ea00", ds28ea00, Ds28ea00, Config, Event),
    dsl_kind!(
        "midi-semantics",
        midi_semantics,
        MidiSemantics,
        Config,
        Event
    ),
    dsl_kind!(
        "dmx512-analysis",
        dmx512_analysis,
        Dmx512Analysis,
        Config,
        Event
    ),
    dsl_kind!("lin-profile", lin_profile, LinProfile, Config, Event),
    dsl_kind!("swim-stm8", swim_stm8, SwimStm8, SwimStm8Config, SwimEvent),
    dsl_kind!(
        "arm-etmv3",
        arm_etmv3,
        ArmEtmv3,
        ArmEtmv3Config,
        ArmEtmv3Event
    ),
    dsl_kind!("smbus", smbus, Smbus, Config, Event),
    dsl_kind!("ps2-device", ps2_device, Ps2Device, Config, Event),
    dsl_kind!("xn297", xn297, Xn297, Config, Event),
    dsl_kind!(
        "jtag-stm32",
        jtag_stm32,
        JtagStm32,
        JtagStm32Config,
        JtagStm32Event
    ),
    dsl_kind!(
        "jtag-ejtag",
        jtag_ejtag,
        JtagEjtag,
        JtagEjtagConfig,
        JtagEjtagEvent
    ),
    dsl_kind!("c2", c2, C2, Config, Event),
    dsl_kind!("avr-pdi", avr_pdi, AvrPdi, Config, Event),
    dsl_kind!("fsi", fsi, Fsi, Config, Event),
    dsl_kind!(
        "jtag-chain",
        jtag_chain,
        JtagChain,
        JtagChainConfig,
        JtagEvent
    ),
    dsl_kind!("ad79x0", ad79x0, Ad79x0, Config, DeviceEvent),
    dsl_kind!(
        "arm-trace-transport",
        arm_trace_transport,
        ArmTraceTransport,
        ArmTraceTransportConfig,
        TraceByteEvent
    ),
    dsl_kind!("arm-itm", arm_itm, ArmItm, ArmItmConfig, ArmItmEvent),
    dsl_kind!(
        "eeprom24xx-memory",
        eeprom24xx_memory,
        Eeprom24xxMemory,
        Config,
        Event
    ),
    dsl_kind!("aud", aud, Aud, Config, Event),
    dsl_kind!(
        "qspi-flash",
        qspi_flash,
        QspiFlash,
        QspiFlashConfig,
        FlashEvent
    ),
    dsl_kind!("ad5626", ad5626, Ad5626, Config, DeviceEvent),
    dsl_kind!("adf435x", adf435x, Adf435x, Config, DeviceEvent),
    dsl_kind!("ltc242x", ltc242x, Ltc242x, Config, DeviceEvent),
    dsl_kind!("tlc5620", tlc5620, Tlc5620, Config, DeviceEvent),
    dsl_kind!("rfm12", rfm12, Rfm12, Config, Event),
    dsl_kind!("max7219", max7219, Max7219, Config, Event),
    dsl_kind!("max72xx", max72xx, Max72xx, Config, Event),
    dsl_kind!("cc1101", cc1101, Cc1101, Config, Event),
    dsl_kind!("enc28j60", enc28j60, Enc28j60, Config, Event),
    dsl_kind!("ade77xx", ade77xx, Ade77xx, Config, Event),
    dsl_kind!("nrf905", nrf905, Nrf905, Config, Event),
    dsl_kind!("st25r39xx-spi", st25r39xx_spi, St25r39xxSpi, Config, Event),
    dsl_kind!("adns5020", adns5020, Adns5020, Config, Event),
    dsl_kind!("a7105", a7105, A7105, Config, Event),
    dsl_kind!("mrf24j40", mrf24j40, Mrf24j40, Config, Event),
    dsl_kind!("adxl345-spi", adxl345_spi, Adxl345Spi, Config, Event),
    dsl_kind!("st7735", st7735, St7735, Config, Event),
    dsl_kind!("st7789", st7789, St7789, Config, Event),
    dsl_kind!("sd-registers", sd_registers, SdRegisters, Config, Event),
    dsl_kind!("spi-tpm", spi_tpm, SpiTpm, Config, Event),
    dsl_kind!("x2444m", x2444m, X2444m, Config, Event),
    dsl_kind!("sd-native", sd_native, SdNative, Config, Event),
    dsl_kind!("rgb-led-spi", rgb_led_spi, RgbLedSpi, Config, Event),
    dsl_kind!("sda2506", sda2506, Sda2506, Config, Event),
    dsl_kind!("adxl345", adxl345, Adxl345, DeviceConfig, DeviceEvent),
    dsl_kind!("ds1307", ds1307, Ds1307, DeviceConfig, DeviceEvent),
    dsl_kind!("lm75", lm75, Lm75, DeviceConfig, DeviceEvent),
    dsl_kind!("mcp230xx", mcp230xx, Mcp230xx, DeviceConfig, DeviceEvent),
    dsl_kind!("mlx90614", mlx90614, Mlx90614, DeviceConfig, DeviceEvent),
    dsl_kind!("pca9571", pca9571, Pca9571, DeviceConfig, DeviceEvent),
    dsl_kind!("rtc8564", rtc8564, Rtc8564, DeviceConfig, DeviceEvent),
    dsl_kind!("tca6408a", tca6408a, Tca6408a, DeviceConfig, DeviceEvent),
    dsl_kind!("edid", edid, Edid, DeviceConfig, DeviceEvent),
    dsl_kind!("ltc26x7", ltc26x7, Ltc26x7, DeviceConfig, DeviceEvent),
    dsl_kind!("mxc6225xu", mxc6225xu, Mxc6225xu, DeviceConfig, DeviceEvent),
    dsl_kind!("nunchuk", nunchuk, Nunchuk, DeviceConfig, DeviceEvent),
    dsl_kind!("atsha204a", atsha204a, Atsha204a, DeviceConfig, DeviceEvent),
    dsl_kind!("xfp", xfp, Xfp, DeviceConfig, DeviceEvent),
    dsl_kind!(
        "amulet-ascii",
        amulet_ascii,
        AmuletAscii,
        DeviceConfig,
        DeviceEvent
    ),
    dsl_kind!("pan1321", pan1321, Pan1321, DeviceConfig, DeviceEvent),
    dsl_kind!(
        "arm-memap",
        arm_memap,
        ArmMemap,
        ArmMemapConfig,
        ArmMemapEvent
    ),
    dsl_kind!(
        "modbus-pdu",
        modbus_pdu,
        ModbusPdu,
        ModbusPduConfig,
        ModbusPduEvent
    ),
    Kind {
        name: "spi",
        manifest: include_str!("../manifests/spi.json"),
        build: build_spi,
        defaults: defaults_of::<SpiConfig>,
        open: open_as::<SpiEvent>,
        kinds: kinds_of::<SpiEvent>,
        bridge: bridge_of::<SpiEvent>,
    },
    Kind {
        name: "spi-nor",
        manifest: include_str!("../manifests/spi-nor.json"),
        build: build_spi_nor,
        defaults: defaults_of::<SpiNorConfig>,
        open: open_as::<NorEvent>,
        kinds: kinds_of::<NorEvent>,
        bridge: bridge_of::<NorEvent>,
    },
    Kind {
        name: "i2s",
        manifest: include_str!("../manifests/i2s.json"),
        build: build_i2s,
        defaults: defaults_of::<I2sConfig>,
        open: open_as::<I2sEvent>,
        kinds: kinds_of::<I2sEvent>,
        bridge: bridge_of::<I2sEvent>,
    },
    Kind {
        name: "pwm",
        manifest: include_str!("../manifests/pwm.json"),
        build: build_pwm,
        defaults: defaults_of::<PwmConfig>,
        open: open_as::<PwmEvent>,
        kinds: kinds_of::<PwmEvent>,
        bridge: bridge_of::<PwmEvent>,
    },
    Kind {
        name: "timing",
        manifest: include_str!("../manifests/timing.json"),
        build: build_timing,
        defaults: defaults_of::<TimingConfig>,
        open: open_as::<TimingEvent>,
        kinds: kinds_of::<TimingEvent>,
        bridge: bridge_of::<TimingEvent>,
    },
    Kind {
        name: "parallel",
        manifest: include_str!("../manifests/parallel.json"),
        build: build_parallel,
        defaults: defaults_of::<ParallelConfig>,
        open: open_as::<ParallelEvent>,
        kinds: kinds_of::<ParallelEvent>,
        bridge: bridge_of::<ParallelEvent>,
    },
    Kind {
        name: "uart",
        manifest: include_str!("../manifests/uart.json"),
        build: build_uart,
        defaults: defaults_of::<UartConfig>,
        open: open_as::<UartEvent>,
        kinds: kinds_of::<UartEvent>,
        bridge: bridge_of::<UartEvent>,
    },
    Kind {
        name: "modbus-rtu",
        manifest: include_str!("../manifests/modbus-rtu.json"),
        build: build_modbus_rtu,
        defaults: no_defaults,
        open: open_as::<ModbusEvent>,
        kinds: kinds_of::<ModbusEvent>,
        bridge: bridge_of::<ModbusEvent>,
    },
    Kind {
        name: "i2c",
        manifest: include_str!("../manifests/i2c.json"),
        build: build_i2c,
        defaults: defaults_of::<I2cConfig>,
        open: open_as::<I2cEvent>,
        kinds: kinds_of::<I2cEvent>,
        bridge: bridge_of::<I2cEvent>,
    },
    Kind {
        name: "eeprom24xx",
        manifest: include_str!("../manifests/eeprom24xx.json"),
        build: build_eeprom24xx,
        defaults: defaults_of::<Eeprom24xxConfig>,
        open: open_as::<EepromEvent>,
        kinds: kinds_of::<EepromEvent>,
        bridge: bridge_of::<EepromEvent>,
    },
    Kind {
        name: "jtag",
        manifest: include_str!("../manifests/jtag.json"),
        build: build_jtag,
        defaults: defaults_of::<JtagConfig>,
        open: open_as::<JtagEvent>,
        kinds: kinds_of::<JtagEvent>,
        bridge: bridge_of::<JtagEvent>,
    },
    Kind {
        name: "swd",
        manifest: include_str!("../manifests/swd.json"),
        build: build_swd,
        defaults: defaults_of::<SwdConfig>,
        open: open_as::<SwdEvent>,
        kinds: kinds_of::<SwdEvent>,
        bridge: bridge_of::<SwdEvent>,
    },
    Kind {
        name: "sdcard-spi",
        manifest: include_str!("../manifests/sdcard-spi.json"),
        build: build_sdcard_spi,
        defaults: defaults_of::<SdCardSpiConfig>,
        open: open_as::<SdEvent>,
        kinds: kinds_of::<SdEvent>,
        bridge: bridge_of::<SdEvent>,
    },
    Kind {
        name: "can",
        manifest: include_str!("../manifests/can.json"),
        build: build_can,
        defaults: defaults_of::<CanConfig>,
        open: open_as::<CanEvent>,
        kinds: kinds_of::<CanEvent>,
        bridge: bridge_of::<CanEvent>,
    },
    Kind {
        name: "onewire-link",
        manifest: include_str!("../manifests/onewire-link.json"),
        build: build_onewire_link,
        defaults: defaults_of::<LinkConfig>,
        open: open_as::<LinkEvent>,
        kinds: kinds_of::<LinkEvent>,
        bridge: bridge_of::<LinkEvent>,
    },
    Kind {
        name: "onewire",
        manifest: include_str!("../manifests/onewire.json"),
        build: build_onewire,
        defaults: no_defaults,
        open: open_as::<OneWireEvent>,
        kinds: kinds_of::<OneWireEvent>,
        bridge: bridge_of::<OneWireEvent>,
    },
    Kind {
        name: "ds18b20",
        manifest: include_str!("../manifests/ds18b20.json"),
        build: build_ds18b20,
        defaults: no_defaults,
        open: open_as::<Ds18b20Event>,
        kinds: kinds_of::<Ds18b20Event>,
        bridge: bridge_of::<Ds18b20Event>,
    },
    Kind {
        name: "usb",
        manifest: include_str!("../manifests/usb.json"),
        build: build_usb,
        defaults: defaults_of::<UsbConfig>,
        open: open_as::<UsbEvent>,
        kinds: kinds_of::<UsbEvent>,
        bridge: bridge_of::<UsbEvent>,
    },
    Kind {
        name: "usb-transactions",
        manifest: include_str!("../manifests/usb-transactions.json"),
        build: build_usb_transactions,
        defaults: no_defaults,
        open: open_as::<TransactionEvent>,
        kinds: kinds_of::<TransactionEvent>,
        bridge: bridge_of::<TransactionEvent>,
    },
    Kind {
        name: "lin",
        manifest: include_str!("../manifests/lin.json"),
        build: build_lin,
        defaults: defaults_of::<LinConfig>,
        open: open_as::<LinEvent>,
        kinds: kinds_of::<LinEvent>,
        bridge: bridge_of::<LinEvent>,
    },
    Kind {
        name: "midi",
        manifest: include_str!("../manifests/midi.json"),
        build: build_midi,
        defaults: no_defaults,
        open: open_as::<MidiEvent>,
        kinds: kinds_of::<MidiEvent>,
        bridge: bridge_of::<MidiEvent>,
    },
    Kind {
        name: "ws281x",
        manifest: include_str!("../manifests/ws281x.json"),
        build: build_ws281x,
        defaults: defaults_of::<Ws281xConfig>,
        open: open_as::<Ws281xEvent>,
        kinds: kinds_of::<Ws281xEvent>,
        bridge: bridge_of::<Ws281xEvent>,
    },
    Kind {
        name: "dmx512",
        manifest: include_str!("../manifests/dmx512.json"),
        build: build_dmx512,
        defaults: no_defaults,
        open: open_as::<Dmx512Event>,
        kinds: kinds_of::<Dmx512Event>,
        bridge: bridge_of::<Dmx512Event>,
    },
    Kind {
        name: "apa102",
        manifest: include_str!("../manifests/apa102.json"),
        build: build_apa102,
        defaults: no_defaults,
        open: open_as::<Apa102Event>,
        kinds: kinds_of::<Apa102Event>,
        bridge: bridge_of::<Apa102Event>,
    },
    Kind {
        name: "mdio",
        manifest: include_str!("../manifests/mdio.json"),
        build: build_mdio,
        defaults: defaults_of::<MdioConfig>,
        open: open_as::<MdioEvent>,
        kinds: kinds_of::<MdioEvent>,
        bridge: bridge_of::<MdioEvent>,
    },
    Kind {
        name: "avr-isp",
        manifest: include_str!("../manifests/avr-isp.json"),
        build: build_avr_isp,
        defaults: no_defaults,
        open: open_as::<IspEvent>,
        kinds: kinds_of::<IspEvent>,
        bridge: bridge_of::<IspEvent>,
    },
    Kind {
        name: "nrf24l01",
        manifest: include_str!("../manifests/nrf24l01.json"),
        build: build_nrf24l01,
        defaults: no_defaults,
        open: open_as::<Nrf24Event>,
        kinds: kinds_of::<Nrf24Event>,
        bridge: bridge_of::<Nrf24Event>,
    },
    Kind {
        name: "usb-pd",
        manifest: include_str!("../manifests/usb-pd.json"),
        build: build_usb_pd,
        defaults: defaults_of::<UsbPdConfig>,
        open: open_as::<UsbPdEvent>,
        kinds: kinds_of::<UsbPdEvent>,
        bridge: bridge_of::<UsbPdEvent>,
    },
    Kind {
        name: "ir-nec",
        manifest: include_str!("../manifests/ir-nec.json"),
        build: build_ir_nec,
        defaults: defaults_of::<NecConfig>,
        open: open_as::<NecEvent>,
        kinds: kinds_of::<NecEvent>,
        bridge: bridge_of::<NecEvent>,
    },
    Kind {
        name: "ir-rc5",
        manifest: include_str!("../manifests/ir-rc5.json"),
        build: build_ir_rc5,
        defaults: defaults_of::<Rc5Config>,
        open: open_as::<Rc5Event>,
        kinds: kinds_of::<Rc5Event>,
        bridge: bridge_of::<Rc5Event>,
    },
    Kind {
        name: "ir-rc6",
        manifest: include_str!("../manifests/ir-rc6.json"),
        build: build_ir_rc6,
        defaults: defaults_of::<Rc6Config>,
        open: open_as::<Rc6Event>,
        kinds: kinds_of::<Rc6Event>,
        bridge: bridge_of::<Rc6Event>,
    },
    Kind {
        name: "ir-sirc",
        manifest: include_str!("../manifests/ir-sirc.json"),
        build: build_ir_sirc,
        defaults: defaults_of::<SircConfig>,
        open: open_as::<SircEvent>,
        kinds: kinds_of::<SircEvent>,
        bridge: bridge_of::<SircEvent>,
    },
    Kind {
        name: "wiegand",
        manifest: include_str!("../manifests/wiegand.json"),
        build: build_wiegand,
        defaults: defaults_of::<WiegandConfig>,
        open: open_as::<WiegandEvent>,
        kinds: kinds_of::<WiegandEvent>,
        bridge: bridge_of::<WiegandEvent>,
    },
    Kind {
        name: "ps2",
        manifest: include_str!("../manifests/ps2.json"),
        build: build_ps2,
        defaults: defaults_of::<Ps2Config>,
        open: open_as::<Ps2Event>,
        kinds: kinds_of::<Ps2Event>,
        bridge: bridge_of::<Ps2Event>,
    },
    Kind {
        name: "am230x",
        manifest: include_str!("../manifests/am230x.json"),
        build: build_am230x,
        defaults: defaults_of::<Am230xConfig>,
        open: open_as::<Am230xEvent>,
        kinds: kinds_of::<Am230xEvent>,
        bridge: bridge_of::<Am230xEvent>,
    },
    Kind {
        name: "microwire",
        manifest: include_str!("../manifests/microwire.json"),
        build: build_microwire,
        defaults: defaults_of::<MicrowireConfig>,
        open: open_as::<MicrowireEvent>,
        kinds: kinds_of::<MicrowireEvent>,
        bridge: bridge_of::<MicrowireEvent>,
    },
    Kind {
        name: "eeprom93xx",
        manifest: include_str!("../manifests/eeprom93xx.json"),
        build: build_eeprom93xx,
        defaults: defaults_of::<Eeprom93xxConfig>,
        open: open_as::<Eeprom93xxEvent>,
        kinds: kinds_of::<Eeprom93xxEvent>,
        bridge: bridge_of::<Eeprom93xxEvent>,
    },
    Kind {
        name: "qspi",
        manifest: include_str!("../manifests/qspi.json"),
        build: build_qspi,
        defaults: defaults_of::<QspiConfig>,
        open: open_as::<QspiEvent>,
        kinds: kinds_of::<QspiEvent>,
        bridge: bridge_of::<QspiEvent>,
    },
    Kind {
        name: "sdcard-sd",
        manifest: include_str!("../manifests/sdcard-sd.json"),
        build: build_sdcard_sd,
        defaults: defaults_of::<SdcardSdConfig>,
        open: open_as::<SdcardSdEvent>,
        kinds: kinds_of::<SdcardSdEvent>,
        bridge: bridge_of::<SdcardSdEvent>,
    },
    Kind {
        name: "swim",
        manifest: include_str!("../manifests/swim.json"),
        build: build_swim,
        defaults: defaults_of::<SwimConfig>,
        open: open_as::<SwimEvent>,
        kinds: kinds_of::<SwimEvent>,
        bridge: bridge_of::<SwimEvent>,
    },
    Kind {
        name: "jtag-dp",
        manifest: include_str!("../manifests/jtag-dp.json"),
        build: build_jtag_dp,
        defaults: defaults_of::<JtagDpConfig>,
        open: open_as::<JtagDpEvent>,
        kinds: kinds_of::<JtagDpEvent>,
        bridge: bridge_of::<JtagDpEvent>,
    },
];
