// SPDX-License-Identifier: Apache-2.0
/** CFP MSA MIS 2.6r06a §§2.2/5: one configured port, mandatory DEVAD 1.
 * Original implementation; DSView inventory consulted for region/identifier scope only. */
import type { Range, bool, u8, u16 } from 'uiharu'
import { StreamDecoder, defineStreamDecoder, eventSchema } from 'uiharu'
import { mdioEvents, type MdioEvent } from '../contracts/mdio'
export interface Config {
  phy: u8
}
export type Region =
  | 'base-id'
  | 'extended-id'
  | 'network-lane'
  | 'nvr4'
  | 'vendor1'
  | 'vendor2'
  | 'user1'
  | 'user2'
  | 'module-control'
  | 'mlg'
  | 'other'
export type Module =
  | 'unknown'
  | 'gbic'
  | 'soldered'
  | 'sfp'
  | 'xsbi'
  | 'xenpak'
  | 'xfp'
  | 'xff'
  | 'xfp-e'
  | 'xpak'
  | 'x2'
  | 'dwdm-sfp'
  | 'qsfp'
  | 'qsfp-plus'
  | 'cfp'
  | 'cxp'
  | 'cfp2'
  | 'cfp4'
  | 'reserved'
export type Reason = 'input-error' | 'turnaround' | 'missing-address'
export type Event =
  | { kind: 'address'; address: u16 }
  | { kind: 'register'; address: u16; read: bool; value: u16; region: Region }
  | { kind: 'identifier'; value: u16; module: Module }
  | { kind: 'error'; reason: Reason }
export const events = eventSchema<Event>('cfp.events', 1).roles({
  address: 'address',
  register: 'data',
  identifier: 'control',
  error: 'error',
})
function region(a: u16): Region {
  if (a >= 0x8000 && a <= 0x807f) return 'base-id'
  if (a >= 0x8080 && a <= 0x80ff) return 'extended-id'
  if (a >= 0x8100 && a <= 0x817f) return 'network-lane'
  if (a >= 0x8180 && a <= 0x81ff) return 'nvr4'
  if (a >= 0x8400 && a <= 0x847f) return 'vendor1'
  if (a >= 0x8480 && a <= 0x84ff) return 'vendor2'
  if (a >= 0x8800 && a <= 0x887f) return 'user1'
  if (a >= 0x8880 && a <= 0x88ff) return 'user2'
  if (a >= 0xa000 && a <= 0xa07f) return 'module-control'
  if (a >= 0xa080 && a <= 0xa0ff) return 'mlg'
  return 'other'
}
function moduleId(v: u16): Module {
  switch (v) {
    case 0:
      return 'unknown'
    case 1:
      return 'gbic'
    case 2:
      return 'soldered'
    case 3:
      return 'sfp'
    case 4:
      return 'xsbi'
    case 5:
      return 'xenpak'
    case 6:
      return 'xfp'
    case 7:
      return 'xff'
    case 8:
      return 'xfp-e'
    case 9:
      return 'xpak'
    case 10:
      return 'x2'
    case 11:
      return 'dwdm-sfp'
    case 12:
      return 'qsfp'
    case 13:
      return 'qsfp-plus'
    case 14:
      return 'cfp'
    case 15:
      return 'cxp'
    case 17:
      return 'cfp2'
    case 18:
      return 'cfp4'
    default:
      return 'reserved'
  }
}
class Cfp extends StreamDecoder<Config, MdioEvent, Event> {
  address: u16 | undefined = undefined
  // eslint-disable-next-line @typescript-eslint/require-await -- Stream lifecycle hook has an async signature.
  override async onGap(_range: Range): Promise<void> {
    this.address = undefined
  }
  override async run(): Promise<void> {
    for (;;) {
      const r = await this.input.next()
      const e = r.event
      if (e.kind === 'error') {
        this.address = undefined
        await this.out.emit({ kind: 'error', reason: 'input-error' }, r.at)
        continue
      }
      if (e.kind === 'frame') {
        if (e.clause !== 45 || e.phy !== this.config.phy || e.reg !== 1) continue
        const read = e.op === 'read' || e.op === 'post-read-increment'
        if (read ? (e.turnaround & 1) !== 0 : e.turnaround !== 2) {
          this.address = undefined
          await this.out.emit({ kind: 'error', reason: 'turnaround' }, r.at)
          continue
        }
        if (e.op === 'address') {
          this.address = e.data
          await this.out.emit({ kind: 'address', address: e.data }, r.at)
          continue
        }
        const address = this.address
        if (address === undefined) {
          await this.out.emit({ kind: 'error', reason: 'missing-address' }, r.at)
          continue
        }
        await this.out.emit(
          { kind: 'register', address, read, value: e.data, region: region(address) },
          r.at,
        )
        if (address === 0x8000 && read)
          await this.out.emit({ kind: 'identifier', value: e.data, module: moduleId(e.data) }, r.at)
        if (e.op === 'post-read-increment') this.address = (address + 1) & 0xffff
      }
    }
  }
}
export const cfp = defineStreamDecoder({
  name: 'cfp',
  category: 'device',
  config: {
    defaults: { phy: 0 },
    validate: (config: Config) => (config.phy > 31 ? 'decode.config.phy' : undefined),
  },
  input: mdioEvents,
  output: events,
  decoder: Cfp,
  locales: {
    'en-US': {
      name: 'CFP management',
      summary: 'Clause 45 DEVAD 1 register regions and module identifier',
      fields: { phy: 'MDIO port' },
      events: {
        address: 'Register address',
        register: 'Register access',
        identifier: 'Module identifier',
        error: 'Protocol error',
      },
      eventFields: {
        address: 'Address',
        read: 'Read',
        value: 'Value',
        region: 'Register region',
        module: 'Module type',
        reason: 'Error reason',
      },
    },
    'zh-CN': {
      name: 'CFP管理',
      summary: 'Clause 45设备地址1寄存器区域和模块标识',
      fields: { phy: 'MDIO端口' },
      events: {
        address: '寄存器地址',
        register: '寄存器访问',
        identifier: '模块标识',
        error: '协议错误',
      },
      eventFields: {
        address: '地址',
        read: '读取',
        value: '数值',
        region: '寄存器区域',
        module: '模块类型',
        reason: '错误原因',
      },
    },
  },
})
