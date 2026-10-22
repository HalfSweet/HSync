# CFP management

`cfp` consumes canonical native `mdio.events@1`. `phy` selects port 0–31 (default
0); CFP requires Clause 45 and device address 1. Clause 22 and other ports/devices
are ignored. Each valid address frame sets this selected device's register
pointer; read/write accesses consume it and post-read-increment advances it modulo 65536. A gap, native framing error or bad turnaround invalidates the pointer.
An access with no observed address reports `missing-address`.

The original TypeScript consumer reports address, read/write direction, register
value and all ten regions covered by the pinned reference: base/extended/network
lane/NVR4, vendor1/2, user1/2, module-control and MLG. Register 8000h reads name
module identifiers, retaining reserved/unknown values. Writes remain visible.
This is the reference's region/identifier profile; further per-register DDM,
command-engine semantics and vendor extensions are outside its scope.

The primary source is the CFP MSA's [Management Interface Specification 2.6r06a](https://approvednetworks.com/content/msa/CFP_MSA_MIS.pdf),
§2.2 (mandatory DEVAD 1), management frame structure, register allocation and base
ID table. The pinned DSView annotation inventory supplied scope only; no GPL code
was copied or adapted. `uiharu-interp/tests/p4_devices.rs` checks address pairing,
unrelated traffic, increments, identifiers, gaps, invalid turnaround, exact
compiled/interpreted events and bounded scheduling.
