/*
 * libsigrok functions only pxlogic.c's driver table and paths the golden cases never take refer
 * to. Built apart from harness.c, whose headers declare them, so they need no real signatures.
 */

#include <stdlib.h>

#define UNREACHED(name) \
    void name(void) { abort(); }

UNREACHED(sr_dev_inst_free)
UNREACHED(sr_usb_dev_inst_free)
UNREACHED(sr_usb_dev_inst_new)
UNREACHED(sr_usb_find)
UNREACHED(std_cleanup)
UNREACHED(std_dev_clear)
UNREACHED(std_dev_list)
UNREACHED(std_gvar_array_i32)
UNREACHED(std_init)
UNREACHED(std_opts_config_list)
UNREACHED(std_scan_complete)
UNREACHED(std_session_send_df_header)
UNREACHED(std_session_send_df_trigger)
UNREACHED(usb_source_add)
UNREACHED(usb_source_remove)
