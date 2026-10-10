/*
 * Golden data for hsync-atklogic: runs ATK-Logic's own CRC and lane order (gCRC32() from
 * pv/static/util.cpp and logic_analyzer_convert_to_device/pc() from pv/usb/usb_base.cpp, which
 * build.sh extracts into extracted.inc) together with the byte building of USBControl::Write()
 * and SessionController::start(), transcribed below without Qt, and prints the bytes of each
 * case. See README.md.
 *
 * Input, one case per line (blank lines and lines starting with '#' are copied):
 *   command code=0x10 [args=<hex>]
 *   settings buffer=0|1 rle=0|1 vth=<volts> hz=<selectHzIndex> depth=<samples> pos=<percent>
 *   trigger en=<16-bit mask> trig=<16 chars X R F C 1 0, channel 0 first> instant=0|1
 *   lanes seed=<n>
 * Output: the input line, " => " and hex: the frame as Write() builds it before padding and
 * lane order; the settings or trigger arguments; for lanes, a block of 2048 bytes made from
 * the seed, then the block after convert_to_device() and after convert_to_pc(), separated by
 * spaces.
 */

#include <cstdint>
#include <cstdio>
#include <cstring>
#include <iostream>
#include <sstream>
#include <string>
#include <vector>

typedef uint8_t quint8;
typedef uint16_t quint16;
typedef uint32_t quint32;
typedef int8_t qint8;
typedef int32_t qint32;
typedef uint64_t quint64;
typedef int64_t qint64;
typedef unsigned char uchar;

#include "extracted.inc"

/* Qt 5's qRound() for doubles and qAbs(). */
static inline int qRound(double d)
{
    return d >= 0.0 ? int(d + 0.5) : int(d - double(int(d - 1)) + 0.5) + int(d - 1);
}
template <typename T> static inline T qAbs(const T &t) { return t >= 0 ? t : -t; }

typedef std::string QByteArray;

/* pv/static/util.cpp: intToBytes(). */
static QByteArray intToBytes(quint64 value, qint8 len = 4)
{
    QByteArray src;
    for (qint8 i = 0; i < len; i++)
        src.push_back((char)(uchar)(value >> (i * 8) & 0xFF));
    return src;
}

/* pv/usb/usb_control.cpp: USBControl::Write(quint8 code, quint8 data[], qint32 len, …), up to
   the call of the other Write(). */
static std::vector<quint8> writeCode(quint8 code, const std::vector<quint8> &data)
{
    qint32 len = (qint32)data.size();
    len += 2;
    std::vector<quint8> newCode(len, 0);
    newCode[0] = code;
    newCode[1] = len - 1;
    if (len - 2 > 0)
        memcpy(newCode.data() + 2, data.data(), len - 2);
    return newCode;
}

/* pv/usb/usb_control.cpp: USBControl::Write(quint8* data, qint32 len), up to SendToDevice(). */
static std::vector<quint8> writeFrame(std::vector<quint8> data)
{
    qint32 len = (qint32)data.size();
    int offset = 8;
    len += 7 + offset;
    std::vector<quint8> pBuffer(len, 0);
    len -= 7 + offset;
    quint32 CRC32 = gCRC32(data.data(), len);
    memset(pBuffer.data() + offset, 0x0A, 1);
    memcpy(pBuffer.data() + offset + 1, data.data(), len);
    memset(pBuffer.data() + offset + 1 + len, 0x0B, 1);
    memcpy(pBuffer.data() + offset + 2 + len, &CRC32, sizeof(CRC32));
    return pBuffer;
}

/* pv/controller/session_controller.h */
enum ChannelTriggerButton { Null, RisingEdge, HighLevel, FallingEdge, LowLevel, DoubleEdge };

/* pv/controller/session_controller.cpp: SessionController::start(), the setArray part, with
   samplingDepth given rather than worked out from setHz and setTime. */
static QByteArray settings(bool isBuffer, bool isRLE, double thresholdLevel, int selectHzIndex,
                           quint64 samplingDepth, double triggerPosition)
{
    QByteArray setArray;
    uchar b = 0;
    if (isBuffer)
        b += 128;
    if (isRLE)
        b += 64;
    setArray.push_back(b);
    b = 0;
    if (thresholdLevel < 0)
        b += 128;
    b += qAbs(qRound(thresholdLevel * 10));
    setArray.push_back(b);
    setArray.push_back((uchar)selectHzIndex + 1);
    setArray.append(intToBytes(samplingDepth, 5));
    quint64 triggerSamplingDepth = (quint64)(samplingDepth / 100 * triggerPosition);
    setArray.append(intToBytes(triggerSamplingDepth, 5));
    return setArray;
}

struct Channel {
    bool enable;
    ChannelTriggerButton triggerType;
};

