//! CLI entry point for running a Raft node with an interactive TUI.
//!
//! This binary starts a Raft node and provides a terminal user interface for
//! interacting with the distributed key-value store. The TUI shows real-time
//! logs from the Raft worker alongside user commands.
//!
//! # TUI Update Strategy
//!
//! The status bar (term, role, leader) updates every 50ms via polling, which is:
//! - Half the Raft tick interval (100ms), ensuring we see state changes promptly
//! - Fast enough for real-time visibility during elections (1-2 seconds total)
//! - Low overhead at 20Hz update rate (single channel query per update)
//!
//! This time-based approach replaced the previous log-line-based updates, which
//! caused stale displays when logs were quiet.
//!
//! # Example usage
//!
//! Start a 3-node cluster:
//! ```bash
//! # Terminal 1 (node 1)
//! cargo run --bin node -- \
//!   --id 1 --listen 127.0.0.1:7101 \
//!   --peer 1=127.0.0.1:7101 --peer 2=127.0.0.1:7102 --peer 3=127.0.0.1:7103
//!
//! # Terminal 2 (node 2)
//! cargo run --bin node -- \
//!   --id 2 --listen 127.0.0.1:7102 \
//!   --peer 1=127.0.0.1:7101 --peer 2=127.0.0.1:7102 --peer 3=127.0.0.1:7103
//!
//! # Terminal 3 (node 3)
//! cargo run --bin node -- \
//!   --id 3 --listen 127.0.0.1:7103 \
//!   --peer 1=127.0.0.1:7101 --peer 2=127.0.0.1:7102 --peer 3=127.0.0.1:7103
//! ```

use std::collections::HashMap;
use std::io;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::{Parser, ValueHint};
use crossbeam_channel::Receiver;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use raft::StateRole;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use ratatui::Terminal;

use raft_replication::protocol::ConsoleCommand;
use raft_replication::runtime::{LogMessage, NodeConfig, NodeHandle, spawn_node};

/// Command-line arguments for the Raft node.
#[derive(Parser, Debug)]
#[command(author, version, about = "Run a Raft node with interactive TUI")]
struct Args {
    /// Numeric node ID (must match one entry in --peer)
    #[arg(long)]
    id: u64,

    /// Address this node should listen on for Raft messages, e.g. 127.0.0.1:7101
    #[arg(long, value_hint = ValueHint::Hostname)]
    listen: String,

    /// Comma-separated peer map: id=addr,id=addr,... (must include self)
    #[arg(long, value_delimiter = ',', value_hint = ValueHint::Other)]
    peer: Vec<String>,
}

/// Application state for the TUI.
struct App {
    /// Node ID
    node_id: u64,
    /// Current input being typed
    input: String,
    /// Cursor position in input
    cursor_position: usize,
    /// All log messages and command outputs
    output_lines: Vec<String>,
    /// Scroll offset for output window
    scroll_offset: usize,
    /// Node handle for sending commands
    handle: NodeHandle,
    /// Log message receiver
    log_rx: Receiver<LogMessage>,
    /// Current node role
    role: StateRole,
    /// Current leader ID
    leader_id: u64,
    /// Current Raft term
    term: u64,
    /// Last time status was updated from worker
    ///
    /// Status is polled every 50ms to provide real-time visibility of role changes,
    /// term increments, and leader transitions during elections. This is half the
    /// Raft tick interval (100ms), ensuring we display state changes promptly without
    /// excessive overhead.
    last_status_update: Instant,
    /// Whether to quit
    should_quit: bool,
}

impl App {
    /// Creates a new App instance.
    fn new(
        node_id: u64,
        handle: NodeHandle,
        log_rx: Receiver<LogMessage>,
    ) -> Self {
        Self {
            node_id,
            input: String::new(),
            cursor_position: 0,
            output_lines: vec![
                format!("Node {} ready. Type HELP (or h) for commands.", node_id),
                "Commands: PUT/p, GET/g, STATUS/s, CAMPAIGN/c, HELP/h, EXIT/e".to_string(),
                "Keyboard: Enter=submit, Ctrl-C/ESC=exit, Up/Down=scroll output".to_string(),
                String::new(),
            ],
            scroll_offset: 0,
            handle,
            log_rx,
            role: StateRole::Follower,
            leader_id: 0,
            term: 0,
            last_status_update: Instant::now(),
            should_quit: false,
        }
    }

