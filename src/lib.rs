//! A pluggable `unsegen` ANSI terminal.
//!
//! # Examples:
//! ```no_run
//! extern crate unsegen;
//! use std::io::stdout;
//!
//! use unsegen::base;
//! use unsegen::widget::{RenderingHints, Widget};
//!
//! use unsegen_terminal::{SlaveInputSink, Terminal};
//!
//! use std::sync::mpsc;
//!
//! struct MpscSlaveInputSink(mpsc::Sender<Box<[u8]>>);
//!
//! impl SlaveInputSink for MpscSlaveInputSink {
//!     fn receive_bytes_from_pty(&mut self, data: Box<[u8]>) {
//!         self.0.send(data).unwrap();
//!     }
//! }
//!
//! fn main() {
//!     let stdout = stdout();
//!
//!     let (pty_sink, pty_src) = mpsc::channel();
//!
//!     let mut term = base::Terminal::new(stdout.lock());
//!
//!     let mut term_widget = Terminal::new(MpscSlaveInputSink(pty_sink)).unwrap();
//!     println!("Created pty: {}", term_widget.slave_name().to_str().unwrap());
//!
//!     while let Ok(bytes) = pty_src.recv() {
//!         // Read input and do further processing here...
//!
//!         // When you write to the created pty, the input should appear on screen!
//!         term_widget.add_byte_input(&bytes);
//!         {
//!             let win = term.create_root_window();
//!             term_widget.draw(win, RenderingHints::default());
//!         }
//!         term.present();
//!     }
//! }
//! ```
extern crate libc;
extern crate nix;
extern crate unsegen;
extern crate vte;
#[allow(dead_code)]
mod ansi;
#[allow(dead_code)]
mod index;
mod pty;
mod terminalwindow;

use ansi::Processor;
use pty::{PTYInput, PTYOutput, PTY};
use std::ffi::{OsStr, OsString};
use unsegen::base::basic_types::*;
use unsegen::base::Window;
use unsegen::container::Container;
use unsegen::input::{Behavior, Input, Key, OperationResult, ScrollBehavior, Scrollable, Writable};
use unsegen::widget::{Demand2D, RenderingHints, Widget};

use terminalwindow::DualWindow;

use std::cell::RefCell;
use std::fs::File;
use std::thread;

fn read_slave_input_loop<S: SlaveInputSink>(mut sink: S, mut reader: PTYOutput) {
    use std::io::Read;

    let mut buffer = [0; 1024];
    while let Ok(n) = reader.read(&mut buffer) {
        let mut bytes = vec![0; n];
        bytes.copy_from_slice(&mut buffer[..n]);
        sink.receive_bytes_from_pty(bytes.into_boxed_slice());
    }
}
/// Implement this trait by forwarding all received bytes to your main loop (somehow, for example
/// using using `chan`). Then, in the main loop call `Terminal::add_byte_input` and update the
/// screen.
///
/// (This is not as elegant as it could be and is subject to change in future versions. Support for
/// Futures is planned once they are stable.)
pub trait SlaveInputSink: std::marker::Send {
    fn receive_bytes_from_pty(&mut self, data: Box<[u8]>);
}

/// An unsegen `Behavior` that passes all (raw!) inputs through to the modelled terminal.
pub struct PassthroughBehavior<'a> {
    term: &'a mut Terminal,
}

impl<'a> PassthroughBehavior<'a> {
    pub fn new(term: &'a mut Terminal) -> Self {
        PassthroughBehavior { term: term }
    }
}

impl<'a> Behavior for PassthroughBehavior<'a> {
    fn input(self, i: Input) -> Option<Input> {
        self.term.process_input(i);
        None
    }
}

/// unsegen `Widget` that models a pseudoterminal and displays its contents to the window when
/// drawn.
///
/// Use `ScrollBehavior` to scroll in the (potentially infinite buffer) and `WriteBehavior` to
/// pass specific keystrokes to the terminal.
pub struct Terminal {
    terminal_window: RefCell<DualWindow>,
    //slave_input_thread: thread::Thread,
    master_input_sink: RefCell<PTYInput>,

    // Hack used to keep the slave device open as long as the master exists.
    // This may not be a good idea, we will see...
    _slave_handle: File,
    slave_name: OsString,

    ansi_processor: Processor,
}

impl Terminal {
    /// Create a Terminal which will use the provided `SlaveInputSink` to notify the user of new
    /// input from the pty.
    ///
    /// This method will create a posix pty. The associated file (path) can be obtained using
    /// `get_slave_name`.
    pub fn new<S: SlaveInputSink + 'static>(input_sink: S) -> std::io::Result<Self> {
        let process_pty = PTY::open().expect("Could not create pty.");

        let ptsname = process_pty.name().to_owned();

        let (pty_input, pty_output) = process_pty.split_io();

        /*let slave_input_thread =*/
        thread::Builder::new()
            .name("slave input thread".to_owned())
            .spawn(move || {
                read_slave_input_loop(input_sink, pty_output);
            })?;

        // Hack:
        // Open slave terminal, so that it does not get destroyed when a gdb process opens it and
        // closes it afterwards.
        let mut pts = std::fs::OpenOptions::new()
            .write(true)
            .read(true)
            .open(&ptsname)?;
        use std::io::Write;
        write!(pts, "")?;

