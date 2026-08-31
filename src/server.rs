//! rmcp wiring: server state, twelve tool shims, and `ServerHandler`.
//!
//! `#[tool_router]` only collects `#[tool]` functions declared in *its own*
//! impl block, so all twelve signatures have to live here. Each body is three
//! lines that delegate to [`crate::tools`], where the real work can use `?`.

use std::sync::{Arc, Mutex};

use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, Implementation, ProtocolVersion, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler,
};

use crate::config::Config;
use crate::envelope::{deliver, envelope_output_schema};
use crate::errors::AppError;
use crate::names;
use crate::paths::PathResolver;
use crate::serial::SessionRegistry;
use crate::tools::{self, Invocation};

#[derive(Clone)]
pub struct Server {
    config: Arc<Config>,
    pub paths: Arc<PathResolver>,
    pub sessions: SessionRegistry,
    /// Which stcgal invocation actually works. `doctor` fills this in; `flash`
    /// reuses it instead of re-probing on every call.
    stcgal: Arc<Mutex<Option<Invocation>>>,
    tool_router: ToolRouter<Server>,
}

impl Server {
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Swap in a different session registry.
    ///
    /// Exists so tests can inject a fake opener and exercise the tools that
    /// read live session state — `safety_preflight`'s UART rule, `flash`'s
    /// port-held check — without a USB adapter attached.
    #[must_use]
    pub fn with_session_registry(mut self, sessions: SessionRegistry) -> Self {
        self.sessions = sessions;
        self
    }

    /// Record the stcgal invocation `doctor` proved works.
    pub fn set_stcgal(&self, invocation: Option<Invocation>) {
        let mut slot = self.stcgal.lock().unwrap_or_else(|e| e.into_inner());
        if invocation.is_some() {
            *slot = invocation;
        }
    }

    /// The cached invocation, probing once if `doctor` has not run yet.
    pub async fn stcgal_invocation(&self) -> Result<Invocation, AppError> {
        if let Some(cached) = self
            .stcgal
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
        {
            return Ok(cached);
        }
        let (_report, found) = tools::doctor::probe_stcgal(&self.config).await;
        match found {
            Some(inv) => {
                self.set_stcgal(Some(inv.clone()));
                Ok(inv)
            }
            None => Err(AppError::ToolNotFound {
                tool: "stcgal".into(),
            }),
        }
    }
}

#[tool_router]
impl Server {
    pub fn new(config: Config) -> Self {
        let paths = PathResolver::new(config.firmware_root.clone());
        let sessions = SessionRegistry::new(config.max_sessions);
        Self {
            config: Arc::new(config),
            paths: Arc::new(paths),
            sessions,
            stcgal: Arc::new(Mutex::new(None)),
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        description = "Report which parts of the 8051 toolchain are installed and usable: sdcc \
                       and packihx (from SDCC) for compiling, stcgal for flashing STC parts. \
                       Also reports FIRMWARE_ROOT and whether path confinement is on, how many \
                       serial ports are attached, and the host architecture. Call this first \
                       when anything fails unexpectedly.",
        output_schema = envelope_output_schema()
    )]
    async fn doctor(&self) -> Result<CallToolResult, McpError> {
        Ok(deliver(names::DOCTOR, tools::doctor::run(self).await))
    }

