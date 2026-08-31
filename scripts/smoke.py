#!/usr/bin/env python3
"""End-to-end smoke test for the mcs51-mcp server over real stdio.

Speaks JSON-RPC to the built binary exactly the way an MCP client does, so this
exercises the actual transport rather than calling Rust functions directly.

    python3 scripts/smoke.py [path-to-binary]

Requires no 8051 hardware. Checks that cannot run without a board are skipped
and reported as such rather than silently passing.
"""

import json
import subprocess
import sys
import os

BIN = sys.argv[1] if len(sys.argv) > 1 else "./target/release/mcs51-mcp"

EXPECTED_TOOLS = {
    "doctor",
    "list_serial_ports",
    "compile",
    "flash",
    "serial_open",
    "serial_write",
    "serial_read",
    "serial_expect",
    "serial_close",
    "serial_list_sessions",
    "safety_preflight",
    "pinout",
}

results = []


def check(name, ok, detail=""):
    results.append((name, ok, detail))
    mark = "PASS" if ok else "FAIL"
    print(f"  [{mark}] {name}" + (f" — {detail}" if detail else ""))
    return ok


def rpc(proc, msg):
    proc.stdin.write(json.dumps(msg) + "\n")
    proc.stdin.flush()


def call_tool(name, args, _id):
    return {
        "jsonrpc": "2.0",
        "id": _id,
        "method": "tools/call",
        "params": {"name": name, "arguments": args},
    }


def envelope_of(result):
    """Every tool must return its envelope in structuredContent."""
    return (result or {}).get("structuredContent")


