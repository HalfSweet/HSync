// SPDX-License-Identifier: Apache-2.0

//! The protocol decoders of uiharu, each a module with its contract (the events it emits),
//! its decoder node, a waveform synthesizer and tests: buses decoded from logic samples (SPI,
//! I²C, UART, CAN, 1-Wire, USB, USB PD, I²S, PWM, timing, parallel, WS281x, JTAG, SWD, MDIO,
//! Microwire, PS/2, Wiegand, the AM2302/DHT22/DHT11 sensor line, the NEC, RC-5, RC-6 and SIRC
//! infrared remote controls) and protocols decoded from a bus's events (SPI NOR flash, SD
//! cards, APA102 LEDs, AVR serial programming and the nRF24L01+ over SPI, 93Cxx EEPROMs over
//! Microwire, 24xx EEPROMs over I²C, Modbus RTU, LIN, MIDI and DMX512 over UART, the 1-Wire
//! network layer and the DS18B20 over 1-Wire, transactions over USB packets).

#![forbid(unsafe_code)]

pub use uiharu_dsl_protocols::{ad79x0, avr_pdi, c2};

pub use uiharu_dsl_protocols::{
    am230x_values, avr_isp_device, cfp, cjtag, dmx512_analysis, ds28ea00, ds243x, ds2408,
    lin_profile, midi_semantics, swd_dp,
};

pub use uiharu_dsl_protocols::{
    adxl345, amulet_ascii, atsha204a, aud, ds1307, edid, eeprom24xx_memory, lm75, ltc26x7,
    mcp230xx, mlx90614, mxc6225xu, nunchuk, pan1321, pca9571, rtc8564, tca6408a, xfp,
};

pub use uiharu_dsl_protocols::{
    a7105, ad5626, ade77xx, adf435x, adns5020, adxl345_spi, cc1101, enc28j60, ltc242x, max72xx,
    max7219, mrf24j40, nrf905, qspi_flash, rfm12, rgb_led_spi, sd_native, sd_registers, sda2506,
    spi_tpm, st25r39xx_spi, st7735, st7789, tlc5620, x2444m, xn297,
};

pub mod am230x;
pub mod apa102;
pub mod arm_etmv3;
pub mod arm_itm;
pub mod arm_memap;
pub mod arm_trace_transport;
pub mod avr_isp;
pub mod can;
pub mod dmx512;
pub mod ds18b20;
pub mod eeprom24xx;
pub mod eeprom93xx;
pub mod i2c;
pub mod i2s;
pub mod ir_nec;
pub mod ir_rc5;
pub mod ir_rc6;
pub mod ir_sirc;
pub mod jtag;
pub mod jtag_chain;
pub mod jtag_ejtag;
pub mod jtag_stm32;
pub mod lin;
pub mod mdio;
pub mod microwire;
pub mod midi;
pub mod modbus_pdu;
pub mod modbus_rtu;
pub mod nrf24l01;
pub mod onewire;
pub mod parallel;
pub mod ps2;
pub mod pwm;
pub mod sdcard_sd;
pub mod sdcard_spi;
pub mod spi;
pub mod spi_nor;
pub mod swd;
pub mod swim;
pub mod swim_stm8;
pub mod timing;
pub mod uart;
pub mod usb;
pub mod usb_pd;
pub mod wiegand;
pub mod ws281x;

pub mod qspi;

pub mod jtag_dp;

pub use uiharu_dsl_protocols::{fsi, ps2_device, smbus};
