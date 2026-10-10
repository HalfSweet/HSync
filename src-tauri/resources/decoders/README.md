# Protocol decoders

The sigrok-compatible Python protocol decoders that DSView ships, copied unchanged from
`DSView/libsigrokdecode4DSL/decoders`. They are free software under the GNU General Public
License, version 2 or (13 of them) version 3, or any later version, as each file's header says;
HSync (GPL-3.0-or-later) runs them through its `sigrokdecode` host (`crates/hsync-srd`,
DESIGN.md §7.3).

Each directory is a Python package with `__init__.py` and `pd.py`; `common` holds helpers some
decoders import. Directory names such as `1-i2c` are the decoder ids DSView saves in `.dsl`
files. Natively decoded buses (I²C, UART, SPI, CAN, CAN-FD) do not run these copies unless a
stacked decoder needs their Python output.
