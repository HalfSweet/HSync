/*
 * Golden data for hsync-pxlogic: runs PXView's own pxlogic driver (src/hardware/pxlogic/pxlogic.c
 * and usb_ctrl.c of PXLogic's libsigrok, built unchanged) against a fake USB handle, and prints
 * what it does on the wire. See README.md.
 *
 * Input, one case per line (blank lines and lines starting with '#' are copied):
 *   open profile=<index into supported_PX> lm=<logic mode> fw=<firmware version, hex>
 *   start profile=<index> mode=buffer|stream chmode=<PX_CHANNEL_ID> rate=<Hz> limit=<samples>
 *         en=<channel mask> vth=<volts> filter=0|1 edge=0|1 ext=<0..5> pos=<percent> loop=0|1
 *         trig=<one of X 0 1 R F C per channel, channel 0 first>
 * Output: the input line, " => " and the operations, separated by " | ":
 *   W <address> <data>        a register write (bulk OUT 0x01, answered on 0x81)
 *   R <address>               a register read
 *   C <endpoint>              a cleared halt
 *   D <endpoint> <bytes> <FNV-1a 64 of the bytes>   a bulk OUT transfer of data
 *   S <endpoint> <bytes>      a bulk IN transfer submitted
 *   I <request> <bytes>       a vendor control IN request
 * then, for open cases, "ret <result> lm <logic mode of the profile in use>".
 *
 * Open cases read firmware files the harness writes itself: byte i of SCI_LOGIC.bin is
 * (i * 31 + 1) & 0xff over 25804 bytes, of hspi_ddr_RST.bin (i * 31 + 2) & 0xff and of
 * hspi_ddr.bin (i * 31 + 3) & 0xff over 12288 bytes each. The bitstreams fill whole 4 KiB
 * pages: firmware_config() sends the rest of an uninitialised buffer after them.
 */

#include "pxlogic.c"

#include <stdarg.h>

/* ---- What the fake device holds -------------------------------------------------------- */

static uint32_t reg_logic_mode;
static uint32_t reg_firmware;
static uint32_t regs[0x3000];
static uint32_t pending[4];
static char out[1 << 16];
static size_t out_len;
static struct sr_trigger *case_trigger;
static char firmware_dir[256];

static void emit(const char *format, ...)
{
    va_list args;
    if (out_len)
        out_len += snprintf(out + out_len, sizeof(out) - out_len, " | ");
    va_start(args, format);
    out_len += vsnprintf(out + out_len, sizeof(out) - out_len, format, args);
    va_end(args);
}

static uint64_t fnv1a(const unsigned char *data, int length)
{
    uint64_t hash = 0xcbf29ce484222325ULL;
    for (int i = 0; i < length; i++) {
        hash ^= data[i];
        hash *= 0x100000001b3ULL;
    }
    return hash;
}

static uint32_t word(const unsigned char *data, int at)
{
    return data[at] | data[at + 1] << 8 | data[at + 2] << 16 | (uint32_t)data[at + 3] << 24;
}

/* ---- libusb ------------------------------------------------------------------------------ */

int libusb_bulk_transfer(libusb_device_handle *dev_handle, unsigned char endpoint,
                         unsigned char *data, int length, int *actual_length,
                         unsigned int timeout)
{
    (void)dev_handle;
    (void)timeout;
    if (endpoint == 0x01 && length == 16) {
        uint32_t command = word(data, 0), address = word(data, 8), value = word(data, 12);
        pending[0] = command;
        pending[1] = 8;
        pending[2] = address;
        if (command == 0xfefe0000) {
            emit("W %04x %08x", address, value);
            if (address < sizeof(regs) / sizeof(regs[0]))
                regs[address] = value;
            pending[3] = 0xfefefefe;
        } else {
            emit("R %04x", address);
            if (address == 8192 + 22 * 4)
                pending[3] = reg_logic_mode;
            else if (address == 8192 + 13 * 4)
                pending[3] = reg_firmware;
            else
                pending[3] = address < sizeof(regs) / sizeof(regs[0]) ? regs[address] : 0;
        }
    } else if (endpoint == 0x81 && length == 16) {
        memcpy(data, pending, 16);
    } else if (!(endpoint & 0x80)) {
        emit("D %02x %d %016llx", endpoint, length,
             (unsigned long long)fnv1a(data, length));
    } else {
        emit("? %02x %d", endpoint, length);
    }
    if (actual_length)
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
    emit("I %02x %u", bRequest, wLength);
    memset(data, 0, wLength);
    return wLength;
}

