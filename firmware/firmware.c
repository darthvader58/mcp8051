/*===========================================================================
 * firmware.c - reference firmware for the mcs51-mcp 8051 dev-loop server
 *
 *   Target  : 8052 core (STC89C52RC, AT89S52, ...), DIP-40
 *   Crystal : 11.0592 MHz  - not negotiable, see uart_init() for why
 *   Link    : UART, 9600 baud, 8 data bits, no parity, 1 stop bit
 *   Build   : sdcc -mmcs51 firmware.c && packihx firmware.ihx > firmware.hex
 *
 * The board sits in a loop reading newline-terminated ASCII commands from the
 * serial port and poking the I/O ports.  Exactly one reply line per command:
 *
 *   PING          -> PONG
 *   SET p b v     -> OK        drive bit b of port p to v   (p 0-3, b 0-7, v 0/1)
 *   GET p b       -> 0 | 1     sample bit b of port p       (reads the PIN)
 *   WRP p hh      -> OK        drive all 8 bits of port p
 *   RDP p         -> hh        sample all 8 bits of port p  (reads the PINS)
 *   anything else -> ERR
 *
 * See PROTOCOL.md for the full wire format, worked examples and error cases.
 *
 * This file is meant to be read as much as run.  It is deliberately plain C:
 * no interrupts, no lookup tables, no pointer tricks.  An 8051 has 256 bytes
 * of RAM and this has to stay obviously correct.
 *
 * SDCC C only.  There is no usable C++ toolchain for the MCS-51 core.
 *=========================================================================*/

#include <8052.h>

/* Longest legal command is "WRP 3 FF" (8 chars), so 32 is roomy.  The buffer
 * is bounded on purpose: read_line() below can never write past it no matter
 * what the host sends. */
#define LINE_MAX   32

/* PING / SET p b v -> at most 4 whitespace-separated tokens. */
#define MAX_TOKENS  4


/*---------------------------------------------------------------------------
 * Shadow latches
 *
 * Every 8051 port pin is a latch driving a weak pull-up plus a strong
 * pull-down.  Reading the SFR name (P1) reads the *pin*; assigning to it
 * writes the *latch*.  Those are not the same thing: if something external
 * holds a pin low - a closed switch, another driver - then reading P1 gives 0
 * for that bit even though the latch holds 1.
 *
 * So a naive read-modify-write bit operation ("P1 = P1 | 0x01") would copy the
 * externally-forced 0 back into the latch and permanently clamp that pin low.
 * The bug is invisible until you wire up an input.
 *
 * The fix is to keep our own copy of what we last *drove* and always start bit
 * operations from that.  0xFF is the 8051's port state after reset - all
 * latches high, every pin free to be pulled down by the outside world - so the
 * shadows start there and main() drives that same value out once at boot to
 * make "shadow == latch" true from the first instruction.
 *-------------------------------------------------------------------------*/
static unsigned char shadow[4] = { 0xFF, 0xFF, 0xFF, 0xFF };

/* Command line buffer and its tokens (pointers into line[]). */
static char  line[LINE_MAX];
static char *tok[MAX_TOKENS];


/*===========================================================================
 * UART - polled, 9600 8N1
 *=========================================================================*/

/*
 * Baud rate comes from Timer 1 overflowing in mode 2 (8-bit auto-reload):
 *
 *     baud = crystal / (12 * 32 * (256 - TH1))          with SMOD = 0
 *
 * The 12 is the machine cycle (12 oscillator periods), the 32 is the UART's
 * own divide-by-16 doubled because SMOD is 0.  With TH1 = 0xFD the reload
 * count is 256 - 253 = 3:
 *
 *     11059200 / (12 * 32 * 3) = 11059200 / 1152 = 9600.000   exactly
 *
 * That exactness is the whole reason for the odd-looking 11.0592 MHz crystal.
 * A "nicer" 12 MHz part gives 12000000 / 1152 = 10416.7 baud - 8.5% off, which
 * is far outside the ~2% a UART can tolerate, and every byte comes back as
 * garbage.  Do not change the crystal without recomputing TH1, and note that
 * for most common crystals no integer TH1 lands on 9600 at all.
 */
