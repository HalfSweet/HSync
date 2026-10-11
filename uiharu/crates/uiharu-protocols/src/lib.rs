// SPDX-License-Identifier: Apache-2.0

//! The protocol decoders of uiharu, each a module with its contract (the events it emits),
//! its decoder node, a waveform synthesizer and tests: buses decoded from logic samples (SPI,
//! I²C, UART, CAN, 1-Wire, USB, USB PD, I²S, PWM, timing, parallel, WS281x, JTAG, SWD, MDIO,
//! the NEC, RC-5, RC-6 and SIRC infrared remote controls) and protocols decoded from a bus's
//! events (SPI NOR flash, SD cards, APA102 LEDs, AVR serial programming and the nRF24L01+ over
//! SPI, 24xx EEPROMs over I²C, Modbus RTU, LIN, MIDI and DMX512 over UART, the 1-Wire network
//! layer and the DS18B20 over 1-Wire, transactions over USB packets).

#![forbid(unsafe_code)]

pub mod am230x;
pub mod apa102;
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
pub mod lin;
pub mod mdio;
pub mod microwire;
pub mod midi;
pub mod modbus_rtu;
pub mod nrf24l01;
pub mod onewire;
pub mod parallel;
pub mod ps2;
pub mod pwm;
pub mod sdcard_spi;
pub mod spi;
pub mod spi_nor;
pub mod swd;
pub mod timing;
pub mod uart;
pub mod usb;
pub mod usb_pd;
pub mod wiegand;
pub mod ws281x;
