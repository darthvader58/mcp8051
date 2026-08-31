//! Table-driven checks of every `safety_preflight` rule.

use mcs51_mcp::config::Config;
use mcs51_mcp::server::Server;
use mcs51_mcp::tools::safety::{Level, SafetyArgs};
use serde_json::Value;

fn server() -> Server {
    Server::new(Config::default())
}

/// Run the tool and return its envelope as JSON, the way a caller sees it.
async fn preflight(mcu_port: u8, bit: u8, level: Level, load_ma: Option<f64>) -> Value {
    let s = server();
    let env = mcs51_mcp::tools::safety::run(
        &s,
        SafetyArgs {
            mcu_port,
            bit,
            level,
            load_ma,
        },
    )
    .await
    .expect("safety_preflight should always produce an envelope");
    serde_json::to_value(env).unwrap()
}

fn codes(v: &Value) -> Vec<String> {
    v["data"]["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .map(|f| f["code"].as_str().unwrap().to_string())
        .collect()
}

fn severity_of(v: &Value, code: &str) -> String {
    v["data"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["code"] == code)
        .unwrap_or_else(|| panic!("expected finding {code}, got {:?}", codes(v)))["severity"]
        .as_str()
        .unwrap()
        .to_string()
}

/// One row of the rule table.
struct Case {
    what: &'static str,
    port: u8,
    bit: u8,
    level: Level,
    load: Option<f64>,
    verdict: &'static str,
    /// Codes that must appear.
    expect: &'static [&'static str],
    /// Codes that must not appear.
    reject: &'static [&'static str],
}

