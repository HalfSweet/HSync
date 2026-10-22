// SPDX-License-Identifier: Apache-2.0
/* eslint-disable @typescript-eslint/no-unnecessary-type-assertion -- DSL integer widths and empty configuration records carry IR meaning. */
/** Explicit ADE7753/ADE7758 models with their distinct addresses, register lengths and signed codes; byte-stream commands may run without CS.
 * Public manufacturer specification: https://www.analog.com/media/en/technical-documentation/data-sheets/ade7753.pdf .
 * Each transfer is bounded state and streaming data. Partial CS frames are skipped;
 * missing required data closes incomplete/aborted rather than inventing bytes. */
import type { bool, Provenance, Tick, Range, u8, u16, u32, u64 } from 'uiharu'
import { StreamDecoder, at, defineStreamDecoder, eventSchema, join, oneOf } from 'uiharu'
import { spiEvents, type SpiEvent } from './spi'
export type End = 'complete' | 'incomplete' | 'aborted' | 'unsupported'
export type Model = 'ade7753' | 'ade7758'
export type Register =
  | 'awatthr'
  | 'bwatthr'
  | 'cwatthr'
  | 'avarhr'
  | 'bvarhr'
  | 'cvarhr'
  | 'avahr'
  | 'bvahr'
  | 'cvahr'
  | 'airms'
  | 'birms'
  | 'cirms'
  | 'avrms'
  | 'bvrms'
  | 'cvrms'
  | 'freq'
  | 'temp'
  | 'wform'
  | 'opmode'
  | 'mmode'
  | 'wavmode'
  | 'compmode'
  | 'lcycmode'
  | 'mask'
  | 'status'
  | 'rstatus'
  | 'zxtout'
  | 'linecyc'
  | 'sagcyc'
  | 'saglvl'
  | 'vpintlvl'
  | 'ipintlvl'
  | 'vpeak'
  | 'ipeak'
  | 'avrmsgain'
  | 'bvrmsgain'
  | 'cvrmsgain'
  | 'aigain'
  | 'bigain'
  | 'cigain'
  | 'awg'
  | 'bwg'
  | 'cwg'
  | 'avarg'
  | 'bvarg'
  | 'cvarg'
  | 'avag'
  | 'bvag'
  | 'cvag'
  | 'avrmsos'
  | 'bvrmsos'
  | 'cvrmsos'
  | 'airmsos'
  | 'birmsos'
  | 'cirmsos'
  | 'awattos'
  | 'bwattos'
  | 'cwattos'
  | 'avaros'
  | 'bvaros'
  | 'cvaros'
  | 'aphcal'
  | 'bphcal'
  | 'cphcal'
  | 'wdiv'
  | 'vardiv'
  | 'vadiv'
  | 'apcfnum'
  | 'apcfden'
  | 'varcfnum'
  | 'varcfden'
  | 'chksum'
  | 'version'
  | 'waveform'
  | 'active-energy'
  | 'reset-active-energy'
  | 'line-active-energy'
  | 'apparent-energy'
  | 'reset-apparent-energy'
  | 'line-apparent-energy'
  | 'line-reactive-energy'
  | 'mode'
  | 'interrupt-enable'
  | 'interrupt-status'
  | 'reset-interrupt-status'
  | 'channel1-offset'
  | 'channel2-offset'
  | 'gain'
  | 'phase-calibration'
  | 'power-offset'
  | 'power-gain'
  | 'power-divider'
  | 'cf-numerator'
  | 'cf-denominator'
  | 'current-rms'
  | 'voltage-rms'
  | 'current-rms-offset'
  | 'voltage-rms-offset'
  | 'apparent-gain'
  | 'apparent-divider'
  | 'line-cycles'
  | 'zero-cross-timeout'
  | 'sag-cycles'
  | 'sag-level'
  | 'current-peak-level'
  | 'voltage-peak-level'
  | 'current-peak'
  | 'reset-current-peak'
  | 'voltage-peak'
  | 'reset-voltage-peak'
  | 'temperature'
  | 'period'
  | 'test-mode'
  | 'checksum'
  | 'revision'
