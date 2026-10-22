# Device overlays requiring additional DSL capabilities

> Backend follow-up: the previously recorded shared-output, fixed-array,
> hexadecimal-u64, numeric-enum and mixed-enum limitations have been implemented
> in the [DSL compiler and both execution paths](../dsl.md#21-types). The failure
> probes below are historical evidence from the initial P0–P4 checkpoint.
> The continuation now supplies cJTAG, SWD/AVR semantics, 1-Wire devices, CFP
> and AM230x values using canonical native/DSL event types. See the
> [current execution record](../../../docs/decoder-coverage.md#p0p4-continuation-after-the-backend-follow-up).

These were deferred at the first checkpoint. They are now implemented with the
original wire contracts: [DS2408](ds2408.md), [DS243x](ds243x.md),
[DS28EA00](ds28ea00.md), [CFP](cfp.md) and [AM230x values](am230x.md).
The original failure evidence below is retained for language-design history.

## DS2408, DS243x and DS28EA00

The existing native `onewire.events@1` contract carries ROM/Search `id: u64`
with `#[serde(with = "uiharu_core::hex_u64")]`. Its JSON representation is a
hexadecimal string. A minimal DSL declaration, `type Event = { kind: 'rom'; id:
u64 }`, generates numeric JSON; the interpreter's integer conversion uses
`Value::as_u64` and rejects the native string. Declaring `id: string` preserves
JSON but changes the compiled producer's public, numeric ROM API. The existing
graph's compiled connection requires identical typed contracts. There is no
transparent type alias that preserves both the validated native API and its
serialization here.

Required capability: declare the existing hexadecimal-u64 field serialization
in a DSL contract, with matching generated Rust and interpreter behavior. A
special registry adapter or a replacement lower bus would evade this limitation
and is deliberately not introduced. The existing native DS18B20 remains available.

## CFP over MDIO

CFP management uses the native MDIO stream, including Clause 45 addressing.
`MdioEvent::Frame.clause` is the validated native `Clause` enum, whose accepted
wire JSON values are exactly numbers 22 and 45 (`serde(into = "u8", try_from =
"u8")`). The desired minimal contract declaration is:

```typescript
type Clause = 22 | 45
type Event = { kind: 'frame'; clause: Clause }
```

The current compiler rejects this at the Clause field: `the type Clause has no
IR type; name a record or give an integer its width`. Declaring an unconstrained
`u8` loses the native validation/API; a string enum changes JSON. The generated
compiled port cannot consume the existing native enum without a new adapter.

Required capability: a validated numeric enum with exact wire values and matching
native/generated/interpreted contract types. No backend extension or adapter is
included. CFP is therefore deferred, not classified as an I²C overlay.

References: the native `onewire/events.rs`, `mdio/events.rs`, the interpreter's
`json.rs`, and the DSL documentation's deferred field serialization attributes;
[CFP MSA Management Interface Specification](https://approvednetworks.com/content/msa/CFP_MSA_MIS.pdf).

## Existing source workarounds (not protocol blockers)

A switch nested inside an item-reading loop currently lowers a nested `break`
to Rust's loop `break`; it cannot express an early switch-case exit faithfully.
The device overlays use explicit `continue` to the item loop instead, including
unselected addresses and malformed/NACK segments. A regression for all fifteen
I²C profiles places an unrelated address/data segment before a selected repeated
START in the same transaction. No backend semantics were changed.

Variable shift counts in generated helpers may require explicit `as u64` in the
source. ATSHA204A's bounded CRC loop supplies this cast. These are source-level
workarounds for expressible protocols, not reasons to skip a decoder.