        Ok(Terminal {
            terminal_window: RefCell::new(DualWindow::new()),
            master_input_sink: RefCell::new(pty_input),
            //slave_input_thread: slave_input_thread,
            _slave_handle: pts,
            slave_name: ptsname,
            ansi_processor: Processor::new(),
        })
    }

    /// Add _raw_ byte input to the terminal window. Call this for bytes that you received
    /// (indirectly) from SlaveInputSink::receive_bytes_from_pty.
    pub fn add_byte_input(&mut self, bytes: &[u8]) {
        use std::ops::DerefMut;
        let mut window_ref = self.terminal_window.borrow_mut();
        let mut sink_ref = self.master_input_sink.borrow_mut();
        for byte in bytes.iter() {
            self.ansi_processor
                .advance(window_ref.deref_mut(), *byte, sink_ref.deref_mut());
        }
    }

    /// Get the name of the slave pseudoterminal that is associated with the `Terminal`.
    ///
    /// (c.f. posix `ptsname`)
    pub fn slave_name(&self) -> &OsStr {
        self.slave_name.as_ref()
    }

    /// Forward the raw input to the terminal.
    fn process_input(&mut self, i: Input) {
        use std::io::Write;
        self.master_input_sink
            .borrow_mut()
            .write_all(i.raw.as_slice())
            .expect("Write to terminal");
    }

    /// Make sure that the underlying state of the terminal windows matches the specified size.
    fn ensure_size(&self, w: Width, h: Height) {
        let mut window = self.terminal_window.borrow_mut();
        if w != window.get_width() || h != window.get_height() {
            window.set_width(w);
            window.set_height(h);

            let w16 = w.raw_value() as u16;
            let h16 = h.raw_value() as u16;
            self.master_input_sink
                .borrow_mut()
                .resize(w16, h16, w16 /* TODO ??*/, h16 /* TODO ??*/)
                .expect("Resize pty");
        }
    }
}

impl Writable for Terminal {
    fn write(&mut self, c: char) -> OperationResult {
        use std::io::Write;
        write!(self.master_input_sink.borrow_mut(), "{}", c).expect("Write key to terminal");
        Ok(())
    }
}

impl Scrollable for Terminal {
    fn scroll_forwards(&mut self) -> OperationResult {
        self.terminal_window.borrow_mut().scroll_forwards()
    }
    fn scroll_backwards(&mut self) -> OperationResult {
        self.terminal_window.borrow_mut().scroll_backwards()
    }
    fn scroll_to_beginning(&mut self) -> OperationResult {
        self.terminal_window.borrow_mut().scroll_to_beginning()
    }
    fn scroll_to_end(&mut self) -> OperationResult {
        self.terminal_window.borrow_mut().scroll_to_end()
    }
}

impl Widget for Terminal {
    fn space_demand(&self) -> Demand2D {
        self.terminal_window.borrow().space_demand()
    }
    fn draw(&self, window: Window, hints: RenderingHints) {
        self.ensure_size(window.get_width(), window.get_height());
        self.terminal_window.borrow_mut().draw(window, hints);
    }
}

/// Default container behavior:
///
/// Scroll using `PageUp`/`PageDown`, jump to beginning/end using `Home`/`End` and pass all other
/// input to the slave terminal.
impl<P: ?Sized> Container<P> for Terminal {
    fn input(&mut self, input: Input, _: &mut P) -> Option<Input> {
        input
            .chain(
                ScrollBehavior::new(self)
                    .forwards_on(Key::PageDown)
                    .backwards_on(Key::PageUp)
                    .to_beginning_on(Key::Home)
                    .to_end_on(Key::End),
            )
            .chain(PassthroughBehavior::new(self))
            .finish()
    }
}

#[cfg(test)]
impl Terminal {
    fn write(&mut self, s: &str) {
        self.add_byte_input(s.as_bytes());
    }
}
#[cfg(test)]
mod test {
    use super::*;
    use unsegen::base::terminal::test::FakeTerminal;
    use unsegen::base::GraphemeCluster;

    struct FakeSlaveInputSink;
    impl SlaveInputSink for FakeSlaveInputSink {
        fn receive_bytes_from_pty(&mut self, _: Box<[u8]>) {}
    }
    fn test_terminal<F: Fn(&mut Terminal)>(window_dim: (u32, u32), after: &str, action: F) {
        let mut term = FakeTerminal::with_size(window_dim);
        {
            let mut window = term.create_root_window();
            window.fill(GraphemeCluster::try_from('_').unwrap());
            let mut tw = Terminal::new(FakeSlaveInputSink).unwrap();
            tw.terminal_window.get_mut().set_show_cursor(false);
            action(&mut tw);
            tw.draw(window, RenderingHints::default());
        }
        term.assert_looks_like(after);
    }
    #[test]
    fn test_terminal_window_simple() {
        test_terminal((5, 1), "_____", |w| w.write(""));
        test_terminal((5, 1), "t____", |w| w.write("t"));
        test_terminal((5, 1), "te___", |w| w.write("te"));
        test_terminal((5, 1), "tes__", |w| w.write("tes"));
        test_terminal((5, 1), "test_", |w| w.write("test"));
        test_terminal((5, 1), "testy", |w| w.write("testy"));
        test_terminal((5, 1), "o____", |w| w.write("testyo"));

        test_terminal((2, 2), "te|st", |w| w.write("te\nst"));
    }
}
