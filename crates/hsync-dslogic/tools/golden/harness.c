/*
 * Golden data for hsync-dslogic: runs DSView's own dsl_fpga_arm() (libsigrok4DSL,
 * hardware/DSL/dsl.c) against a fake USB handle and prints the DSL_setting and
 * DSL_setting_ext32 bytes it would send. See README.md.
 *
 * Input, one case per line (blank lines and lines starting with '#' are copied):
 *   pid=0x0020 speed=H chmode=0 rate=1000000 limit=1048576 pos=50 en=0xffff
 *   [filter=1] [clk=1] [edge=1] [rle=1] [instant=1]
 *   [trig=<32 chars X01RFC, channel 0 first>]
 *   [tmode=adv|serial stages=N s<i>=<32 chars>/<32 chars>/<logic>/<inv0>/<inv1>/<count0>]
 * Output: the input line, " => ", the setting as hex and the ext32 setting as hex (or "-").
 */

#include "libsigrok-internal.h"
#include "hardware/DSL/command.h"
#include "hardware/DSL/dsl.h"
#include "log.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

extern struct ds_trigger *trigger;

/* What libsigrok4DSL's other files would provide. */
char DS_RES_PATH[500];
xlog_writer *sr_log;
int xlog_err(xlog_writer *wr, const char *format, ...) { (void)wr; (void)format; return 0; }
int xlog_warn(xlog_writer *wr, const char *format, ...) { (void)wr; (void)format; return 0; }
int xlog_info(xlog_writer *wr, const char *format, ...) { (void)wr; (void)format; return 0; }
int xlog_dbg(xlog_writer *wr, const char *format, ...) { (void)wr; (void)format; return 0; }
int xlog_detail(xlog_writer *wr, const char *format, ...) { (void)wr; (void)format; return 0; }

static unsigned char bulk[2][512];
static int bulk_len[2];
static int bulk_count;

int libusb_bulk_transfer(libusb_device_handle *dev_handle, unsigned char endpoint,
                         unsigned char *data, int length, int *actual_length,
                         unsigned int timeout)
{
    (void)dev_handle;
    (void)endpoint;
    (void)timeout;
    if (bulk_count < 2 && length <= 512) {
        memcpy(bulk[bulk_count], data, length);
        bulk_len[bulk_count] = length;
    }
    bulk_count++;
    *actual_length = length;
    return 0;
}

int libusb_control_transfer(libusb_device_handle *dev_handle, uint8_t request_type,
                            uint8_t bRequest, uint16_t wValue, uint16_t wIndex,
                            unsigned char *data, uint16_t wLength, unsigned int timeout)
{
    (void)dev_handle;
    (void)request_type;
    (void)wValue;
    (void)wIndex;
    (void)timeout;
    /* HW_STATUS reads: SYS_CLR and GPIF_DONE set, so dsl_fpga_arm() proceeds. */
    if (bRequest == CMD_CTL_RD)
        memset(data, bmSYS_CLR | bmGPIF_DONE, wLength);
    return wLength;
}

static const struct DSL_profile *find_profile(int pid, int speed)
{
    for (int i = 0; supported_DSLogic[i].vid; i++)
        if (supported_DSLogic[i].pid == pid && (int)supported_DSLogic[i].usb_speed == speed)
            return &supported_DSLogic[i];
    return NULL;
}

static void hex(const unsigned char *data, int length)
{
    for (int i = 0; i < length; i++)
        printf("%02x", data[i]);
}