int libusb_clear_halt(libusb_device_handle *dev_handle, unsigned char endpoint)
{
    (void)dev_handle;
    emit("C %02x", endpoint);
    return 0;
}

struct libusb_transfer *libusb_alloc_transfer(int iso_packets)
{
    (void)iso_packets;
    return calloc(1, sizeof(struct libusb_transfer));
}

int libusb_submit_transfer(struct libusb_transfer *transfer)
{
    emit("S %02x %d", transfer->endpoint, transfer->length);
    return 0;
}

void libusb_free_transfer(struct libusb_transfer *transfer) { free(transfer); }
int libusb_cancel_transfer(struct libusb_transfer *transfer) { (void)transfer; return 0; }
int libusb_claim_interface(libusb_device_handle *h, int i) { (void)h; (void)i; return 0; }
int libusb_release_interface(libusb_device_handle *h, int i) { (void)h; (void)i; return 0; }
void libusb_close(libusb_device_handle *h) { (void)h; }
const char *libusb_error_name(int code) { (void)code; return "error"; }
void libusb_free_config_descriptor(struct libusb_config_descriptor *c) { (void)c; }
void libusb_free_device_list(libusb_device **l, int u) { (void)l; (void)u; }
int libusb_get_active_config_descriptor(libusb_device *d, struct libusb_config_descriptor **c)
{
    (void)d;
    (void)c;
    return LIBUSB_ERROR_NOT_FOUND;
}
uint8_t libusb_get_bus_number(libusb_device *d) { (void)d; return 1; }
uint8_t libusb_get_device_address(libusb_device *d) { (void)d; return 2; }
int libusb_get_device_descriptor(libusb_device *d, struct libusb_device_descriptor *x)
{
    (void)d;
    (void)x;
    return LIBUSB_ERROR_NOT_SUPPORTED;
}
ssize_t libusb_get_device_list(libusb_context *c, libusb_device ***l)
{
    (void)c;
    *l = NULL;
    return 0;
}
int libusb_get_device_speed(libusb_device *d) { (void)d; return LIBUSB_SPEED_SUPER; }
int libusb_get_string_descriptor_ascii(libusb_device_handle *h, uint8_t i, unsigned char *s,
                                       int n)
{
    (void)h;
    (void)i;
    (void)s;
    (void)n;
    return LIBUSB_ERROR_NOT_SUPPORTED;
}
int libusb_handle_events_timeout_completed(libusb_context *c, struct timeval *t, int *d)
{
    (void)c;
    (void)t;
    (void)d;
    return 0;
}
int libusb_open(libusb_device *d, libusb_device_handle **h)
{
    (void)d;
    *h = (libusb_device_handle *)0x1;
    return 0;
}
int libusb_set_auto_detach_kernel_driver(libusb_device_handle *h, int e)
{
    (void)h;
    (void)e;
    return 0;
}

/* ---- libsigrok ---------------------------------------------------------------------------- */

int sr_log(int loglevel, const char *format, ...) { (void)loglevel; (void)format; return 0; }

struct sr_channel *sr_channel_new(struct sr_dev_inst *sdi, int index, int type,
                                  gboolean enabled, const char *name)
{
    struct sr_channel *ch = g_malloc0(sizeof(struct sr_channel));
    ch->sdi = sdi;
    ch->index = index;
    ch->type = type;
    ch->enabled = enabled;
    ch->name = g_strdup(name);
    sdi->channels = g_slist_append(sdi->channels, ch);
    return ch;
}

void sr_channel_free(struct sr_channel *ch)
{
    g_free(ch->name);
    g_free(ch);
}