    /// Processes log messages from the worker (non-blocking).
    fn drain_logs(&mut self) {
        while let Ok(log) = self.log_rx.try_recv() {
            let line = match log {
                LogMessage::Info(msg) => msg,
                LogMessage::Error(msg) => format!("ERROR: {}", msg),
            };
            self.output_lines.push(line);
        }
    }

    /// Updates node status by querying the worker.
    fn update_status(&mut self) {
        if let Ok(status) = self.handle.status() {
            self.role = status.role;
            self.leader_id = status.leader_id;
            self.term = status.term;
            self.last_status_update = Instant::now();
        }
    }

    /// Handles keyboard input.
    fn handle_key(&mut self, code: KeyCode) -> Result<()> {
        match code {
            KeyCode::Char(c) => {
                self.input.insert(self.cursor_position, c);
                self.cursor_position += 1;
            }
            KeyCode::Backspace => {
                if self.cursor_position > 0 {
                    self.cursor_position -= 1;
                    self.input.remove(self.cursor_position);
                }
            }
            KeyCode::Left => {
                self.cursor_position = self.cursor_position.saturating_sub(1);
            }
            KeyCode::Right => {
                if self.cursor_position < self.input.len() {
                    self.cursor_position += 1;
                }
            }
            KeyCode::Up => {
                self.scroll_offset = self.scroll_offset.saturating_sub(1);
            }
            KeyCode::Down => {
                self.scroll_offset = self.scroll_offset.saturating_add(1);
            }
            KeyCode::Enter => {
                self.execute_command()?;
            }
            KeyCode::Esc => {
                self.should_quit = true;
            }
            _ => {}
        }
        Ok(())
    }

    /// Executes the current input as a command.
    fn execute_command(&mut self) -> Result<()> {
        let input = self.input.trim().to_string();
        if input.is_empty() {
            return Ok(());
        }

        // Echo the command
        self.output_lines.push(format!("> {}", input));

        // Parse and execute
        match ConsoleCommand::parse(&input, true) {
            Ok(ConsoleCommand::Put { key, value }) => {
                match self.handle.put(key.clone(), value) {
                    Ok(new_value) => {
                        self.output_lines.push(format!("OK: {} = {}", key, new_value));
                    }
                    Err(err) => {
                        self.output_lines.push(format!("ERROR: {}", err));
                    }
                }
            }
            Ok(ConsoleCommand::Get { key }) => {
                match self.handle.get(key.clone())? {
                    Some(value) => {
                        self.output_lines.push(format!("{} = {}", key, value));
                    }
                    None => {
                        self.output_lines.push(format!("{} not found", key));
                    }
                }
            }
            Ok(ConsoleCommand::Status) => {
                let status = self.handle.status()?;
                self.output_lines.push(format!(
                    "Node {} | Term: {} | Role: {:?} | Leader: {}",
                    status.node_id, status.term, status.role, status.leader_id
                ));
                if status.store.is_empty() {
                    self.output_lines.push("  Store: empty".to_string());
                } else {
                    for (k, v) in status.store {
                        self.output_lines.push(format!("  {} = {}", k, v));
                    }
                }
                self.role = status.role;
                self.leader_id = status.leader_id;
                self.term = status.term;
            }
            Ok(ConsoleCommand::Campaign) => {
                match self.handle.campaign() {
                    Ok(msg) => {
                        self.output_lines.push(msg);
                    }
                    Err(err) => {
                        self.output_lines.push(format!("ERROR: {}", err));
                    }
                }
            }
            Ok(ConsoleCommand::Help) => {
                self.output_lines.push("Commands (case-insensitive):".to_string());
                self.output_lines.push("  PUT <key> <value>  (alias: p)  -- replicate via Raft".to_string());
                self.output_lines.push("  GET <key>          (alias: g)  -- read local value".to_string());
                self.output_lines.push("  STATUS             (alias: s)  -- show node state".to_string());
                self.output_lines.push("  CAMPAIGN           (alias: c)  -- force election".to_string());
                self.output_lines.push("  HELP               (alias: h)  -- show this message".to_string());
                self.output_lines.push("  EXIT               (alias: e)  -- shut down node".to_string());
            }
            Ok(ConsoleCommand::Exit) => {
                self.output_lines.push("Shutting down...".to_string());
                self.handle.shutdown()?;
                self.should_quit = true;
            }
            Err(err) => {
                self.output_lines.push(format!("ERROR: {}", err));
            }
        }

        // Clear input
        self.input.clear();
        self.cursor_position = 0;

        Ok(())
    }
}