def main():
    if not os.path.exists(BIN):
        print(f"binary not found: {BIN}\nBuild it first: cargo build --release")
        return 1

    print(f"mcs51-mcp smoke test — {BIN}\n")

    proc = subprocess.Popen(
        [BIN],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        text=True,
        bufsize=1,
    )

    script = [
        {
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": {"name": "smoke", "version": "0"},
            },
        },
        {"jsonrpc": "2.0", "method": "notifications/initialized"},
        {"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}},
        call_tool("doctor", {}, 3),
        call_tool("pinout", {}, 4),
        call_tool("pinout", {"pin": 19}, 5),
        call_tool("list_serial_ports", {}, 6),
        call_tool("serial_list_sessions", {}, 7),
        # The LED case from circuits.md: active-low, 7.7 mA. Must pass.
        call_tool("safety_preflight", {"mcu_port": 1, "bit": 0, "level": "low", "load_ma": 7.7}, 8),
        # Driving RXD strands the link we are talking over. Must be blocked.
        call_tool("safety_preflight", {"mcu_port": 3, "bit": 0, "level": "low"}, 9),
        # P0 is open-drain with no internal pull-up.
        call_tool("safety_preflight", {"mcu_port": 0, "bit": 0, "level": "high", "load_ma": 5.0}, 10),
        # Over the 10 mA per-pin ceiling.
        call_tool("safety_preflight", {"mcu_port": 1, "bit": 2, "level": "low", "load_ma": 25.0}, 11),
        call_tool("safety_preflight", {"mcu_port": 9, "bit": 0, "level": "low"}, 12),
        # Failure paths must return clean envelopes, never a panic.
        call_tool("serial_open", {"port": "/dev/cu.definitely-not-a-real-port", "session": "s1"}, 13),
        call_tool("serial_read", {"session": "nonexistent"}, 14),
        call_tool("compile", {"source": "/tmp/definitely-not-here.c"}, 15),
        call_tool("flash", {"chip": "at89s", "hex": "/tmp/x.hex", "port": "/dev/cu.x"}, 16),
        call_tool("flash", {"chip": "banana", "hex": "/tmp/x.hex", "port": "/dev/cu.x"}, 17),
    ]

    for m in script:
        rpc(proc, m)
    proc.stdin.close()

    replies = {}
    for line in proc.stdout:
        line = line.strip()
        if not line:
            continue
        try:
            m = json.loads(line)
        except json.JSONDecodeError:
            check("stdout is pure JSON-RPC", False, f"non-JSON line: {line[:80]!r}")
            continue
        if "id" in m:
            replies[m["id"]] = m
    proc.wait(timeout=10)

    print("transport")
    init = replies.get(1, {}).get("result", {})
    check("initialize responds", bool(init))
    check(
        "serverInfo is mcs51-mcp (not rmcp)",
        init.get("serverInfo", {}).get("name") == "mcs51-mcp",
        str(init.get("serverInfo")),
    )
    check("protocolVersion negotiated", bool(init.get("protocolVersion")), init.get("protocolVersion", ""))

    print("\ntool surface")
    tools = {t["name"] for t in replies.get(2, {}).get("result", {}).get("tools", [])}
    check("exactly 12 tools", len(tools) == 12, f"got {len(tools)}")
    check("tool names match spec", tools == EXPECTED_TOOLS,
          f"missing={sorted(EXPECTED_TOOLS - tools)} extra={sorted(tools - EXPECTED_TOOLS)}")
    schemas = replies.get(2, {}).get("result", {}).get("tools", [])
    check("every tool declares an outputSchema",
          all(t.get("outputSchema") for t in schemas),
          f"without={[t['name'] for t in schemas if not t.get('outputSchema')]}")

    print("\nenvelope contract")
    for _id in range(3, 18):
        r = replies.get(_id, {}).get("result")
        env = envelope_of(r)
        if env is None:
            check(f"id={_id} returns structuredContent", False)
            continue
        shape_ok = "ok" in env and "status" in env
        check(f"id={_id} envelope has ok+status", shape_ok, "" if shape_ok else str(env)[:90])

    print("\ndoctor sees the real toolchain")
    doc = envelope_of(replies.get(3, {}).get("result")) or {}
    blob = json.dumps(doc)
    check("finds sdcc", "sdcc" in blob and "4.6" in blob)
    check("finds stcgal", "stcgal" in blob and "1.10" in blob)
    check("reports packihx without inventing a version", "packihx" in blob)

    print("\npinout facts")
    pin = json.dumps(envelope_of(replies.get(5, {}).get("result")) or {})
    check("pin 19 is XTAL1", "XTAL1" in pin, pin[:80])
    allp = json.dumps(envelope_of(replies.get(4, {}).get("result")) or {})
    for label in ("EA", "VCC", "GND", "RXD", "TXD", "PSEN", "ALE"):
        check(f"full pinout mentions {label}", label in allp)

    print("\nsafety_preflight rules")
    led = envelope_of(replies.get(8, {}).get("result")) or {}
    check("active-low LED at 7.7 mA passes", led.get("ok") is True, json.dumps(led)[:110])
    uart = envelope_of(replies.get(9, {}).get("result")) or {}
    check("driving P3.0 (RXD) is blocked", uart.get("ok") is False, json.dumps(uart)[:110])
    p0 = envelope_of(replies.get(10, {}).get("result")) or {}
    check("P0 driven high is flagged", p0.get("status") in ("warning", "error"), json.dumps(p0)[:110])
    over = envelope_of(replies.get(11, {}).get("result")) or {}
    check("25 mA sink exceeds 10 mA pin limit", over.get("ok") is False, json.dumps(over)[:110])
    oor = envelope_of(replies.get(12, {}).get("result")) or {}
    check("mcu_port=9 is rejected", oor.get("ok") is False, json.dumps(oor)[:110])

    print("\nfailure paths stay clean (no panics)")
    for _id, label in [
        (13, "serial_open on a bogus port"),
        (14, "serial_read on unknown session"),
        (15, "compile of a missing source"),
        (16, "flash chip=at89s (documented stub)"),
        (17, "flash with an unknown chip"),
    ]:
        env = envelope_of(replies.get(_id, {}).get("result")) or {}
        has_code = bool(env.get("error_code"))
        check(f"{label} → structured error", env.get("ok") is False and has_code,
              json.dumps(env)[:110])

    at89s = json.dumps(envelope_of(replies.get(16, {}).get("result")) or {})
    check("at89s stub names a real alternative",
          any(k in at89s.lower() for k in ("minipro", "usbasp", "isp", "programmer")))

    print("\nnot covered (no 8051 attached to this machine):")
    print("  - a real flash to a board")
    print("  - a real PING/PONG serial round-trip")

    failed = [n for n, ok, _ in results if not ok]
    print(f"\n{len(results) - len(failed)}/{len(results)} passed")
    if failed:
        print("failed: " + ", ".join(failed))
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