int sr_resource_open(struct sr_context *ctx, struct sr_resource *res, int type, const char *name)
{
    (void)ctx;
    (void)type;
    char path[512];
    snprintf(path, sizeof(path), "%s/%s", firmware_dir, name);
    FILE *file = fopen(path, "rb");
    if (!file)
        return SR_ERR;
    fseek(file, 0, SEEK_END);
    res->size = (uint64_t)ftell(file);
    fseek(file, 0, SEEK_SET);
    res->handle = file;
    return SR_OK;
}

ssize_t sr_resource_read(struct sr_context *ctx, const struct sr_resource *res, void *buf,
                         size_t count)
{
    (void)ctx;
    return (ssize_t)fread(buf, 1, count, res->handle);
}

int sr_resource_close(struct sr_context *ctx, struct sr_resource *res)
{
    (void)ctx;
    fclose(res->handle);
    return SR_OK;
}

struct sr_trigger *sr_session_trigger_get(struct sr_session *session)
{
    (void)session;
    return case_trigger;
}

int sr_session_send(const struct sr_dev_inst *sdi, const struct sr_datafeed_packet *packet)
{
    (void)sdi;
    (void)packet;
    return SR_OK;
}

int std_str_idx(GVariant *data, const char *a[], unsigned int n)
{
    const char *s = g_variant_get_string(data, NULL);
    for (unsigned int i = 0; i < n; i++)
        if (a[i] && !strcmp(s, a[i]))
            return (int)i;
    return -1;
}

int std_u64_idx(GVariant *data, const uint64_t a[], unsigned int n)
{
    uint64_t value = g_variant_get_uint64(data);
    for (unsigned int i = 0; i < n; i++)
        if (a[i] == value)
            return (int)i;
    return -1;
}

/* The libsigrok functions the cases never reach are in unreached.c. */

/* ---- Cases -------------------------------------------------------------------------------- */

static const char *value(const char *line, const char *key)
{
    static char found[256];
    size_t length = strlen(key);
    const char *at = line;
    while ((at = strstr(at, key)) != NULL) {
        if ((at == line || at[-1] == ' ') && at[length] == '=') {
            const char *start = at + length + 1;
            size_t n = strcspn(start, " \n");
            memcpy(found, start, n);
            found[n] = 0;
            return found;
        }
        at += length;
    }
    return "";
}

static void write_pattern(const char *name, int seed, int length)
{
    char path[512];
    snprintf(path, sizeof(path), "%s/%s", firmware_dir, name);
    FILE *file = fopen(path, "wb");
    for (int i = 0; i < length; i++)
        fputc((i * 31 + seed) & 0xff, file);
    fclose(file);
}

static struct sr_dev_inst *new_instance(const struct PX_profile *prof)
{
    struct sr_dev_inst *sdi = g_malloc0(sizeof(struct sr_dev_inst));
    struct sr_usb_dev_inst *usb = g_malloc0(sizeof(struct sr_usb_dev_inst));
    struct PX_context *devc = pxlogic_dev_new(prof);
    devc->usb_speed = prof->usb_speed;
    devc->usb_dev = (struct libusb_device *)0x1;
    usb->address = 2;
    sdi->conn = usb;
    sdi->priv = devc;
    sdi->driver = di;
    sdi->status = SR_ST_INACTIVE;
    setup_probes(sdi, channel_modes[devc->ch_mode].num);
    return sdi;
}

static void run_open(const char *line)
{
    const struct PX_profile *prof = &supported_PX[atoi(value(line, "profile"))];
    struct sr_dev_inst *sdi = new_instance(prof);
    static struct drv_context drvc;
    gboolean fpga_done = 0;
    reg_logic_mode = (uint32_t)strtoul(value(line, "lm"), NULL, 0);
    reg_firmware = (uint32_t)strtoul(value(line, "fw"), NULL, 16);
    pxlogic_driver_info.context = &drvc;
    int ret = hw_usb_open(di, sdi, &fpga_done);
    struct PX_context *devc = sdi->priv;
    emit("ret %d lm %u", ret, devc->profile->logic_mode);
}

static void set_string(struct sr_dev_inst *sdi, uint32_t key, const char *text)
{
    config_set(key, g_variant_new_string(text), sdi, NULL);
}