static void uart_init(void)
{
    EA = 0;             /* fully polled firmware - no interrupts, ever      */

    TMOD &= 0x0F;       /* clear Timer 1's nibble, leave Timer 0 untouched  */
    TMOD |= 0x20;       /* Timer 1, mode 2: 8-bit auto-reload               */
    TH1   = 0xFD;       /* reload value  -> 9600 baud (see above)           */
    TL1   = 0xFD;       /* first count starts at the reload value too       */
    PCON &= 0x7F;       /* SMOD = 0: no baud doubling                       */

    SCON  = 0x50;       /* mode 1 (8-bit UART, timer baud) + REN=1 (rx on)  */
    TI    = 0;          /* start with both flags clear                      */
    RI    = 0;
    TR1   = 1;          /* run Timer 1: the baud clock is now ticking       */
}

/* Send one byte and wait for it to leave.  TI is set by hardware when the
 * stop bit has been shifted out; it never self-clears, so we clear it. */
static void uart_tx(char c)
{
    SBUF = c;
    while (!TI) { /* wait for the transmit-complete flag */ }
    TI = 0;
}

/* Block until a byte arrives.  Same deal: RI is hardware-set, software-cleared. */
static char uart_rx(void)
{
    while (!RI) { /* wait for a received byte */ }
    RI = 0;
    return (char)SBUF;
}

/* Send a reply line.  CR+LF so a dumb terminal renders it properly; a host
 * parser just trims the line ending. */
static void uart_reply(const char *s)
{
    while (*s) {
        uart_tx(*s);
        s++;
    }
    uart_tx('\r');
    uart_tx('\n');
}


/*===========================================================================
 * Port access
 *
 * The port SFRs live in the special-function-register space, which the 8051
 * can only reach by direct addressing - there is no way to index them through
 * a pointer.  A switch is the honest way to turn a port number into a port.
 *=========================================================================*/

/* Read the physical PINS of port p.  This is the point of an input: it must
 * NOT return shadow[p], or GET/RDP would only ever echo what we last wrote.
 *
 * Note that a pin can only be read as an input if its latch holds 1 - a latch
 * holding 0 actively drives the pin low and it will always read 0.  Reset
 * leaves every latch at 1, so pins are input-ready out of the box; if you have
 * driven a pin low, release it with "SET p b 1" before reading it.
 *
 * Port 0 is open-drain with no internal pull-up, so P0 reads float unless you
 * have fitted external pull-ups (10k typical).  P1/P2/P3 have weak internal
 * pull-ups and read high when unconnected. */
static unsigned char port_read_pins(unsigned char p)
{
    switch (p) {
    case 0:  return P0;
    case 1:  return P1;
    case 2:  return P2;
    default: return P3;
    }
}

/* Drive all 8 latches of port p and record what we drove. */
static void port_write(unsigned char p, unsigned char v)
{
    shadow[p] = v;
    switch (p) {
    case 0:  P0 = v; break;
    case 1:  P1 = v; break;
    case 2:  P2 = v; break;
    default: P3 = v; break;
    }
}


/*===========================================================================
 * Tiny string / parsing helpers
 *
 * Hand-rolled rather than pulled from <string.h> and <stdlib.h>: these are a
 * few instructions each, and every one of them rejects malformed input
 * explicitly instead of doing something undefined with it.
 *=========================================================================*/

static unsigned char is_space(char c)
{
    return (c == ' ' || c == '\t') ? 1 : 0;
}

static char to_upper(char c)
{
    return (c >= 'a' && c <= 'z') ? (char)(c - ('a' - 'A')) : c;
}

/* Case-insensitive compare of a token against an UPPERCASE keyword literal.
 * Accepting "set" as well as "SET" costs nothing and makes the board far
 * friendlier to talk to by hand from a terminal. */
static unsigned char keyword_is(const char *s, const char *kw)
{
    while (*kw) {
        if (to_upper(*s) != *kw)
            return 0;
        s++;
        kw++;
    }
    return (*s == '\0') ? 1 : 0;   /* token must end exactly where kw does */
}

/* Parse a token that must be exactly one decimal digit in [lo, hi].
 * Returns the value, or 0xFF if the token is anything else.  Single-digit-only
 * is deliberate: it rejects "10", "007" and "-1" without any extra code. */
