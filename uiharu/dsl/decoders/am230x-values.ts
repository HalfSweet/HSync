// SPDX-License-Identifier: Apache-2.0
/** Engineering values from Aosong AM2302 and classic integer-only DHT11 data frames. */
import type { bool, u32 } from 'uiharu'
import { StreamDecoder, defineStreamDecoder, eventSchema } from 'uiharu'
import { am230xEvents, type Am230xEvent } from '../contracts/am230x'
export type Model = 'am2302' | 'dht11'
export interface Config {
  model: Model
}
export type Reason = 'checksum' | 'decimal' | 'range' | 'input-error'
export type Event =
  | {
      kind: 'reading'
      humidityMilliPercent: u32
      temperatureMilliCelsius: u32
      negative: bool
      valid: bool
    }
  | { kind: 'error'; reason: Reason }
export const events = eventSchema<Event>('am230x-values.events', 1).roles({
  reading: 'data',
  error: 'error',
})
class Am230xValues extends StreamDecoder<Config, Am230xEvent, Event> {
  override async run(): Promise<void> {
    for (;;) {
      const r = await this.input.next()
      const e = r.event
      if (e.kind === 'error') {
        await this.out.emit({ kind: 'error', reason: 'input-error' }, r.at)
        continue
      }
      if (e.kind === 'reading') {
        if (!e.checksumOk) {
          await this.out.emit({ kind: 'error', reason: 'checksum' }, r.at)
          continue
        }
        let humidity: u32 = e.humidityRaw * 100
        let temperature: u32 = (e.temperatureRaw & 0x7fff) * 100
        let negative = (e.temperatureRaw & 0x8000) !== 0
        if (this.config.model === 'dht11') {
          if (e.bytes[1] !== 0 || e.bytes[3] !== 0) {
            await this.out.emit({ kind: 'error', reason: 'decimal' }, r.at)
            continue
          }
          humidity = e.bytes[0] * 1000
          temperature = e.bytes[2] * 1000
          negative = false
        }
        const valid = humidity <= 100000 && (negative ? temperature <= 40000 : temperature <= 80000)
        await this.out.emit(
          {
            kind: 'reading',
            humidityMilliPercent: humidity,
            temperatureMilliCelsius: temperature,
            negative,
            valid,
          },
          r.at,
        )
        if (!valid) await this.out.emit({ kind: 'error', reason: 'range' }, r.at)
      }
    }
  }
}
export const am230xValues = defineStreamDecoder({
  name: 'am230x-values',
  category: 'device',
  config: { defaults: { model: 'am2302' } },
  input: am230xEvents,
  output: events,
  decoder: Am230xValues,
  locales: {
    'en-US': {
      name: 'AM230x engineering values',
      summary: 'AM2302/DHT22 and classic DHT11 humidity and temperature',
      fields: { model: 'Sensor model' },
      values: { model: { am2302: 'AM2302 / DHT22', dht11: 'Classic DHT11' } },
      events: { reading: 'Sensor reading', error: 'Invalid reading' },
      eventFields: {
        humidityMilliPercent: 'Humidity (milli-percent)',
        temperatureMilliCelsius: 'Temperature (milli-Celsius)',
        negative: 'Negative temperature',
        valid: 'In range',
        reason: 'Error reason',
      },
    },
    'zh-CN': {
      name: 'AM230x工程值',
      summary: 'AM2302/DHT22和经典DHT11温湿度',
      fields: { model: '传感器型号' },
      values: { model: { am2302: 'AM2302 / DHT22', dht11: '经典DHT11' } },
      events: { reading: '传感器读数', error: '无效读数' },
      eventFields: {
        humidityMilliPercent: '湿度（千分之一百分比）',
        temperatureMilliCelsius: '温度（毫摄氏度）',
        negative: '负温度',
        valid: '范围有效',
        reason: '错误原因',
      },
    },
  },
})