static void run_start(const char *line)
{
    const struct PX_profile *prof = &supported_PX[atoi(value(line, "profile"))];
    struct sr_dev_inst *sdi = new_instance(prof);
    struct PX_context *devc = sdi->priv;
    ((struct sr_usb_dev_inst *)sdi->conn)->devhdl = (libusb_device_handle *)0x1;
    sdi->status = SR_ST_ACTIVE;

    set_string(sdi, SR_CONF_OPERATION_MODE,
               strcmp(value(line, "mode"), "stream") ? "Buffer Mode" : "Stream Mode");
    set_string(sdi, SR_CONF_CHANNEL_MODE, channel_mode_str[atoi(value(line, "chmode"))]);
    config_set(SR_CONF_SAMPLERATE, g_variant_new_uint64(strtoull(value(line, "rate"), NULL, 0)),
               sdi, NULL);
    config_set(SR_CONF_LIMIT_SAMPLES,
               g_variant_new_uint64(strtoull(value(line, "limit"), NULL, 0)), sdi, NULL);
    config_set(SR_CONF_VTH, g_variant_new_double(atof(value(line, "vth"))), sdi, NULL);
    set_string(sdi, SR_CONF_FILTER, filter_modes[atoi(value(line, "filter"))]);
    set_string(sdi, SR_CONF_CLOCK_EDGE, signal_edges[atoi(value(line, "edge"))]);
    set_string(sdi, SR_CONF_EX_TRIGGER_MATCH, extern_trigger_match_strs[atoi(value(line, "ext"))]);
    config_set(SR_CONF_CAPTURE_RATIO, g_variant_new_uint64(strtoull(value(line, "pos"), NULL, 0)),
               sdi, NULL);
    config_set(SR_CONF_LOOP_MODE, g_variant_new_boolean(atoi(value(line, "loop"))), sdi, NULL);

    unsigned long enabled = strtoul(value(line, "en"), NULL, 0);
    for (GSList *l = sdi->channels; l; l = l->next) {
        struct sr_channel *ch = l->data;
        ch->enabled = (enabled >> ch->index) & 1;
    }

    const char *trig = value(line, "trig");
    struct sr_trigger *trigger = g_malloc0(sizeof(struct sr_trigger));
    struct sr_trigger_stage *stage = g_malloc0(sizeof(struct sr_trigger_stage));
    for (GSList *l = sdi->channels; l; l = l->next) {
        struct sr_channel *ch = l->data;
        if (ch->index >= (int)strlen(trig))
            break;
        int match = 0;
        switch (trig[ch->index]) {
        case '0': match = SR_TRIGGER_ZERO; break;
        case '1': match = SR_TRIGGER_ONE; break;
        case 'R': match = SR_TRIGGER_RISING; break;
        case 'F': match = SR_TRIGGER_FALLING; break;
        case 'C': match = SR_TRIGGER_EDGE; break;
        default: continue;
        }
        struct sr_trigger_match *m = g_malloc0(sizeof(struct sr_trigger_match));
        m->channel = ch;
        m->match = match;
        stage->matches = g_slist_append(stage->matches, m);
    }
    trigger->stages = g_slist_append(trigger->stages, stage);
    case_trigger = trigger;

    devc->cb_data = sdi;
    start_transfers(sdi);
}

int main(void)
{
    char line[1024];
    snprintf(firmware_dir, sizeof(firmware_dir), "%s/hsync-pxlogic-golden-fw",
             getenv("TMPDIR") ? getenv("TMPDIR") : "/tmp");
    mkdir(firmware_dir, 0755);
    write_pattern("SCI_LOGIC.bin", 1, 25804);
    write_pattern("hspi_ddr_RST.bin", 2, 12288);
    write_pattern("hspi_ddr.bin", 3, 12288);

    while (fgets(line, sizeof(line), stdin)) {
        line[strcspn(line, "\n")] = 0;
        if (line[0] == 0 || line[0] == '#') {
            printf("%s\n", line);
            continue;
        }
        out_len = 0;
        out[0] = 0;
        memset(regs, 0, sizeof(regs));
        if (!strncmp(line, "open ", 5))
            run_open(line);
        else if (!strncmp(line, "start ", 6))
            run_start(line);
        else {
            fprintf(stderr, "unknown case: %s\n", line);
            return 1;
        }
        printf("%s => %s\n", line, out);
    }
    return 0;
}
