// SPDX-License-Identifier: Apache-2.0
/* eslint-disable @typescript-eslint/no-unnecessary-type-assertion -- Integer assertions select IR widths and truncation; TypeScript aliases alone do not encode these semantics. */

/** Chain selection from the IEEE1149.1 scan concatenation model; explicit wire-order
 * before/selected/after widths, never guess topology from a captured bit pattern.
 */
import type { u8, u16, u32, u64, Bounded, Tick, Range } from 'uiharu'
import { StreamDecoder, defineStreamDecoder, eventSchema, over } from 'uiharu'
export type TapState =
  | 'test-logic-reset'
  | 'run-test-idle'
  | 'select-dr-scan'
  | 'capture-dr'
  | 'shift-dr'
  | 'exit1-dr'
  | 'pause-dr'
  | 'exit2-dr'
  | 'update-dr'
  | 'select-ir-scan'
  | 'capture-ir'
  | 'shift-ir'
  | 'exit1-ir'
  | 'pause-ir'
  | 'exit2-ir'
  | 'update-ir'
export type Register = 'ir' | 'dr'
export type ResetBy = 'tms' | 'trst'
export type EndReason = 'complete' | 'aborted'
export type JtagEvent =
  | { kind: 'state'; state: TapState }
  | { kind: 'reset'; by: ResetBy }
  | { kind: 'scan-begin'; scan: u64; register: Register }
  | {
      kind: 'shift'
      scan: u64
      offset: u32
      tdi: Bounded<u8, 32> | undefined
      tdo: Bounded<u8, 32> | undefined
      bits: u16
    }
  | { kind: 'scan-end'; scan: u64; bits: u32; reason: EndReason }