#[tokio::test]
async fn the_rule_table_holds() {
    let cases = [
        Case {
            what: "the reference demo LED: active-low, 330R, 7.7 mA",
            port: 1,
            bit: 0,
            level: Level::Low,
            load: Some(7.7),
            verdict: "pass_with_warnings",
            expect: &["PORT_AGGREGATE_ADVISORY"],
            reject: &["SINK_CURRENT_OVER_PIN_LIMIT", "SINK_CURRENT_NEAR_PIN_LIMIT"],
        },
        Case {
            what: "the naive LED: drive high into 10 mA, which the pin cannot source",
            port: 1,
            bit: 0,
            level: Level::High,
            load: Some(10.0),
            verdict: "blocked",
            expect: &["SOURCE_CURRENT_INSUFFICIENT"],
            reject: &[],
        },
        Case {
            what: "sourcing below the 0.1 mA threshold is not flagged",
            port: 1,
            bit: 3,
            level: Level::High,
            load: Some(0.05),
            verdict: "pass_with_warnings",
            expect: &["PORT_AGGREGATE_ADVISORY"],
            reject: &["SOURCE_CURRENT_INSUFFICIENT"],
        },
        Case {
            what: "sinking 25 mA is over the 10 mA per-pin ceiling",
            port: 1,
            bit: 2,
            level: Level::Low,
            load: Some(25.0),
            verdict: "blocked",
            expect: &["SINK_CURRENT_OVER_PIN_LIMIT"],
            reject: &["SINK_CURRENT_NEAR_PIN_LIMIT"],
        },
        Case {
            what: "sinking 9 mA is legal but close to the ceiling",
            port: 1,
            bit: 2,
            level: Level::Low,
            load: Some(9.0),
            verdict: "pass_with_warnings",
            expect: &["SINK_CURRENT_NEAR_PIN_LIMIT"],
            reject: &["SINK_CURRENT_OVER_PIN_LIMIT"],
        },
        Case {
            what: "P3.0 is RXD, the link itself",
            port: 3,
            bit: 0,
            level: Level::Low,
            load: None,
            verdict: "blocked",
            expect: &["UART_PIN_CONFLICT", "LOAD_NOT_SPECIFIED"],
            reject: &[],
        },
        Case {
            what: "P3.1 is TXD",
            port: 3,
            bit: 1,
            level: Level::High,
            load: Some(1.0),
            verdict: "blocked",
            expect: &["UART_PIN_CONFLICT"],
            reject: &[],
        },
        Case {
            what: "P0 driven high into a load has no pull-up to do it with",
            port: 0,
            bit: 0,
            level: Level::High,
            load: Some(5.0),
            verdict: "blocked",
            expect: &[
                "P0_NEEDS_EXTERNAL_PULLUP",
                "SOURCE_CURRENT_INSUFFICIENT",
                "EXTERNAL_MEMORY_PIN",
            ],
            reject: &[],
        },
        Case {
            what: "P0 sinking is fine, but the open drain is still worth saying",
            port: 0,
            bit: 3,
            level: Level::Low,
            load: Some(5.0),
            verdict: "pass_with_warnings",
            expect: &["P0_NEEDS_EXTERNAL_PULLUP"],
            reject: &["SOURCE_CURRENT_INSUFFICIENT", "SINK_CURRENT_OVER_PIN_LIMIT"],
        },
        Case {
            what: "P1.5 is MOSI, shared with the SPI ISP programmer",
            port: 1,
            bit: 5,
            level: Level::Low,
            load: Some(1.0),
            verdict: "pass_with_warnings",
            expect: &["ISP_PIN_CONFLICT"],
            reject: &[],
        },
        Case {
            what: "P1.7 is SCK",
            port: 1,
            bit: 7,
            level: Level::Low,
            load: Some(1.0),
            verdict: "pass_with_warnings",
            expect: &["ISP_PIN_CONFLICT"],
            reject: &[],
        },
        Case {
            what: "P3.2 is INT0 — worth knowing, not worth blocking",
            port: 3,
            bit: 2,
            level: Level::Low,
            load: Some(1.0),
            verdict: "pass_with_warnings",
            expect: &["INTERRUPT_PIN_REPURPOSED"],
            reject: &["UART_PIN_CONFLICT"],
        },
        Case {
            what: "P2 doubles as the high address byte",
            port: 2,
            bit: 4,
            level: Level::Low,
            load: Some(1.0),
            verdict: "pass_with_warnings",
            expect: &["EXTERNAL_MEMORY_PIN"],
            reject: &[],
        },
        Case {
            what: "omitting load_ma skips the current checks, and says so",
            port: 1,
            bit: 4,
            level: Level::Low,
            load: None,
            verdict: "pass_with_warnings",
            expect: &["LOAD_NOT_SPECIFIED"],
            reject: &["SINK_CURRENT_OVER_PIN_LIMIT", "SINK_CURRENT_NEAR_PIN_LIMIT"],
        },
        Case {
            what: "an input draws nothing, so no aggregate advisory",
            port: 1,
            bit: 4,
            level: Level::Input,
            load: None,
            verdict: "pass",
            expect: &["LOAD_NOT_SPECIFIED"],
            reject: &["PORT_AGGREGATE_ADVISORY"],
        },
        Case {
            what: "port 9 does not exist",
            port: 9,
            bit: 0,
            level: Level::Low,
            load: None,
            verdict: "blocked",
            expect: &["PORT_OUT_OF_RANGE"],
            reject: &[],
        },
        Case {
            what: "bit 8 does not exist",
            port: 1,
            bit: 8,
            level: Level::Low,
            load: None,
            verdict: "blocked",
            expect: &["BIT_OUT_OF_RANGE"],
            reject: &[],
        },
    ];

    for c in cases {
        let v = preflight(c.port, c.bit, c.level, c.load).await;
        let got = codes(&v);
        assert_eq!(
            v["data"]["verdict"], c.verdict,
            "{}: verdict was {:?}, findings {got:?}",
            c.what, v["data"]["verdict"]
        );
        for want in c.expect {
            assert!(
                got.iter().any(|g| g == want),
                "{}: expected {want}, got {got:?}",
                c.what
            );
        }
        for unwanted in c.reject {
            assert!(
                !got.iter().any(|g| g == unwanted),
                "{}: did not expect {unwanted}, got {got:?}",
                c.what
            );
        }
    }
}

#[tokio::test]
async fn blocked_sets_ok_false_so_a_caller_skimming_ok_cannot_miss_it() {
    let v = preflight(3, 0, Level::Low, None).await;
    assert_eq!(v["ok"], false);
    assert_eq!(v["status"], "error");
    assert_eq!(v["error_code"], "SAFETY_BLOCKED");
    assert!(v["error"].as_str().unwrap().contains("RXD"));
    assert!(v["remedy"].is_string());
}