export type Event =
  | { kind: 'begin'; tx: u64 }
  | { kind: 'command'; tx: u64; opcode: u8; address: u16; read: bool; status: u8 | undefined }
  | { kind: 'data'; tx: u64; offset: u32; address: u16; value: u8; read: bool }
  | {
      kind: 'register'
      tx: u64
      register: Register
      read: bool
      bits: u8
      raw: u32
      negative: bool
      magnitude: u32
    }
  | {
      kind: 'mode'
      tx: u64
      highPassDisabled: bool
      lowPassDisabled: bool
      cfDisabled: bool
      sagDisabled: bool
      suspended: bool
      temperatureStart: bool
      reset: bool
      lineCycle: bool
      channel1Disabled: bool
      channel2Disabled: bool
      swap: bool
      sampleRate: u8
      waveform: u8
      positiveOnly: bool
    }
  | { kind: 'gain'; tx: u64; channel1: u8; channel2: u8; inputRange: u8 }
  | {
      kind: 'operational-mode'
      tx: u64
      highPassDisabled: bool
      lowPassDisabled: bool
      cfDisabled: bool
      disableMode: u8
      reset: bool
    }
  | { kind: 'end'; tx: u64; reason: End }
export const events = eventSchema<Event>('ade77xx.events', 1).roles({
  begin: 'framing',
  command: 'command',
  data: 'data',
  register: 'data',
  mode: 'control',
  gain: 'control',
  'operational-mode': 'control',
  end: 'framing',
})
export interface Config {
  model: Model
  framed: bool
}
class Device extends StreamDecoder<Config, SpiEvent, Event> {
  stage: u8 = 0
  tx: u64 = 0
  address: u16 = 0
  read = false
  count: u8 = 0
  target: u8 = 0
  raw: u32 = 0
  registerAt: Provenance | undefined = undefined
  override hold(): Tick | undefined {
    return this.registerAt?.span.start
  }
  async abort(where: Provenance, reason: End): Promise<void> {
    if (this.stage === 1) await this.out.emit({ kind: 'end', tx: this.tx, reason }, where)
    this.stage = 2
    this.count = 0
    this.raw = 0
    this.registerAt = undefined
  }
  override async onGap(range: Range): Promise<void> {
    await this.abort(at(range.start), 'aborted')
  }
  override async onEnd(tick: Tick): Promise<void> {
    await this.abort(at(tick), 'incomplete')
  }
  bits(address: u16): u8 {
    if (this.config.model === 'ade7758') {
      if (address === 1) return 16
      if (address === 2) return 16
      if (address === 3) return 16
      if (address === 4) return 16
      if (address === 5) return 16
      if (address === 6) return 16
      if (address === 7) return 16
      if (address === 8) return 16
      if (address === 9) return 16
      if (address === 10) return 24
      if (address === 11) return 24
      if (address === 12) return 24
      if (address === 13) return 24
      if (address === 14) return 24
      if (address === 15) return 24
      if (address === 16) return 12
      if (address === 17) return 8
      if (address === 18) return 24
      if (address === 19) return 8
      if (address === 20) return 8
      if (address === 21) return 8
      if (address === 22) return 8
      if (address === 23) return 8
      if (address === 24) return 24
      if (address === 25) return 24
      if (address === 26) return 24
      if (address === 27) return 16
      if (address === 28) return 16
      if (address === 29) return 8
      if (address === 30) return 8
      if (address === 31) return 8
      if (address === 32) return 8
      if (address === 33) return 8
      if (address === 34) return 8
      if (address === 35) return 8
      if (address === 36) return 12
      if (address === 37) return 12
      if (address === 38) return 12
      if (address === 39) return 12
      if (address === 40) return 12
      if (address === 41) return 12
      if (address === 42) return 12
      if (address === 43) return 12
      if (address === 44) return 12
      if (address === 45) return 12
      if (address === 46) return 12
      if (address === 47) return 12
      if (address === 48) return 12
      if (address === 49) return 12
      if (address === 50) return 12
      if (address === 51) return 12
      if (address === 52) return 12
      if (address === 53) return 12
      if (address === 54) return 12
      if (address === 55) return 12
      if (address === 56) return 12
      if (address === 57) return 12
      if (address === 58) return 12
      if (address === 59) return 12
      if (address === 60) return 12
      if (address === 61) return 12
      if (address === 62) return 12
      if (address === 63) return 7
      if (address === 64) return 7
      if (address === 65) return 7
      if (address === 66) return 8
      if (address === 67) return 8
      if (address === 68) return 8
      if (address === 69) return 16
      if (address === 70) return 12
      if (address === 71) return 16
      if (address === 72) return 12
      if (address === 126) return 8
      if (address === 127) return 8
      return 0
    }

    if (address === 1) return 24
    if (address === 2) return 24
    if (address === 3) return 24
    if (address === 4) return 24
    if (address === 5) return 24
    if (address === 6) return 24
    if (address === 7) return 24
    if (address === 8) return 24
    if (address === 9) return 16
    if (address === 10) return 16
    if (address === 11) return 16
    if (address === 12) return 16
    if (address === 13) return 8
    if (address === 14) return 8
    if (address === 15) return 8
    if (address === 16) return 6
    if (address === 17) return 16
    if (address === 18) return 12
    if (address === 19) return 8
    if (address === 20) return 12
    if (address === 21) return 12
    if (address === 22) return 24
    if (address === 23) return 24
    if (address === 24) return 12
    if (address === 25) return 12
    if (address === 26) return 12
    if (address === 27) return 8
    if (address === 28) return 16
    if (address === 29) return 12
    if (address === 30) return 8
    if (address === 31) return 8
    if (address === 32) return 8
    if (address === 33) return 8
    if (address === 34) return 24
    if (address === 35) return 24
    if (address === 36) return 24
    if (address === 37) return 24
    if (address === 38) return 8
    if (address === 39) return 16
    if (address === 61) return 8
    if (address === 63) return 8
    if (address === 62) return 6
    return 0
  }
  name(address: u16): Register {
    if (this.config.model === 'ade7758') {
      if (address === 1) return 'awatthr'
      if (address === 2) return 'bwatthr'
      if (address === 3) return 'cwatthr'
      if (address === 4) return 'avarhr'
      if (address === 5) return 'bvarhr'
      if (address === 6) return 'cvarhr'
      if (address === 7) return 'avahr'
      if (address === 8) return 'bvahr'
      if (address === 9) return 'cvahr'
      if (address === 10) return 'airms'
      if (address === 11) return 'birms'
      if (address === 12) return 'cirms'
      if (address === 13) return 'avrms'
      if (address === 14) return 'bvrms'
      if (address === 15) return 'cvrms'
      if (address === 16) return 'freq'
      if (address === 17) return 'temp'
      if (address === 18) return 'wform'
      if (address === 19) return 'opmode'
      if (address === 20) return 'mmode'
      if (address === 21) return 'wavmode'
      if (address === 22) return 'compmode'
      if (address === 23) return 'lcycmode'
      if (address === 24) return 'mask'
      if (address === 25) return 'status'
      if (address === 26) return 'rstatus'
      if (address === 27) return 'zxtout'
      if (address === 28) return 'linecyc'
      if (address === 29) return 'sagcyc'
      if (address === 30) return 'saglvl'
      if (address === 31) return 'vpintlvl'
      if (address === 32) return 'ipintlvl'
      if (address === 33) return 'vpeak'
      if (address === 34) return 'ipeak'
      if (address === 35) return 'gain'
      if (address === 36) return 'avrmsgain'
      if (address === 37) return 'bvrmsgain'
      if (address === 38) return 'cvrmsgain'
      if (address === 39) return 'aigain'
      if (address === 40) return 'bigain'
      if (address === 41) return 'cigain'
      if (address === 42) return 'awg'
      if (address === 43) return 'bwg'
      if (address === 44) return 'cwg'
      if (address === 45) return 'avarg'
      if (address === 46) return 'bvarg'
      if (address === 47) return 'cvarg'
      if (address === 48) return 'avag'
      if (address === 49) return 'bvag'
      if (address === 50) return 'cvag'
      if (address === 51) return 'avrmsos'
      if (address === 52) return 'bvrmsos'
      if (address === 53) return 'cvrmsos'
      if (address === 54) return 'airmsos'
      if (address === 55) return 'birmsos'
      if (address === 56) return 'cirmsos'
      if (address === 57) return 'awattos'
      if (address === 58) return 'bwattos'
      if (address === 59) return 'cwattos'
      if (address === 60) return 'avaros'
      if (address === 61) return 'bvaros'
      if (address === 62) return 'cvaros'
      if (address === 63) return 'aphcal'
      if (address === 64) return 'bphcal'
      if (address === 65) return 'cphcal'
      if (address === 66) return 'wdiv'
      if (address === 67) return 'vardiv'
      if (address === 68) return 'vadiv'
      if (address === 69) return 'apcfnum'
      if (address === 70) return 'apcfden'
      if (address === 71) return 'varcfnum'
      if (address === 72) return 'varcfden'
      if (address === 126) return 'chksum'
      if (address === 127) return 'version'
      return 'revision'
    }

    if (address === 1) return 'waveform'
    if (address === 2) return 'active-energy'
    if (address === 3) return 'reset-active-energy'
    if (address === 4) return 'line-active-energy'
    if (address === 5) return 'apparent-energy'
    if (address === 6) return 'reset-apparent-energy'
    if (address === 7) return 'line-apparent-energy'
    if (address === 8) return 'line-reactive-energy'
    if (address === 9) return 'mode'
    if (address === 10) return 'interrupt-enable'
    if (address === 11) return 'interrupt-status'
    if (address === 12) return 'reset-interrupt-status'
    if (address === 13) return 'channel1-offset'
    if (address === 14) return 'channel2-offset'
    if (address === 15) return 'gain'
    if (address === 16) return 'phase-calibration'
    if (address === 17) return 'power-offset'
    if (address === 18) return 'power-gain'
    if (address === 19) return 'power-divider'
    if (address === 20) return 'cf-numerator'
    if (address === 21) return 'cf-denominator'
    if (address === 22) return 'current-rms'
    if (address === 23) return 'voltage-rms'
    if (address === 24) return 'current-rms-offset'
    if (address === 25) return 'voltage-rms-offset'
    if (address === 26) return 'apparent-gain'
    if (address === 27) return 'apparent-divider'
    if (address === 28) return 'line-cycles'
    if (address === 29) return 'zero-cross-timeout'
    if (address === 30) return 'sag-cycles'
    if (address === 31) return 'sag-level'
    if (address === 32) return 'current-peak-level'
    if (address === 33) return 'voltage-peak-level'
    if (address === 34) return 'current-peak'
    if (address === 35) return 'reset-current-peak'
    if (address === 36) return 'voltage-peak'
    if (address === 37) return 'reset-voltage-peak'
    if (address === 38) return 'temperature'
    if (address === 39) return 'period'
    if (address === 61) return 'test-mode'
    if (address === 62) return 'checksum'
    return 'revision'
  }