static unsigned char parse_digit(const char *s, unsigned char lo, unsigned char hi)
{
    unsigned char v;

    if (s[0] < '0' || s[0] > '9')
        return 0xFF;
    if (s[1] != '\0')
        return 0xFF;

    v = (unsigned char)(s[0] - '0');
    if (v < lo || v > hi)
        return 0xFF;
    return v;
}

/* One hex digit -> 0..15, or 0xFF if it is not a hex digit. */
static unsigned char hex_value(char c)
{
    if (c >= '0' && c <= '9') return (unsigned char)(c - '0');
    if (c >= 'A' && c <= 'F') return (unsigned char)(c - 'A' + 10);
    if (c >= 'a' && c <= 'f') return (unsigned char)(c - 'a' + 10);
    return 0xFF;
}

/* Parse a token that must be exactly two hex digits ("FF", "0f", ...).
 * Returns 1 and stores the byte on success, 0 on failure. */
static unsigned char parse_hex8(const char *s, unsigned char *out)
{
    unsigned char hi, lo;

    hi = hex_value(s[0]);
    if (hi == 0xFF) return 0;
    lo = hex_value(s[1]);
    if (lo == 0xFF) return 0;
    if (s[2] != '\0') return 0;      /* exactly two digits, no more */

    *out = (unsigned char)((hi << 4) | lo);
    return 1;
}

/* Nibble -> uppercase hex character. */
static char hex_digit(unsigned char n)
{
    n &= 0x0F;
    return (n < 10) ? (char)('0' + n) : (char)('A' + (n - 10));
}

/* Send a byte as exactly two UPPERCASE hex digits, as a reply line. */
static void reply_hex8(unsigned char v)
{
    char out[3];

    out[0] = hex_digit((unsigned char)(v >> 4));
    out[1] = hex_digit(v);
    out[2] = '\0';
    uart_reply(out);
}


/*===========================================================================
 * Line input
 *=========================================================================*/

/*
 * Read one command line into line[].  Returns 1 if the line was too long to
 * fit (the caller answers ERR), 0 otherwise.
 *
 * CR and LF are both accepted as end-of-line.  A terminal typically sends CR
 * on Enter, a script typically sends LF, and a CRLF pair sends both - so the
 * CRLF case delivers one command followed by one *empty* line.  main() answers
 * nothing at all to an empty line, which is what keeps replies exactly 1:1
 * with commands no matter which line ending the other end uses.
 *
 * Overflow is handled by dropping the excess rather than by writing past the
 * buffer: we keep consuming to the end of the line so one over-long line
 * produces exactly one ERR instead of a cascade of them.
 */
static unsigned char read_line(void)
{
    unsigned char n = 0;
    unsigned char overflow = 0;
    char c;

    for (;;) {
        c = uart_rx();
        if (c == '\r' || c == '\n')
            break;
        if (n < (LINE_MAX - 1))
            line[n++] = c;
        else
            overflow = 1;
    }

    line[n] = '\0';
    return overflow;
}

/*
 * Split line[] in place into tok[0..n-1] on runs of spaces/tabs.  Leading and
 * trailing whitespace simply vanish, so "  GET 1 0  " parses like "GET 1 0".
 *
 * Returns the token count, or 0xFF if there are more than MAX_TOKENS tokens
 * (which the caller turns into ERR - no command takes that many arguments).
 */
static unsigned char tokenize(void)
{
    unsigned char n = 0;
    char *s = line;

    for (;;) {
        while (is_space(*s))
            s++;
        if (*s == '\0')
            break;

        if (n == MAX_TOKENS)
            return 0xFF;            /* trailing junk after a full command */

        tok[n++] = s;
        while (*s != '\0' && !is_space(*s))
            s++;
        if (*s != '\0')
            *s++ = '\0';            /* terminate this token, step past it */
    }

    return n;
}


/*===========================================================================
 * Command handling
 *=========================================================================*/

/*
 * The RXD/TXD guard.
 *
 * P3.0 is RXD and P3.1 is TXD - the one and only link the host has to this
 * board.  Driving either of them as a GPIO cuts the wire we are talking over:
 * a stray "SET 3 0 0" would strand the session with no way back except a power
 * cycle, and no error message, because the error message cannot get out.
 *
 * So any write that would touch P3.0 or P3.1 is refused, replies ERR, and
 * changes nothing.  Reads are always allowed - sampling a pin is harmless.
 *
 * Because WRP writes all eight bits at once, "WRP 3 hh" is rejected outright
 * rather than mask-preserving bits 0-1.  Silently writing something other than
 * what was asked for and still answering OK would be the worse failure: the
 * host would believe all 8 bits landed.  P3.2-P3.7 remain fully writable one
 * at a time with "SET 3 b v".
 */
