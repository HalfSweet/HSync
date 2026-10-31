// SPDX-License-Identifier: Apache-2.0 OR GPL-3.0-or-later
/* Executes the official Sipeed control implementation against a register model. */
#include <assert.h>
#include <inttypes.h>
#include <stdio.h>
#include <string.h>

#include "slogic.h"

struct device {
    uint32_t command;
    unsigned payload_len, busy, polls;
    uint16_t clocks[4];
    uint8_t payload[16];
};

static uint32_t word(const uint8_t *bytes)
{
    return (uint32_t)bytes[0] | ((uint32_t)bytes[1] << 8) |
           ((uint32_t)bytes[2] << 16) | ((uint32_t)bytes[3] << 24);
}

static void put(uint8_t *bytes, uint32_t value)
{
    for (unsigned i = 0; i < 4; i++)
        bytes[i] = (uint8_t)(value >> (i * 8));
}

static void trace(char direction, uint8_t req, uint16_t addr, uint16_t index,
                  const uint8_t *data, uint16_t len)
{
    printf("%c %02x %04x %04x ", direction, req, addr, index);
    for (unsigned i = 0; i < len; i++)
        printf("%02x", data[i]);
    putchar('\n');
}

static int write_control(void *ctx, uint8_t req, uint16_t addr, uint16_t index,
                         const uint8_t *data, uint16_t len, unsigned timeout)
{
    struct device *d = ctx;
    assert((req == 1 || req == 0xb1) && index == 0 && len == 4 && timeout == 500);
    trace('W', req, addr, index, data, len);
    if (req == 0xb1) {
        assert(addr == 0);
        return len;
    }
    if (addr == 0xc) {
        d->command = word(data);
        d->polls = 0;
        memset(d->payload, 0, sizeof(d->payload));
        for (unsigned i = 8; i < d->payload_len; i++)
            d->payload[i] = (uint8_t)(0xa0 + i);
        if (d->command == 2)
            put(d->payload, (uint32_t)d->clocks[0] << 16);
    } else if (addr >= 0x10 && addr <= 0x1c) {
        memcpy(d->payload + addr - 0x10, data, len);
        if (addr == 0x10 && d->command == 2) {
            unsigned slot = word(data) & 0xffff;
            uint16_t mhz = slot < 4 ? d->clocks[slot] : 0;
            put(d->payload, slot | ((uint32_t)mhz << 16));
        }
    }
    return len;
}

static int read_control(void *ctx, uint8_t req, uint16_t addr, uint16_t index,
                        uint8_t *data, uint16_t len, unsigned timeout)
{
    struct device *d = ctx;
    assert(req == 0 && index == 0 && len == 4 && timeout == 500);
    if (addr == 0xc) {
        unsigned size = d->payload_len ? d->payload_len : (d->command == 2 ? 8 : 4);
        uint32_t header = ++d->polls <= d->busy ? 0 : (1u << 16) | (size << 9);
        put(data, header);
    } else {
        assert(addr >= 0x10 && addr <= 0x1c);
        memcpy(data, d->payload + addr - 0x10, len);
    }
    trace('R', req, addr, index, data, len);
    return len;
}

static void capture(uint16_t pid, const char *name, int channels, uint64_t rate,
                    double threshold, unsigned payload_len, unsigned busy, uint16_t first,
                    uint16_t second)
{
    struct device d = { .payload_len = payload_len, .busy = busy, .clocks = {first, second} };
    slogic_transport t = {.ctx = &d, .control_write = write_control, .control_read = read_control};
    slogic_config c = {.channel_count = channels, .samplerate_hz = rate, .threshold_v = threshold};
    const slogic_model *m = slogic_model_for_pid(pid);
    assert(m && slogic_max_rate(m, channels) >= rate);
    printf("CASE %s %d %" PRIu64 " %.3f\n", name, channels, rate, threshold);
    assert(slogic_reset(m, &t) == SLOGIC_OK);
    assert(slogic_configure(m, &t, &c) == SLOGIC_OK);
    assert(slogic_run(m, &t, &c) == SLOGIC_OK);
    assert(slogic_stop(m, &t) == SLOGIC_OK);
    puts("END");
}

int main(void)
{
    puts("# sipeed/libsigrok bf6b346e597d112f48102fe6a7b2318c6d7b49e2");
    capture(SLOGIC_PID_16U3, "four_fastest", 4, 800000000, 0.0, 0, 0, 800, 0);
    capture(SLOGIC_PID_16U3, "eight_fastest", 8, 400000000, 1.8, 0, 0, 800, 0);
    capture(SLOGIC_PID_16U3, "sixteen_fastest", 16, 200000000, 3.3, 0, 0, 800, 0);
    capture(SLOGIC_PID_16U3, "slowest_max_threshold", 4, 5000000, 6.0, 0, 0, 800, 0);
    capture(SLOGIC_PID_16U3, "clock_walk", 16, 200000000, 1.0, 0, 0, 1400, 800);
    capture(SLOGIC_PID_16U3, "clock_fallback", 16, 200000000, 2.5, 0, 0, 1400, 0);
    capture(SLOGIC_PID_16U3, "busy_extended_payload", 8, 100000000, 0.3, 16, 2, 800, 0);
    /* SLogic32U3 reports a 1400 MHz clock in slot 0 and 800 MHz in slot 1. */
    capture(SLOGIC_PID_32U3, "u32_four_fastest", 4, 1400000000, 1.7, 0, 0, 1400, 800);
    capture(SLOGIC_PID_32U3, "u32_eight_fastest", 8, 800000000, 1.2, 0, 0, 1400, 800);
    capture(SLOGIC_PID_32U3, "u32_sixteen_fastest", 16, 400000000, 0.9, 0, 0, 1400, 800);
    capture(SLOGIC_PID_32U3, "u32_thirty_two_fastest", 32, 200000000, 3.3, 0, 0, 1400, 800);
    capture(SLOGIC_PID_32U3, "u32_thirty_two_5g", 32, 100000000, 2.5, 0, 0, 1400, 800);
    capture(SLOGIC_PID_32U3, "u32_slowest", 32, 5000000, 0.6, 0, 0, 1400, 800);
    const uint16_t pids[] = {SLOGIC_PID_COMBO8, SLOGIC_PID_8U2};
    for (unsigned p = 0; p < sizeof(pids) / sizeof(pids[0]); p++) {
        const slogic_model *m = slogic_model_for_pid(pids[p]);
        assert(m);
        for (int channels = 2; channels <= 8; channels *= 2) {
            for (size_t i = 0; i < m->rate_count; i++) {
                uint64_t rate = m->rates[i];
                if (rate > slogic_max_rate(m, channels))
                    continue;
                char name[80];
                snprintf(name, sizeof(name), "combo8_%04x_%d_%" PRIu64, pids[p], channels, rate);
                capture(pids[p], name, channels, rate, 0.0, 0, 0, 0, 0);
            }
        }
    }
    return 0;
}