#[tokio::test]
async fn warnings_keep_ok_true() {
    let v = preflight(1, 0, Level::Low, Some(7.7)).await;
    assert_eq!(v["ok"], true);
    assert_eq!(v["status"], "warning");
    assert_eq!(v["error_code"], Value::Null);
}

#[tokio::test]
async fn a_clean_pass_is_status_ok() {
    let v = preflight(1, 4, Level::Input, None).await;
    assert_eq!(v["ok"], true);
    assert_eq!(v["status"], "ok");
    assert_eq!(v["data"]["verdict"], "pass");
}

#[tokio::test]
async fn severities_are_graded_not_uniform() {
    let p0_high = preflight(0, 0, Level::High, Some(5.0)).await;
    assert_eq!(severity_of(&p0_high, "P0_NEEDS_EXTERNAL_PULLUP"), "blocker");

    // Same pin, no load, only reading: worth a warning, not a block.
    let p0_input = preflight(0, 0, Level::Input, None).await;
    assert_eq!(
        severity_of(&p0_input, "P0_NEEDS_EXTERNAL_PULLUP"),
        "warning"
    );
    assert_eq!(severity_of(&p0_input, "EXTERNAL_MEMORY_PIN"), "info");
}

#[tokio::test]
async fn findings_name_the_physical_pin_from_the_shared_table() {
    // safety_preflight and pinout read the same table, so the pin numbering
    // they report must agree — including Port 0's descending order.
    let v = preflight(0, 0, Level::Low, Some(1.0)).await;
    assert_eq!(v["data"]["pin"]["pin"], 39, "P0.0 is physical pin 39");
    assert_eq!(v["data"]["pin"]["name"], "P0.0");

    let v = preflight(0, 7, Level::Low, Some(1.0)).await;
    assert_eq!(v["data"]["pin"]["pin"], 32, "P0.7 is physical pin 32");

    let v = preflight(1, 0, Level::Low, Some(1.0)).await;
    assert_eq!(v["data"]["pin"]["pin"], 1);
    assert_eq!(v["data"]["pin"]["name"], "P1.0");
}

#[tokio::test]
async fn the_uart_warning_escalates_when_a_session_is_actually_open() {
    use mcs51_mcp::serial::SessionRegistry;

    let s = server();
    let quiet = preflight(3, 0, Level::Low, None).await;
    let quiet_msg = quiet["data"]["findings"][0]["message"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(!quiet_msg.contains("open serial session"));

    // Now with a live session on the books. The registry is behind a trait, so
    // this needs no hardware.
    let registry = SessionRegistry::with_opener(
        4,
        std::sync::Arc::new(|_p: &str, _b: u32, _t: std::time::Duration| {
            Ok(Box::new(FakePort) as Box<dyn mcs51_mcp::serial::SerialLink>)
        }),
    );
    registry
        .open(
            "board",
            "/dev/cu.usbserial-10",
            9600,
            std::time::Duration::from_millis(50),
        )
        .unwrap();

    let s = s.with_session_registry(registry);
    let env = mcs51_mcp::tools::safety::run(
        &s,
        SafetyArgs {
            mcu_port: 3,
            bit: 0,
            level: Level::Low,
            load_ma: None,
        },
    )
    .await
    .unwrap();
    let v = serde_json::to_value(env).unwrap();
    let msg = v["data"]["findings"][0]["message"].as_str().unwrap();
    assert!(
        msg.contains("open serial session") && msg.contains("/dev/cu.usbserial-10"),
        "with a live session the message should name it: {msg}"
    );
    assert_eq!(v["ok"], false);
}

struct FakePort;

impl mcs51_mcp::serial::SerialLink for FakePort {
    fn set_timeout(&mut self, _t: std::time::Duration) -> std::io::Result<()> {
        Ok(())
    }
    fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
        Err(std::io::Error::from(std::io::ErrorKind::TimedOut))
    }
    fn write_all(&mut self, _buf: &[u8]) -> std::io::Result<()> {
        Ok(())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