static unsigned char write_would_hit_uart(unsigned char p, unsigned char b)
{
    return (p == 3 && (b == 0 || b == 1)) ? 1 : 0;
}

static void handle_command(unsigned char n)
{
    unsigned char p, b, v, mask, val;

    /* ---- PING ---------------------------------------------------------- */
    if (n == 1 && keyword_is(tok[0], "PING")) {
        uart_reply("PONG");
        return;
    }

    /* ---- SET p b v ------------------------------------------------------ */
    if (n == 4 && keyword_is(tok[0], "SET")) {
        p = parse_digit(tok[1], 0, 3);
        b = parse_digit(tok[2], 0, 7);
        v = parse_digit(tok[3], 0, 1);
        if (p == 0xFF || b == 0xFF || v == 0xFF) {
            uart_reply("ERR");
            return;
        }
        if (write_would_hit_uart(p, b)) {
            uart_reply("ERR");          /* RXD/TXD guard - nothing changed */
            return;
        }

        /* Start from the shadow, never from a re-read of the pins. */
        mask = (unsigned char)(1u << b);
        if (v)
            val = (unsigned char)(shadow[p] | mask);
        else
            val = (unsigned char)(shadow[p] & (unsigned char)~mask);

        port_write(p, val);
        uart_reply("OK");
        return;
    }

    /* ---- GET p b -------------------------------------------------------- */
    if (n == 3 && keyword_is(tok[0], "GET")) {
        p = parse_digit(tok[1], 0, 3);
        b = parse_digit(tok[2], 0, 7);
        if (p == 0xFF || b == 0xFF) {
            uart_reply("ERR");
            return;
        }

        mask = (unsigned char)(1u << b);
        /* Read the PIN, not the shadow - that is the whole point of an input. */
        uart_reply((port_read_pins(p) & mask) ? "1" : "0");
        return;
    }

    /* ---- WRP p hh ------------------------------------------------------- */
    if (n == 3 && keyword_is(tok[0], "WRP")) {
        p = parse_digit(tok[1], 0, 3);
        if (p == 0xFF || !parse_hex8(tok[2], &val)) {
            uart_reply("ERR");
            return;
        }
        if (p == 3) {
            uart_reply("ERR");          /* would write P3.0/P3.1 - refused */
            return;
        }

        port_write(p, val);
        uart_reply("OK");
        return;
    }

    /* ---- RDP p ---------------------------------------------------------- */
    if (n == 2 && keyword_is(tok[0], "RDP")) {
        p = parse_digit(tok[1], 0, 3);
        if (p == 0xFF) {
            uart_reply("ERR");
            return;
        }

        reply_hex8(port_read_pins(p));  /* pins again, not the shadow */
        return;
    }

    /* ---- anything else -------------------------------------------------- */
    uart_reply("ERR");
}


/*===========================================================================
 * main
 *=========================================================================*/

void main(void)
{
    unsigned char i;
    unsigned char overflow;
    unsigned char n;

    /* Drive the shadow values out once so "shadow == latch" holds from the
     * very start.  0xFF is already the post-reset state, so nothing on the
     * board twitches; this just makes the invariant explicit.  Writing 0xFF to
     * P3 is safe and in fact required - the UART only drives TXD when that
     * latch bit is 1. */
    for (i = 0; i < 4; i++)
        port_write(i, shadow[i]);

    uart_init();

    /* The firmware never speaks unless spoken to: no boot banner, no
     * unsolicited output.  Every byte the host sees is a reply to a command it
     * sent, which keeps host-side parsing trivial.  Use PING to check life. */
    for (;;) {
        overflow = read_line();
        n = tokenize();

        if (overflow || n == 0xFF) {
            uart_reply("ERR");          /* line too long, or too many tokens */
        } else if (n == 0) {
            /* Blank line - stay silent.  This is what makes a CRLF terminal
             * work without emitting a spurious ERR for every Enter. */
        } else {
            handle_command(n);
        }
    }
}