export const jtagEvents = eventSchema<JtagEvent>('jtag.events', 1).roles({
  state: 'framing',
  reset: 'line',
  'scan-begin': 'framing',
  shift: 'data',
  'scan-end': 'framing',
})
export interface JtagChainConfig {
  irBefore: u32
  irBits: u16
  irAfter: u32
  drBefore: u32
  drBits: u32
  drAfter: u32
}
function validate(config: JtagChainConfig): string | undefined {
  if (
    config.irBefore > 4096 ||
    config.irAfter > 4096 ||
    config.drBefore > 4096 ||
    config.drAfter > 4096 ||
    config.irBits < 1 ||
    config.irBits > 256 ||
    config.drBits < 1 ||
    config.drBits > 4096
  )
    return 'decode.config.chainWidths'
  return undefined
}
class JtagChain extends StreamDecoder<JtagChainConfig, JtagEvent, JtagEvent> {
  scan: u64 | undefined = undefined
  before: u32 = 0
  width: u32 = 0
  after: u32 = 0
  selected: u32 = 0
  total: u32 = 0
  malformed = false
  async abort(tick: Tick): Promise<void> {
    const scan = this.scan
    if (scan !== undefined)
      await this.out.emit(
        { kind: 'scan-end', scan, bits: this.selected, reason: 'aborted' },
        over(tick, tick, tick),
      )
    this.scan = undefined
  }
  override async onGap(range: Range): Promise<void> {
    await this.abort(range.start)
  }
  override async onEnd(tick: Tick): Promise<void> {
    await this.abort(tick)
  }
  override async run(): Promise<void> {
    for (;;) {
      const item = await this.input.next()
      const event = item.event
      if (event.kind === 'state') {
        await this.out.emit(event, item.at)
        continue
      }
      if (event.kind === 'reset') {
        this.scan = undefined
        await this.out.emit(event, item.at)
        continue
      }
      if (event.kind === 'scan-begin') {
        this.scan = event.scan
        this.before = event.register === 'ir' ? this.config.irBefore : this.config.drBefore
        this.width = event.register === 'ir' ? this.config.irBits : this.config.drBits
        this.after = event.register === 'ir' ? this.config.irAfter : this.config.drAfter
        this.selected = 0
        this.total = 0
        this.malformed = false
        await this.out.emit(event, item.at)
        continue
      }
      if (event.kind === 'shift') {
        if (this.scan !== event.scan) continue
        if (event.offset !== this.total) this.malformed = true
        const tdi = event.tdi
        const tdo = event.tdo
        if (tdi !== undefined) {
          if (tdo !== undefined) {
            if (event.bits > 256) {
              this.malformed = true
              continue
            }
            let index: u16 = 0
            while (index < event.bits) {
              const position: u32 = event.offset + index
              if (position >= this.before && position - this.before < this.width) {
                const inputByte = tdi[index >> 3]
                const outputByte = tdo[index >> 3]
                if (inputByte !== undefined) {
                  if (outputByte !== undefined) {
                    const input: Bounded<u8, 32> = [((inputByte >> (index & 7)) & 1) as u8]
                    const output: Bounded<u8, 32> = [((outputByte >> (index & 7)) & 1) as u8]
                    await this.out.emit(
                      {
                        kind: 'shift',
                        scan: event.scan,
                        offset: position - this.before,
                        tdi: input,
                        tdo: output,
                        bits: 1,
                      },
                      item.at,
                    )
                    this.selected += 1
                  } else this.malformed = true
                } else this.malformed = true
              }
              index += 1
            }
          } else this.malformed = true
        } else this.malformed = true
        this.total = event.offset + event.bits
      }
      if (event.kind === 'scan-end') {
        if (this.scan !== event.scan) continue
        const valid =
          !this.malformed &&
          this.total === event.bits &&
          event.bits === this.before + this.width + this.after &&
          this.selected === this.width
        await this.out.emit(
          {
            kind: 'scan-end',
            scan: event.scan,
            bits: this.selected,
            reason: event.reason === 'complete' && valid ? 'complete' : 'aborted',
          },
          item.at,
        )
        this.scan = undefined
      }
    }
  }
}
export const jtagChain = defineStreamDecoder({
  name: 'jtag-chain',
  category: 'device',
  config: {
    defaults: { irBefore: 0, irBits: 4, irAfter: 0, drBefore: 0, drBits: 35, drAfter: 0 },
    validate,
  },
  input: jtagEvents,
  requires: () => ({ tdi: true, tdo: true }),
  output: jtagEvents,
  props: () => ({ tdi: true, tdo: true }),
  locales: {
    'en-US': {
      name: 'JTAG chain selection',
      summary: 'Select an explicitly sized TAP scan window; reject mismatched chains',
      fields: {
        irBefore: 'IR bits before TAP',
        irBits: 'Selected IR width',
        irAfter: 'IR bits after TAP',
        drBefore: 'DR bits before TAP',
        drBits: 'Selected DR width',
        drAfter: 'DR bits after TAP',
      },
      events: {
        state: 'TAP state',
        reset: 'Reset',
        'scan-begin': 'Scan begin',
        shift: 'Selected bit',
        'scan-end': 'Scan end',
      },
      eventFields: {
        state: 'State',
        by: 'Reset source',
        scan: 'Scan',
        register: 'Register',
        offset: 'Bit offset',
        tdi: 'TDI bits',
        tdo: 'TDO bits',
        bits: 'Bits',
        reason: 'End reason',
      },
    },
    'zh-CN': {
      name: 'JTAG 链选择',
      summary: '选择指定宽度的 TAP 扫描窗口并拒绝长度不匹配',
      fields: {
        irBefore: 'TAP 前 IR 位数',
        irBits: '选中 IR 宽度',
        irAfter: 'TAP 后 IR 位数',
        drBefore: 'TAP 前 DR 位数',
        drBits: '选中 DR 宽度',
        drAfter: 'TAP 后 DR 位数',
      },
      events: {
        state: 'TAP 状态',
        reset: '复位',
        'scan-begin': '扫描开始',
        shift: '选中位',
        'scan-end': '扫描结束',
      },
      eventFields: {
        state: '状态',
        by: '复位源',
        scan: '扫描',
        register: '寄存器',
        offset: '位偏移',
        tdi: 'TDI 位',
        tdo: 'TDO 位',
        bits: '位数',
        reason: '结束原因',
      },
    },
  },
  decoder: JtagChain,
})