    #[tool(
        description = "List attached serial ports, ranked with the ones you should actually use \
                       first. macOS exposes every device twice (/dev/cu.* and /dev/tty.*); the \
                       /dev/tty.* node blocks on DCD and will hang. Includes USB vendor/product \
                       ids, manufacturer and product strings, and the likely bridge chip.",
        output_schema = envelope_output_schema()
    )]
    async fn list_serial_ports(&self) -> Result<CallToolResult, McpError> {
        Ok(deliver(
            names::LIST_SERIAL_PORTS,
            tools::serial_session::list_ports(self).await,
        ))
    }

    #[tool(
        description = "Compile a C source for the 8051 with `sdcc -mmcs51`, then pack the result \
                       into an Intel-HEX image with packihx. On failure the sdcc diagnostics are \
                       returned verbatim. The produced .hex is validated by content (packihx \
                       always exits 0, so its exit code proves nothing).",
        output_schema = envelope_output_schema()
    )]
    async fn compile(
        &self,
        Parameters(args): Parameters<tools::compile::CompileArgs>,
    ) -> Result<CallToolResult, McpError> {
        Ok(deliver(
            names::COMPILE,
            tools::compile::run(self, args).await,
        ))
    }

    #[tool(
        description = "Write an Intel-HEX image to the target. chip=\"stc\" flashes an STC89C52 \
                       through its serial bootloader with stcgal — the board MUST be \
                       power-cycled after the call starts, because the bootloader only listens \
                       just after power-up. chip=\"at89s\" is not supported and returns an \
                       explanation: AT89S parts program over SPI ISP and need a hardware \
                       programmer.",
        output_schema = envelope_output_schema()
    )]
    async fn flash(
        &self,
        Parameters(args): Parameters<tools::flash::FlashArgs>,
    ) -> Result<CallToolResult, McpError> {
        Ok(deliver(names::FLASH, tools::flash::run(self, args).await))
    }

    #[tool(
        description = "Open a serial session to the board and give it an id that later serial_* \
                       calls use. Defaults to 9600 baud, which is what the reference firmware \
                       runs at with an 11.0592 MHz crystal. Use a /dev/cu.* port path.",
        output_schema = envelope_output_schema()
    )]
    async fn serial_open(
        &self,
        Parameters(args): Parameters<tools::serial_session::OpenArgs>,
    ) -> Result<CallToolResult, McpError> {
        Ok(deliver(
            names::SERIAL_OPEN,
            tools::serial_session::open(self, args).await,
        ))
    }

    #[tool(
        description = "Send a line to an open session. A trailing newline is appended if you \
                       omit one, because the firmware's line protocol only acts on complete \
                       lines. Commands: PING, SET p b v, GET p b, WRP p hh, RDP p.",
        output_schema = envelope_output_schema()
    )]
    async fn serial_write(
        &self,
        Parameters(args): Parameters<tools::serial_io::WriteArgs>,
    ) -> Result<CallToolResult, McpError> {
        Ok(deliver(
            names::SERIAL_WRITE,
            tools::serial_io::write(self, args).await,
        ))
    }

    #[tool(
        description = "Read whatever the board has sent, waiting up to timeout_ms \
                       (default 1000). Returns early once the line goes quiet, so a fast reply \
                       does not cost the whole window.",
        output_schema = envelope_output_schema()
    )]
    async fn serial_read(
        &self,
        Parameters(args): Parameters<tools::serial_io::ReadArgs>,
    ) -> Result<CallToolResult, McpError> {
        Ok(deliver(
            names::SERIAL_READ,
            tools::serial_io::read(self, args).await,
        ))
    }

    #[tool(
        description = "Wait for a literal substring to arrive on an open session, returning the \
                       instant it matches rather than at the end of the window. Use this \
                       instead of serial_read plus a guessed delay. Fails with PATTERN_NOT_FOUND \
                       (including what did arrive) if timeout_ms passes first.",
        output_schema = envelope_output_schema()
    )]
    async fn serial_expect(
        &self,
        Parameters(args): Parameters<tools::serial_io::ExpectArgs>,
    ) -> Result<CallToolResult, McpError> {
        Ok(deliver(
            names::SERIAL_EXPECT,
            tools::serial_io::expect(self, args).await,
        ))
    }

    #[tool(
        description = "Close a serial session and release its port. If an operation is still \
                       running the close is recorded and applied the moment that operation \
                       finishes. Call this before flashing a port you have open.",
        output_schema = envelope_output_schema()
    )]
    async fn serial_close(
        &self,
        Parameters(args): Parameters<tools::serial_session::CloseArgs>,
    ) -> Result<CallToolResult, McpError> {
        Ok(deliver(
            names::SERIAL_CLOSE,
            tools::serial_session::close(self, args).await,
        ))
    }

    #[tool(
        description = "List open serial sessions with their id, port, baud, age, bytes read and \
                       written, and state (idle, busy, or poisoned).",
        output_schema = envelope_output_schema()
    )]
    async fn serial_list_sessions(&self) -> Result<CallToolResult, McpError> {
        Ok(deliver(
            names::SERIAL_LIST_SESSIONS,
            tools::serial_session::list_sessions(self).await,
        ))
    }

    #[tool(
        description = "Check a pin plan against the AT89S52/STC89C52 datasheet before anything \
                       is wired. Catches the mistakes that quietly destroy an 8051 project: \
                       driving a pin high into a load it can only sink (ports source ~60 uA but \
                       sink 10 mA), using Port 0 without an external pull-up, exceeding the \
                       per-pin or per-port current budget, and repurposing P3.0/P3.1 — the UART \
                       this server talks over. Returns findings rolled up to pass, \
                       pass_with_warnings, or blocked; blocked sets ok=false.",
        output_schema = envelope_output_schema()
    )]
    async fn safety_preflight(
        &self,
        Parameters(args): Parameters<tools::safety::SafetyArgs>,
    ) -> Result<CallToolResult, McpError> {
        Ok(deliver(
            names::SAFETY_PREFLIGHT,
            tools::safety::run(self, args).await,
        ))
    }

    #[tool(
        description = "DIP-40 pinout for the 8051/8052 (AT89S52, STC89C52). Pass a pin number \
                       for one pin, or omit it for the whole package plus port summaries, \
                       current budgets and clock notes. Note Port 0 descends: pin 32 is P0.7 and \
                       pin 39 is P0.0.",
        output_schema = envelope_output_schema()
    )]
    async fn pinout(
        &self,
        Parameters(args): Parameters<tools::pinout::PinoutArgs>,
    ) -> Result<CallToolResult, McpError> {
        Ok(deliver(names::PINOUT, tools::pinout::run(self, args).await))
    }
}