/* SessionController::start(), the dataBytes part of the simple trigger (type 0). */
static QByteArray trigger(const std::vector<Channel> &array, bool isInstantly)
{
    QByteArray dataBytes;
    uchar b;
    for (int i = 0; i < (int)array.size(); i += 2) {
        b = 0;
        for (int ii = 0; ii < 2; ii++) {
            int index = i + ii;
            if (array[index].enable) {
                b += (uchar)(ii % 2 == 0 ? 128 : 8);
                switch (array[index].triggerType) {
                case RisingEdge:
                    b += (uchar)(ii % 2 == 0 ? 16 : 1);
                    break;
                case HighLevel:
                    b += (uchar)(ii % 2 == 0 ? 64 : 4);
                    break;
                case FallingEdge:
                    b += (uchar)(ii % 2 == 0 ? 32 : 2);
                    break;
                case LowLevel:
                    break;
                case DoubleEdge:
                    b += (uchar)(ii % 2 == 0 ? 16 : 1);
                    b += (uchar)(ii % 2 == 0 ? 32 : 2);
                    break;
                default:
                    b += (uchar)(ii % 2 == 0 ? 16 : 1);
                    b += (uchar)(ii % 2 == 0 ? 32 : 2);
                    b += (uchar)(ii % 2 == 0 ? 64 : 4);
                    break;
                }
            }
        }
        dataBytes.push_back(b);
    }
    dataBytes.push_back((uchar)(isInstantly ? 1 : 0));
    return dataBytes;
}

static std::string hex(const void *data, size_t len)
{
    static const char digits[] = "0123456789abcdef";
    const quint8 *bytes = (const quint8 *)data;
    std::string out;
    for (size_t i = 0; i < len; i++) {
        out.push_back(digits[bytes[i] >> 4]);
        out.push_back(digits[bytes[i] & 15]);
    }
    return out;
}

static std::vector<quint8> unhex(const std::string &text)
{
    std::vector<quint8> out;
    for (size_t i = 0; i + 1 < text.size(); i += 2)
        out.push_back((quint8)std::stoul(text.substr(i, 2), nullptr, 16));
    return out;
}

static std::string value(const std::string &line, const std::string &key)
{
    std::istringstream words(line);
    std::string word;
    while (words >> word)
        if (word.compare(0, key.size() + 1, key + "=") == 0)
            return word.substr(key.size() + 1);
    return "";
}

static ChannelTriggerButton condition(char c)
{
    switch (c) {
    case 'R':
        return RisingEdge;
    case 'F':
        return FallingEdge;
    case 'C':
        return DoubleEdge;
    case '1':
        return HighLevel;
    case '0':
        return LowLevel;
    default:
        return Null;
    }
}

int main()
{
    std::string line;
    while (std::getline(std::cin, line)) {
        if (line.empty() || line[0] == '#') {
            std::cout << line << "\n";
            continue;
        }
        std::istringstream words(line);
        std::string kind;
        words >> kind;
        std::string out;
        if (kind == "command") {
            quint8 code = (quint8)std::stoul(value(line, "code"), nullptr, 0);
            std::vector<quint8> frame = writeFrame(writeCode(code, unhex(value(line, "args"))));
            out = hex(frame.data(), frame.size());
        } else if (kind == "settings") {
            QByteArray bytes = settings(value(line, "buffer") == "1", value(line, "rle") == "1",
                                        std::stod(value(line, "vth")),
                                        std::stoi(value(line, "hz")),
                                        std::stoull(value(line, "depth")),
                                        std::stod(value(line, "pos")));
            out = hex(bytes.data(), bytes.size());
        } else if (kind == "trigger") {
            unsigned enabled = (unsigned)std::stoul(value(line, "en"), nullptr, 0);
            std::string trig = value(line, "trig");
            std::vector<Channel> channels;
            for (int i = 0; i < 16; i++)
                channels.push_back(Channel{(enabled >> i & 1) != 0, condition(trig[i])});
            QByteArray bytes = trigger(channels, value(line, "instant") == "1");
            out = hex(bytes.data(), bytes.size());
        } else if (kind == "lanes") {
            unsigned seed = (unsigned)std::stoul(value(line, "seed"));
            quint8 block[2048], device[2048], pc[2048];
            for (unsigned i = 0; i < 2048; i++) {
                seed = seed * 1103515245u + 12345u;
                block[i] = (quint8)(seed >> 16);
            }
            logic_analyzer_convert_to_device(block, device, sizeof(block));
            logic_analyzer_convert_to_pc(block, pc, sizeof(block));
            out = hex(block, sizeof(block)) + " " + hex(device, sizeof(device)) + " " +
                  hex(pc, sizeof(pc));
        } else {
            std::cerr << "unknown case: " << line << "\n";
            return 1;
        }
        std::cout << line << " => " << out << "\n";
    }
    return 0;
}