/// Renders the TUI.
fn render_ui(frame: &mut ratatui::Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(5),        // Output window
            Constraint::Length(3),     // Input window
            Constraint::Length(1),     // Status bar
        ])
        .split(frame.area());

    // Output window (scrollable logs)
    let output_height = chunks[0].height as usize;
    let total_lines = app.output_lines.len();
    let scroll_offset = app.scroll_offset.min(total_lines.saturating_sub(output_height));
    let visible_lines: Vec<ListItem> = app
        .output_lines
        .iter()
        .skip(scroll_offset)
        .take(output_height)
        .map(|line| ListItem::new(line.as_str()))
        .collect();

    let output_block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" Output (↑/↓ to scroll, {} lines) ", total_lines));
    let output_list = List::new(visible_lines).block(output_block);
    frame.render_widget(output_list, chunks[0]);

    // Input window
    let input_block = Block::default()
        .borders(Borders::ALL)
        .title(" Input (Enter to submit, ESC/Ctrl-C to quit) ");
    let input_text = Paragraph::new(app.input.as_str()).block(input_block);
    frame.render_widget(input_text, chunks[1]);

    // Set cursor position in input area
    frame.set_cursor_position((
        chunks[1].x + app.cursor_position as u16 + 1,
        chunks[1].y + 1,
    ));

    // Status bar
    let status_text = format!(
        " Node {} | Term: {} | Role: {:?} | Leader: {} ",
        app.node_id, app.term, app.role, app.leader_id
    );
    let status_style = match app.role {
        StateRole::Leader => Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
        StateRole::Candidate => Style::default().fg(Color::Yellow),
        _ => Style::default().fg(Color::White),
    };
    let status = Paragraph::new(status_text).style(status_style);
    frame.render_widget(status, chunks[2]);
}

/// Main entry point: spawns a Raft node and runs the TUI.
fn main() -> Result<()> {
    let args = Args::parse();
    let peers = parse_peers(&args.peer)?;

    // Validate that this node's ID maps to its listen address
    if peers.get(&args.id).map(String::as_str) != Some(args.listen.as_str()) {
        return Err(anyhow::anyhow!(
            "self id {} must map to listen addr {} via --peer entries",
            args.id,
            args.listen
        ));
    }

    // Spawn the Raft node
    let (handle, log_rx) = spawn_node(NodeConfig {
        id: args.id,
        listen_addr: args.listen.clone(),
        peers,
    })?;

    // Initialize TUI
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app state
    let mut app = App::new(args.id, handle, log_rx);
    app.update_status();

    // Main event loop
    let result = run_tui(&mut terminal, &mut app);

    // Cleanup TUI
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

/// Runs the TUI event loop.
fn run_tui(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> Result<()> {
    loop {
        // Drain any pending log messages
        app.drain_logs();

        // Render UI
        terminal.draw(|frame| render_ui(frame, app))?;

        // Poll for keyboard events with timeout
        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    // Handle Ctrl-C
                    if key.code == KeyCode::Char('c')
                        && key.modifiers.contains(event::KeyModifiers::CONTROL)
                    {
                        app.should_quit = true;
                    } else {
                        app.handle_key(key.code)?;
                    }
                }
            }
        }

        // Update status every 50ms for real-time display
        //
        // Why 50ms?
        // - Real-time visibility: Elections complete within 1-2 seconds, so we need
        //   frequent updates to show intermediate states (Follower→Candidate→Leader)
        // - Half the Raft tick: At 50ms (half of the 100ms tick interval), we catch
        //   state changes quickly without polling faster than Raft can progress
        // - Low overhead: Status query is a single channel send/recv, negligible cost
        //   at 20Hz update rate
        if app.last_status_update.elapsed() >= Duration::from_millis(50) {
            app.update_status();
        }

        if app.should_quit {
            break;
        }
    }

    Ok(())
}

/// Parses peer entries from the command line into a map.
fn parse_peers(entries: &[String]) -> Result<HashMap<u64, String>> {
    let mut peers = HashMap::new();
    for entry in entries {
        let Some((id_str, addr)) = entry.split_once('=') else {
            return Err(anyhow::anyhow!(
                "invalid peer entry '{entry}', expected id=addr"
            ));
        };
        let id: u64 = id_str
            .parse()
            .with_context(|| format!("invalid peer id in '{entry}'"))?;
        peers.insert(id, addr.to_string());
    }
    if peers.is_empty() {
        return Err(anyhow::anyhow!(
            "at least one --peer entry is required (include self)"
        ));
    }
    Ok(peers)
}