// `router = self.tool_router` is not optional: bare `#[tool_handler]` compiles
// fine but silently defaults to `Self::tool_router()`, rebuilding the whole
// router on every tools/call and leaving the field above dead.
#[tool_handler(router = self.tool_router)]
impl ServerHandler for Server {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                env!("CARGO_PKG_NAME"),
                env!("CARGO_PKG_VERSION"),
            ))
            .with_protocol_version(ProtocolVersion::V_2025_11_25)
            .with_instructions(INSTRUCTIONS.to_string())
    }
}

const INSTRUCTIONS: &str = "\
The 8051 (MCS-51) development loop, host-side. This server runs on the host: it compiles with \
SDCC, flashes over a USB-TTL adapter, and talks to the board over UART. Nothing here runs on the \
microcontroller.

Typical loop:
  1. doctor                 - confirm sdcc/packihx/stcgal are present
  2. safety_preflight       - check the pin plan BEFORE wiring or driving anything
  3. compile                - sdcc -mmcs51, then packihx, with the .hex validated by content
  4. flash chip=\"stc\"       - stcgal; POWER-CYCLE the board after the call starts
  5. serial_open / serial_write / serial_expect / serial_close

Every tool returns the same envelope: ok, status, tool, error_code, error, remedy, command, \
exit_code, duration_ms, stdout, stderr, data, next_actions. Branch on `ok` and `error_code`; \
`next_actions` suggests the call to make next.

Hardware facts worth knowing before you generate firmware or wiring advice:
  - Ports SINK ~10 mA but SOURCE only ~60 uA. Wire loads active-low: VCC -> R -> LED -> pin, and \
drive the pin LOW to turn it on. Driving a pin high into a load does not work on this part.
  - Port 0 is open-drain with no internal pull-up; it needs an external pull-up (10k) to drive high.
  - P3.0/P3.1 are RXD/TXD, the only link to the board. Driving them as GPIO strands the session \
until a power cycle; the reference firmware answers ERR to SET 3 0 and SET 3 1.
  - Current budget is three tiers: 10 mA per pin, 26 mA for Port 0 and 15 mA for each of Ports \
1-3, 71 mA across the whole device.
  - The 11.0592 MHz crystal is what makes 9600 baud exact (TH1 = 0xFD, SMOD = 0).

Firmware line protocol over 9600 8N1, newline-terminated ASCII:
  PING -> PONG | SET p b v -> OK | GET p b -> 0|1 | WRP p hh -> OK | RDP p -> hh | else -> ERR
";
