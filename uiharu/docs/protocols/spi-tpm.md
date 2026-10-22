<!-- SPDX-License-Identifier: Apache-2.0 -->

# TPM SPI interface

`spi-tpm` decodes the four-byte header, read/write direction, 24-bit address,1–64-byte payload length and MISO wait-state flow control. Header and interface-register events span all four header bytes. Readiness bytes are excluded from the payload.

For canonical D4-prefixed addresses, locality0–4 and interface register names follow TIS/PTP: access, interrupt enable/vector/status, interface capability, status, data FIFO, device/vendor ID and revision. TPM2.0 adds interface ID and extended FIFO names. Locality4 hash start/end/data aliases and TPM1.2 locality0 legacy registers are explicit; unknown/noncanonical addresses remain unknown. Version selection changes the named register profile rather than pretending to parse TPM authorization or command packets.

References: [ST33TPHF2XSPI](https://www.st.com/resource/en/datasheet/st33tphf2xspi.pdf) for SPI framing; [TCG TPM1.2 table10](https://trustedcomputinggroup.org/wp-content/uploads/TCG_PCClientTPMSpecification_1-20_1-00_FINAL.pdf) and [TCG PTP1.03 register space](https://trustedcomputinggroup.org/wp-content/uploads/TCG_PC_Client_Platform_TPM_Profile_PTP_2.0_r1.03_v22.pdf) for names. Tests cover two readiness bytes, payload length, locality/version boundaries, unknown addresses and compiled/interpreted scheduling. Command/auth/session parsing and CRB are outside this profile.