static int run(char *line)
{
    char *saveptr = NULL;
    int pid = -1, speed = LIBUSB_SPEED_HIGH, chmode = 0;
    unsigned long long rate = 0, limit = 0, en = 0;
    int pos = 0, filter = 0, clk = 0, edge = 0, rle = 0, instant = 0;
    int tmode = SIMPLE_TRIGGER, stages = 0;
    const char *simple = NULL;

    ds_trigger_init();

    for (char *tok = strtok_r(line, " \t\n", &saveptr); tok; tok = strtok_r(NULL, " \t\n", &saveptr)) {
        char *eq = strchr(tok, '=');
        if (!eq)
            return -1;
        *eq = '\0';
        const char *key = tok, *value = eq + 1;
        if (!strcmp(key, "pid"))
            pid = (int)strtol(value, NULL, 0);
        else if (!strcmp(key, "speed"))
            speed = value[0] == 'S' ? LIBUSB_SPEED_SUPER : LIBUSB_SPEED_HIGH;
        else if (!strcmp(key, "chmode"))
            chmode = atoi(value);
        else if (!strcmp(key, "rate"))
            rate = strtoull(value, NULL, 0);
        else if (!strcmp(key, "limit"))
            limit = strtoull(value, NULL, 0);
        else if (!strcmp(key, "pos"))
            pos = atoi(value);
        else if (!strcmp(key, "en"))
            en = strtoull(value, NULL, 0);
        else if (!strcmp(key, "filter"))
            filter = atoi(value);
        else if (!strcmp(key, "clk"))
            clk = atoi(value);
        else if (!strcmp(key, "edge"))
            edge = atoi(value);
        else if (!strcmp(key, "rle"))
            rle = atoi(value);
        else if (!strcmp(key, "instant"))
            instant = atoi(value);
        else if (!strcmp(key, "trig")) {
            if (strlen(value) != MaxTriggerProbes)
                return -1;
            simple = value;
        } else if (!strcmp(key, "tmode"))
            tmode = !strcmp(value, "serial") ? SERIAL_TRIGGER : ADV_TRIGGER;
        else if (!strcmp(key, "stages"))
            stages = atoi(value);
        else if (key[0] == 's' && key[1] >= '0' && key[1] <= '9') {
            int stage = atoi(key + 1);
            char t0[MaxTriggerProbes + 1], t1[MaxTriggerProbes + 1];
            unsigned logic, inv0, inv1, count0;
            if (stage >= TriggerStages ||
                sscanf(value, "%32[^/]/%32[^/]/%u/%u/%u/%u", t0, t1, &logic, &inv0, &inv1, &count0) != 6)
                return -1;
            for (int ch = 0; ch < MaxTriggerProbes; ch++) {
                trigger->trigger0[stage][ch] = t0[ch];
                trigger->trigger1[stage][ch] = t1[ch];
            }
            trigger->trigger_logic[stage] = (unsigned char)logic;
            trigger->trigger0_inv[stage] = (unsigned char)inv0;
            trigger->trigger1_inv[stage] = (unsigned char)inv1;
            trigger->trigger0_count[stage] = count0;
        } else
            return -1;
    }

    const struct DSL_profile *profile = find_profile(pid, speed);
    if (!profile)
        return -1;

    /* As TriggerDock::try_commit_trigger() and LogicSignal::commit_trig() leave it. */
    if (!instant) {
        ds_trigger_set_pos(pos);
        ds_trigger_set_mode(tmode);
        if (tmode != SIMPLE_TRIGGER) {
            ds_trigger_set_en(1);
            ds_trigger_set_stage(stages);
        } else if (simple) {
            for (int ch = 0; ch < MaxTriggerProbes; ch++) {
                if (simple[ch] != 'X') {
                    ds_trigger_set_en(1);
                    ds_trigger_probe_set(ch, simple[ch], 'X');
                }
            }
        }
    }

    static struct DSL_context devc;
    static struct sr_usb_dev_inst usb;
    static struct sr_dev_inst sdi;
    static struct sr_channel channels[32];
    memset(&devc, 0, sizeof devc);
    memset(&usb, 0, sizeof usb);
    memset(&sdi, 0, sizeof sdi);
    memset(channels, 0, sizeof channels);

    devc.profile = profile;
    devc.ch_mode = chmode;
    devc.stream = channel_modes[chmode].stream;
    devc.cur_samplerate = rate;
    devc.limit_samples = limit;
    devc.clock_type = clk;
    devc.clock_edge = edge;
    devc.rle_mode = rle;
    devc.instant = instant;
    devc.filter = filter;
    devc.test_mode = SR_TEST_NONE;
    devc.cb_data = &sdi;

    usb.devhdl = (libusb_device_handle *)&usb;
    sdi.priv = &devc;
    sdi.conn = &usb;
    sdi.mode = LOGIC;
    sdi.channels = NULL;
    for (int ch = 0; ch < channel_modes[chmode].num; ch++) {
        channels[ch].index = ch;
        channels[ch].enabled = (en >> ch) & 1;
        sdi.channels = g_slist_append(sdi.channels, &channels[ch]);
    }

    /* As dev_acquisition_start() sets them before arming. */
    devc.actual_samples = (devc.limit_samples + SAMPLES_ALIGN) & ~SAMPLES_ALIGN;
    devc.actual_bytes = devc.actual_samples / DSLOGIC_ATOMIC_SAMPLES * dsl_en_ch_num(&sdi) * DSLOGIC_ATOMIC_SIZE;

    bulk_count = 0;
    int ret = dsl_fpga_arm(&sdi);
    g_slist_free(sdi.channels);
    if (ret != SR_OK || bulk_count < 1 || bulk_len[0] != (int)sizeof(struct DSL_setting))
        return -1;

    printf(" => ");
    hex(bulk[0], bulk_len[0]);
    printf(" ");
    if (bulk_count > 1)
        hex(bulk[1], bulk_len[1]);
    else
        printf("-");
    printf("\n");
    return 0;
}

int main(void)
{
    char line[4096], copy[4096];
    while (fgets(line, sizeof line, stdin)) {
        line[strcspn(line, "\n")] = '\0';
        if (line[0] == '\0' || line[0] == '#') {
            printf("%s\n", line);
            continue;
        }
        strcpy(copy, line);
        printf("%s", line);
        if (run(copy) != 0) {
            fprintf(stderr, "bad case: %s\n", line);
            return 1;
        }
    }
    return 0;
}