  async complete(
    tx: u64,
    address: u16,
    read: bool,
    inputRaw: u32,
    where: Provenance,
  ): Promise<void> {
    let raw = inputRaw
    const bits = this.bits(address)
    raw &= (1 << bits) - 1
    let signed =
      address <= 4 ||
      address === 8 ||
      address === 16 ||
      address === 17 ||
      address === 18 ||
      address === 24 ||
      address === 25 ||
      address === 26 ||
      address === 38
    if (this.config.model === 'ade7758')
      signed =
        (address >= 1 && address <= 15) ||
        address === 17 ||
        address === 18 ||
        (address >= 36 && address <= 65)
    const negative = signed && (raw & (1 << (bits - 1))) !== 0
    await this.out.emit(
      {
        kind: 'register',
        tx: tx,
        register: this.name(address),
        read: read,
        bits,
        raw,
        negative,
        magnitude: negative ? (1 << bits) - raw : raw,
      },
      where,
    )
    if (this.config.model === 'ade7753' && address === 9)
      await this.out.emit(
        {
          kind: 'mode',
          tx: tx,
          highPassDisabled: (raw & 1) !== 0,
          lowPassDisabled: (raw & 2) !== 0,
          cfDisabled: (raw & 4) !== 0,
          sagDisabled: (raw & 8) !== 0,
          suspended: (raw & 16) !== 0,
          temperatureStart: (raw & 32) !== 0,
          reset: (raw & 64) !== 0,
          lineCycle: (raw & 128) !== 0,
          channel1Disabled: (raw & 256) !== 0,
          channel2Disabled: (raw & 512) !== 0,
          swap: (raw & 1024) !== 0,
          sampleRate: ((raw >> 11) & 3) as u8,
          waveform: ((raw >> 13) & 3) as u8,
          positiveOnly: (raw & 32768) !== 0,
        },
        where,
      )
    if (this.config.model === 'ade7753' && address === 15)
      await this.out.emit(
        {
          kind: 'gain',
          tx: tx,
          channel1: (raw & 7) as u8,
          channel2: ((raw >> 5) & 7) as u8,
          inputRange: ((raw >> 3) & 3) as u8,
        },
        where,
      )

    if (this.config.model === 'ade7758' && address === 19)
      await this.out.emit(
        {
          kind: 'operational-mode',
          tx,
          highPassDisabled: (raw & 1) !== 0,
          lowPassDisabled: (raw & 2) !== 0,
          cfDisabled: (raw & 4) !== 0,
          disableMode: ((raw >> 3) & 7) as u8,
          reset: (raw & 64) !== 0,
        },
        where,
      )
    await this.out.emit({ kind: 'end', tx, reason: 'complete' }, where)
  }
  override async run(): Promise<void> {
    for (;;) {
      const received = await this.input.next()
      const event = received.event
      const where = received.at
      if (event.kind === 'begin') {
        if (event.partial || this.config.framed || this.stage === 2) {
          await this.abort(where, 'aborted')
          this.stage = event.partial ? 2 : 0
        }
      } else if (event.kind === 'end') {
        if (this.config.framed || event.reason === 'aborted')
          await this.abort(where, event.reason === 'aborted' ? 'aborted' : 'incomplete')
      } else if (event.kind === 'word') {
        if (this.stage === 2) continue
        if (this.stage === 0) {
          const opcode = event.mosi
          if (opcode === undefined) continue
          this.tx += 1
          this.address = opcode & (this.config.model === 'ade7758' ? 127 : 63)
          this.read = (opcode & 128) === 0
          const bits = this.bits(this.address)
          await this.out.emit({ kind: 'begin', tx: this.tx }, where)
          await this.out.emit(
            {
              kind: 'command',
              tx: this.tx,
              opcode,
              address: this.address,
              read: this.read,
              status: event.miso,
            },
            where,
          )
          if (bits === 0 || (this.config.model === 'ade7753' && (opcode & 64) !== 0)) {
            await this.out.emit({ kind: 'end', tx: this.tx, reason: 'unsupported' }, where)
            this.stage = 2
            continue
          }
          this.target = 1
          if (bits > 8) this.target = 2
          if (bits > 16) this.target = 3
          this.raw = 0
          this.count = 0
          this.registerAt = undefined
          this.stage = 1
          continue
        }
        const value = this.read ? event.miso : event.mosi
        if (value === undefined) {
          await this.abort(where, 'aborted')
          continue
        }
        this.registerAt = this.registerAt === undefined ? where : join(this.registerAt, where)
        await this.out.emit(
          {
            kind: 'data',
            tx: this.tx,
            offset: this.count,
            address: this.address,
            value,
            read: this.read,
          },
          where,
        )
        this.raw = (this.raw << 8) | value
        this.count += 1
        if (this.count === this.target) {
          const span = this.registerAt
          if (span === undefined) {
            await this.abort(where, 'aborted')
            continue
          }
          this.registerAt = undefined
          this.stage = 0
          await this.complete(this.tx, this.address, this.read, this.raw, span)
        }
      }
    }
  }
}
export const device = defineStreamDecoder({
  name: 'ade77xx',
  category: 'device',
  config: { defaults: { model: 'ade7758', framed: false } },
  input: spiEvents,
  requires: (config) => ({
    wordBits: 8,
    bitOrder: 'msb-first',
    mosi: true,
    miso: true,
    cs: oneOf(true, config.framed),
  }),
  output: events,
  props: (config) => ({ model: config.model }),
  locales: {
    'en-US': {
      name: 'ADE77XX',
      summary: 'ADE7753/7758 register widths, signed codes and MODE/GAIN controls',
      fields: { model: 'Model', framed: 'Restart commands on chip select' },
      events: {
        begin: 'Begin',
        command: 'Command',
        data: 'Data',
        register: 'Register',
        mode: 'Mode',
        gain: 'Gain',
        end: 'End',
        'operational-mode': 'Operational mode',
      },
      eventFields: {
        tx: 'Transaction',
        opcode: 'Opcode',
        address: 'Address',
        read: 'Read',
        status: 'Status',
        offset: 'Offset',
        value: 'Value',
        register: 'Register',
        bits: 'Bits',
        raw: 'Raw',
        negative: 'Negative',
        magnitude: 'Magnitude',
        highPassDisabled: 'High Pass Disabled',
        lowPassDisabled: 'Low Pass Disabled',
        cfDisabled: 'Cf Disabled',
        sagDisabled: 'Sag Disabled',
        suspended: 'Suspended',
        temperatureStart: 'Temperature Start',
        reset: 'Reset',
        lineCycle: 'Line Cycle',
        channel1Disabled: 'Channel1 Disabled',
        channel2Disabled: 'Channel2 Disabled',
        swap: 'Swap',
        sampleRate: 'Sample Rate',
        waveform: 'Waveform',
        positiveOnly: 'Positive Only',
        channel1: 'Channel1',
        channel2: 'Channel2',
        inputRange: 'Input Range',
        reason: 'Reason',
        disableMode: 'Channel disable mode',
      },
      values: { model: { ade7753: 'ADE7753', ade7758: 'ADE7758' } },
    },
    'zh-CN': {
      name: 'ADE77XX',
      summary: '解析ADE7753/7758寄存器宽度、有符号编码及模式和增益控制',
      fields: { model: '芯片型号', framed: '片选重新对齐命令' },
      events: {
        begin: '开始',
        command: '命令',
        data: '数据通道',
        register: '寄存器',
        mode: '模式',
        gain: '增益',
        end: '结束',
        'operational-mode': '运行模式',
      },
      eventFields: {
        tx: '事务号',
        opcode: '操作码',
        address: '地址',
        read: '读取',
        status: '状态',
        offset: '偏移',
        value: '数值',
        register: '寄存器',
        bits: '位数',
        raw: '原始值',
        negative: '负数',
        magnitude: '绝对值',
        highPassDisabled: '高通滤波禁用',
        lowPassDisabled: '低通滤波禁用',
        cfDisabled: 'CF输出禁用',
        sagDisabled: '电压跌落检测禁用',
        suspended: '已挂起',
        temperatureStart: '启动温度转换',
        reset: '复位',
        lineCycle: '线路周期',
        channel1Disabled: '通道1禁用',
        channel2Disabled: '通道2禁用',
        swap: '交换通道',
        sampleRate: '采样速率',
        waveform: '波形选择',
        positiveOnly: '仅累积正值',
        channel1: '通道1',
        channel2: '通道2',
        inputRange: '输入量程',
        reason: '结束原因',
        disableMode: '通道禁用模式',
      },
      values: { model: { ade7753: 'ADE7753', ade7758: 'ADE7758' } },
    },
  },
  decoder: Device,
})
