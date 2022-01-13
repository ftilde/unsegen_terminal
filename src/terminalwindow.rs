use ansi;
use ansi::{Attr, CursorStyle, Handler, TermInfo};
use unsegen::base::basic_types::*;
use unsegen::base::Color as UColor;
use unsegen::base::{
    BoolModifyMode, Cursor, CursorState, CursorTarget, Style, StyleModifier, StyledGraphemeCluster,
    Window, WrappingMode, UNBOUNDED_HEIGHT, UNBOUNDED_WIDTH,
};
use unsegen::input::{OperationResult, Scrollable};
use unsegen::widget::{Demand, Demand2D, RenderingHints};

use log::warn;

use index;
use std::cmp::{max, min};
use std::fmt::Write;
use std::ops::{Deref, DerefMut};

#[derive(Clone)]
struct Line {
    content: Vec<StyledGraphemeCluster>,
}

impl Line {
    fn empty() -> Self {
        Line {
            content: Vec::new(),
        }
    }

    fn length(&self) -> u32 {
        self.content.len() as u32
    }

    fn clear(&mut self) {
        self.content.clear();
    }

    fn height_for_width(&self, width: Width) -> Height {
        //TODO: this might not be correct if there are wide clusters within the content, hmm...
        if width == 0 {
            Height::new(1).unwrap()
        } else {
            Height::new(self.length().checked_sub(1).unwrap_or(0) as i32 / width.raw_value() + 1)
                .unwrap()
        }
    }

    fn get_cell_mut(&mut self, x: ColIndex) -> Option<&mut StyledGraphemeCluster> {
        if x < 0 {
            return None;
        }
        let x = x.raw_value() as usize;
        // Grow horizontally to desired position
        let missing_elements = (x + 1).checked_sub(self.content.len()).unwrap_or(0);
        self.content
            .extend(::std::iter::repeat(StyledGraphemeCluster::default()).take(missing_elements));

        let element = self
            .content
            .get_mut(x)
            .expect("element existent assured previously");
        Some(element)
    }

    fn get_cell(&self, x: ColIndex) -> Option<&StyledGraphemeCluster> {
        if x < 0 {
            return None;
        }
        /*
        //TODO: maybe we want to grow? problems with mutability...
        // Grow horizontally to desired position
        let missing_elements = (x as usize+ 1).checked_sub(self.content.len()).unwrap_or(0);
        self.content.extend(::std::iter::repeat(StyledGraphemeCluster::default()).take(missing_elements));
        */

        let element = self
            .content
            .get(x.raw_value() as usize)
            .expect("element existent assured previously");
        Some(element)
    }
}

struct LineBuffer {
    lines: Vec<Line>,
    window_width: Width,
    default_style: Style,
}
impl LineBuffer {
    pub fn new() -> Self {
        LineBuffer {
            lines: Vec::new(),
            window_width: Width::new(0).unwrap(),
            default_style: Style::default(),
        }
    }

    fn height_as_displayed(&self) -> Height {
        self.lines
            .iter()
            .map(|l| l.height_for_width(self.window_width))
            .sum()
    }

    pub fn set_window_width(&mut self, w: Width) {
        self.window_width = w;
    }
}

impl CursorTarget for LineBuffer {
    fn get_width(&self) -> Width {
        Width::new(UNBOUNDED_WIDTH).unwrap()
    }
    fn get_soft_width(&self) -> Width {
        self.window_width
    }
    fn get_height(&self) -> Height {
        Height::new(UNBOUNDED_HEIGHT).unwrap()
    }
    fn get_cell_mut(&mut self, x: ColIndex, y: RowIndex) -> Option<&mut StyledGraphemeCluster> {
        if y < 0 {
            return None;
        }
        let y = y.raw_value() as usize;
        // Grow vertically to desired position
        let missing_elements = (y + 1).checked_sub(self.lines.len()).unwrap_or(0);
        self.lines
            .extend(::std::iter::repeat(Line::empty()).take(missing_elements));

        let line = self
            .lines
            .get_mut(y)
            .expect("line existence assured previously");

        line.get_cell_mut(x)
    }
    fn get_cell(&self, x: ColIndex, y: RowIndex) -> Option<&StyledGraphemeCluster> {
        /*
        //TODO: maybe we want to grow? problems with mutability...
        // Grow vertically to desired position
        let missing_elements = (y as usize + 1).checked_sub(self.lines.len()).unwrap_or(0);
        self.lines.extend(::std::iter::repeat(Line::empty()).take(missing_elements));
        */

        if y < 0 {
            return None;
        }

        let line = self
            .lines
            .get(y.raw_value() as usize)
            .expect("line existence assured previously");

        line.get_cell(x)
    }
    fn get_default_style(&self) -> Style {
        self.default_style
    }
}

pub struct TerminalWindow {
    window_width: Width,
    window_height: Height,
    buffer: LineBuffer,
    cursor_state: CursorState,
    scrollback_position: Option<RowIndex>,
    scroll_step: Height,
    cursor_style: CursorStyle,

    // Terminal state
    show_cursor: bool,
}

impl TerminalWindow {
    pub fn new() -> Self {
        TerminalWindow {
            window_width: Width::new(0).unwrap(),
            window_height: Height::new(0).unwrap(),
            buffer: LineBuffer::new(),
            cursor_state: CursorState::default(),
            scrollback_position: None,
            scroll_step: Height::new(1).unwrap(),
            cursor_style: CursorStyle::Block,

            show_cursor: true,
        }
    }

    // position of the first (displayed) row of the buffer that will NOT be displayed
    fn current_scrollback_pos(&self) -> RowIndex {
        self.scrollback_position
            .unwrap_or(self.buffer.height_as_displayed().from_origin())
    }

    #[cfg(test)]
    pub fn set_show_cursor(&mut self, show: bool) {
        self.show_cursor = show;
    }

    pub fn set_width(&mut self, w: Width) {
        self.window_width = w;
        self.buffer.set_window_width(w);
    }

    pub fn set_height(&mut self, h: Height) {
        self.window_height = h;
    }

    pub fn get_width(&self) -> Width {
        self.window_width
    }

    pub fn get_height(&self) -> Height {
        self.window_height
    }

    fn with_cursor<F: FnOnce(&mut Cursor<LineBuffer>)>(&mut self, f: F) {
        let mut state = CursorState::default();
        ::std::mem::swap(&mut state, &mut self.cursor_state);
        let mut cursor = Cursor::from_state(&mut self.buffer, state);
        f(&mut cursor);
        self.cursor_state = cursor.into_state();
    }

    fn line_to_buffer_pos_y(&self, line: index::Line) -> RowIndex {
        RowIndex::new(
            max(
                0,
                self.buffer.lines.len() as i32 - self.window_height.raw_value(),
            ) + line.0 as i32,
        )
    }
    fn col_to_buffer_pos_x(&self, col: index::Column) -> ColIndex {
        ColIndex::new(col.0 as i32)
    }

    pub fn space_demand(&self) -> Demand2D {
        // at_least => We can grow if there is space
        // However, we also don't ask for the complete width/height of the terminal in order to
        // avoid hogging space when the window size is reduced.
        Demand2D {
            width: Demand::at_least(1),
            height: Demand::at_least(1),
        }
    }

    pub fn draw(&mut self, mut window: Window, _: RenderingHints) {
        let cursor_style_mod = match self.cursor_style {
            CursorStyle::Beam => {
                // TODO: not sure how to emulate a beam...
                StyleModifier::new().underline(BoolModifyMode::Toggle)
            }
            CursorStyle::Block => StyleModifier::new().invert(BoolModifyMode::Toggle),
            CursorStyle::Underline => StyleModifier::new().underline(BoolModifyMode::Toggle),
        };
        //temporarily change buffer to show cursor:
        if self.show_cursor {
            self.with_cursor(|cursor| {
                if let Some(cell) = cursor.get_current_cell_mut() {
                    cursor_style_mod.modify(&mut cell.style);
                }
            });
        }

        let height = window.get_height();
        let width = window.get_width();

        if height == 0 || width == 0 || self.buffer.lines.is_empty() {
            return;
        }

        let scrollback_offset =
            -(self.current_scrollback_pos() - self.buffer.height_as_displayed());
        let minimum_y_start = scrollback_offset + height;
        let start_line = self
            .buffer
            .lines
            .len()
            .checked_sub(minimum_y_start.raw_value() as usize)
            .unwrap_or(0);
        let line_range = start_line..;
        let y_start: RowIndex = min(
            RowIndex::new(0),
            minimum_y_start
                - self.buffer.lines[line_range.clone()]
                    .iter()
                    .map(|line| line.height_for_width(width))
                    .sum::<Height>(),
        );
        let mut cursor = Cursor::new(&mut window)
            .position(ColIndex::new(0), y_start)
            .wrapping_mode(WrappingMode::Wrap);
        for line in self.buffer.lines[line_range].iter() {
            cursor.write_preformatted(line.content.as_slice());
            cursor.wrap_line();
        }

        //revert cursor change
        if self.show_cursor {
            self.with_cursor(|cursor| {
                if let Some(cell) = cursor.get_current_cell_mut() {
                    cursor_style_mod.modify(&mut cell.style);
                }
            });
        }
    }
}

fn ansi_to_unsegen_color(ansi_color: ansi::Color) -> UColor {
    match ansi_color {
        ansi::Color::Named(c) => match c {
            ansi::NamedColor::Black => UColor::Black,
            ansi::NamedColor::Red => UColor::Red,
            ansi::NamedColor::Green => UColor::Green,
            ansi::NamedColor::Yellow => UColor::Yellow,
            ansi::NamedColor::Blue => UColor::Blue,
            ansi::NamedColor::Magenta => UColor::Magenta,
            ansi::NamedColor::Cyan => UColor::Cyan,
            ansi::NamedColor::White => UColor::White,
            ansi::NamedColor::BrightBlack => UColor::LightBlack,
            ansi::NamedColor::BrightRed => UColor::LightRed,
            ansi::NamedColor::BrightGreen => UColor::LightGreen,
            ansi::NamedColor::BrightYellow => UColor::LightYellow,
            ansi::NamedColor::BrightBlue => UColor::LightBlue,
            ansi::NamedColor::BrightMagenta => UColor::LightMagenta,
            ansi::NamedColor::BrightCyan => UColor::LightCyan,
            ansi::NamedColor::BrightWhite => UColor::LightWhite,
            ansi::NamedColor::Foreground => UColor::White, //??
            ansi::NamedColor::Background => UColor::Black, //??
            ansi::NamedColor::CursorText => {
                // This is kind of tricky to get...
                UColor::Black
            }
            ansi::NamedColor::Cursor => {
                // This is kind of tricky to get...
                UColor::Black
            }
            // Also not sure what to do here
            ansi::NamedColor::DimBlack => UColor::Black,
            ansi::NamedColor::DimRed => UColor::Red,
            ansi::NamedColor::DimGreen => UColor::Green,
            ansi::NamedColor::DimYellow => UColor::Yellow,
            ansi::NamedColor::DimBlue => UColor::Blue,
            ansi::NamedColor::DimMagenta => UColor::Magenta,
            ansi::NamedColor::DimCyan => UColor::Cyan,
            ansi::NamedColor::DimWhite => UColor::White,
        },
        ansi::Color::Spec(c) => UColor::Rgb {
            r: c.r,
            g: c.g,
            b: c.b,
        },
        ansi::Color::Indexed(c) => {
            //TODO: We might in the future implement a separate color table, but for new we "reuse"
            //the table of the underlying terminal:
            UColor::Ansi(c)
        }
    }
}

enum BufferMode {
    Main,
    Alternate,
}

pub struct DualWindow {
    main: TerminalWindow,
    alternate: TerminalWindow,
    mode: BufferMode,
}

impl DualWindow {
    pub fn new() -> Self {
        DualWindow {
            main: TerminalWindow::new(),
            alternate: TerminalWindow::new(),
            mode: BufferMode::Main,
        }
    }
}

impl Deref for DualWindow {
    type Target = TerminalWindow;

    fn deref(&self) -> &Self::Target {
        match self.mode {
            BufferMode::Main => &self.main,
            BufferMode::Alternate => &self.alternate,
        }
    }
}

impl DerefMut for DualWindow {
    fn deref_mut(&mut self) -> &mut Self::Target {
        match self.mode {
            BufferMode::Main => &mut self.main,
            BufferMode::Alternate => &mut self.alternate,
        }
    }
}

impl Handler for DualWindow {
    /// OSC to set window title
    fn set_title(&mut self, _: &str) {
        //TODO: (Although this might not make sense to implement. Do we want to display a title?)
    }

    /// Set the cursor style
    fn set_cursor_style(&mut self, style: CursorStyle) {
        self.cursor_style = style;
    }

    /// A character to be displayed
    fn input(&mut self, c: char) {
        self.with_cursor(|cursor| {
            write!(cursor, "{}", c).unwrap();
        });
    }

    /// Set cursor to position
    fn goto(&mut self, line: index::Line, col: index::Column) {
        let x = self.col_to_buffer_pos_x(col);
        let y = self.line_to_buffer_pos_y(line);
        self.with_cursor(|cursor| {
            cursor.move_to(x, y);
        });
    }

    /// Set cursor to specific row
    fn goto_line(&mut self, line: index::Line) {
        let y = self.line_to_buffer_pos_y(line);
        self.with_cursor(|cursor| {
            cursor.move_to_y(y);
        });
    }

    /// Set cursor to specific column
    fn goto_col(&mut self, col: index::Column) {
        let x = self.col_to_buffer_pos_x(col);
        self.with_cursor(|cursor| {
            cursor.move_to_x(x);
        });
    }

    /// Insert blank characters in current line starting from cursor
    fn insert_blank(&mut self, _: index::Column) {
        //TODO
        warn!("Unimplemented: insert_blank");
    }

    /// Move cursor up `rows`
    fn move_up(&mut self, line: index::Line) {
        self.with_cursor(|cursor| {
            cursor.move_by(ColDiff::new(0), RowDiff::new(-(line.0 as i32)));
        });
    }

    /// Move cursor down `rows`
    fn move_down(&mut self, line: index::Line) {
        self.with_cursor(|cursor| {
            cursor.move_by(ColDiff::new(0), RowDiff::new(line.0 as i32));
        });
    }

    /// Identify the terminal (should write back to the pty stream)
    ///
    /// TODO this should probably return an io::Result
    fn identify_terminal<W: ::std::io::Write>(&mut self, _: &mut W) {
        //TODO
        warn!("Unimplemented: identify_terminal");
    }

    /// Report device status
    fn device_status<W: ::std::io::Write>(&mut self, _: &mut W, _: usize) {
        //TODO
        warn!("Unimplemented: device_status");
    }

    /// Move cursor forward `cols`
    fn move_forward(&mut self, cols: index::Column) {
        self.with_cursor(|cursor| {
            for _ in 0..cols.0 {
                cursor.move_right();
            }
        });
    }

    /// Move cursor backward `cols`
    fn move_backward(&mut self, cols: index::Column) {
        self.with_cursor(|cursor| {
            for _ in 0..cols.0 {
                cursor.move_left();
            }
        });
    }

    /// Move cursor down `rows` and set to column 1
    fn move_down_and_cr(&mut self, _: index::Line) {
        //TODO
        warn!("Unimplemented: move_down_and_cr");
    }

    /// Move cursor up `rows` and set to column 1
    fn move_up_and_cr(&mut self, _: index::Line) {
        //TODO
        warn!("Unimplemented: move_up_and_cr");
    }

    /// Put `count` tabs
    fn put_tab(&mut self, count: i64) {
        self.with_cursor(|cursor| {
            for _ in 0..count {
                write!(cursor, "\t").unwrap();
            }
        });
    }

    /// Backspace `count` characters
    fn backspace(&mut self) {
        self.with_cursor(|cursor| {
            cursor.move_left();
        });
    }

    /// Carriage return
    fn carriage_return(&mut self) {
        self.with_cursor(|cursor| cursor.carriage_return());
    }

    /// Linefeed
    fn linefeed(&mut self) {
        self.with_cursor(|cursor| {
            // Slight hack:
            // Write something into the new line to force the buffer to update it's size.
            cursor.write("\n ");
            cursor.move_by(ColDiff::new(-1), RowDiff::new(0));
        });
    }

    /// Ring the bell
    fn bell(&mut self) {
        //omitted
    }

    /// Substitute char under cursor
    fn substitute(&mut self) {
        //TODO... substitute with what?
        warn!("Unimplemented: substitute");
    }

    /// Newline
    fn newline(&mut self) {
        //TODO
        warn!("Unimplemented: newline");
    }

    /// Set current position as a tabstop
    fn set_horizontal_tabstop(&mut self) {
        //TODO
        warn!("Unimplemented: set_horizontal_tabstop");
    }

    /// Scroll up `rows` rows
    fn scroll_up(&mut self, _: index::Line) {
        //TODO
        warn!("Unimplemented: scroll_up");
    }

    /// Scroll down `rows` rows
    fn scroll_down(&mut self, _: index::Line) {
        //TODO
        warn!("Unimplemented: scroll_down");
    }

    /// Insert `count` blank lines
    fn insert_blank_lines(&mut self, _: index::Line) {
        //TODO
        warn!("Unimplemented: insert_blank_lines");
    }

    /// Delete `count` lines
    fn delete_lines(&mut self, _: index::Line) {
        //TODO
        warn!("Unimplemented: delete_lines");
    }

    /// Erase `count` chars in current line following cursor
    ///
    /// Erase means resetting to the default state (default colors, no content,
    /// no mode flags)
    fn erase_chars(&mut self, _: index::Column) {
        //TODO
        warn!("Unimplemented: erase_chars");
    }

    /// Delete `count` chars
    ///
    /// Deleting a character is like the delete key on the keyboard - everything
    /// to the right of the deleted things is shifted left.
    fn delete_chars(&mut self, _: index::Column) {
        //TODO
        warn!("Unimplemented: delete_chars");
    }

    /// Move backward `count` tabs
    fn move_backward_tabs(&mut self, _count: i64) {
        //TODO
        warn!("Unimplemented: move_backward_tabs");
    }

    /// Move forward `count` tabs
    fn move_forward_tabs(&mut self, _count: i64) {
        //TODO
        warn!("Unimplemented: move_forward_tabs");
    }

    /// Save current cursor position
    fn save_cursor_position(&mut self) {
        //TODO
        warn!("Unimplemented: save_cursor_position");
    }

    /// Restore cursor position
    fn restore_cursor_position(&mut self) {
        //TODO
        warn!("Unimplemented: restore_cursor_position");
    }

    /// Clear current line
    fn clear_line(&mut self, mode: ansi::LineClearMode) {
        self.with_cursor(|cursor| match mode {
            ansi::LineClearMode::Right => {
                cursor.clear_line_right();
            }
            ansi::LineClearMode::Left => {
                cursor.clear_line_left();
            }
            ansi::LineClearMode::All => {
                cursor.clear_line();
            }
        });
    }

    /// Clear screen
    fn clear_screen(&mut self, mode: ansi::ClearMode) {
        let clear_range = match mode {
            ansi::ClearMode::Below => {
                self.clear_line(ansi::LineClearMode::Right);

                let range_end = self.buffer.lines.len();
                let mut cursor_pos = 0;
                self.with_cursor(|cursor| cursor_pos = cursor.get_row().raw_value());
                let range_start = (cursor_pos + 1).clamp(0, range_end as i32) as usize;

                range_start..range_end
            }
            ansi::ClearMode::Above => {
                self.clear_line(ansi::LineClearMode::Left);

                let range_start = self
                    .buffer
                    .lines
                    .len()
                    .checked_sub(self.window_height.into())
                    .unwrap_or(0);

                let mut cursor_pos = 0;
                self.with_cursor(|cursor| cursor_pos = cursor.get_row().raw_value());
                let range_end =
                    cursor_pos.clamp(range_start as i32, self.buffer.lines.len() as i32) as usize;

                range_start..range_end
            }
            ansi::ClearMode::All => {
                self.buffer
                    .lines
                    .len()
                    .checked_sub(self.window_height.into())
                    .unwrap_or(0)..self.buffer.lines.len()
            }
            ansi::ClearMode::Saved => {
                warn!("Unimplemented: clear_screen saved");
                return;
            }
        };
        for line in self.buffer.lines[clear_range].iter_mut() {
            line.clear();
        }
    }

    /// Clear tab stops
    fn clear_tabs(&mut self, _: ansi::TabulationClearMode) {
        //TODO
        warn!("Unimplemented: clear_tabs");
    }

    /// Reset terminal state
    fn reset_state(&mut self) {
        //TODO
        warn!("Unimplemented: reset_state");
    }

    /// Reverse Index
    ///
    /// Move the active position to the same horizontal position on the
    /// preceding line. If the active position is at the top margin, a scroll
    /// down is performed
    fn reverse_index(&mut self) {
        //TODO
        warn!("Unimplemented: reverse_index");
    }

    /// set a terminal attribute
    fn terminal_attribute(&mut self, attr: Attr) {
        self.with_cursor(|c| {
            match attr {
                Attr::Reset => c.set_style_modifier(StyleModifier::new()),
                Attr::Bold => {
                    c.apply_style_modifier(StyleModifier::new().bold(true));
                }
                Attr::Dim => {
                    /* What is this? */
                    warn!("Unimplemented: attr Dim")
                }
                Attr::Italic => {
                    c.apply_style_modifier(StyleModifier::new().italic(true));
                }
                Attr::Underscore => {
                    c.apply_style_modifier(StyleModifier::new().underline(true));
                }
                Attr::BlinkSlow => warn!("Unimplemented: attr BlinkSlow"),
                Attr::BlinkFast => warn!("Unimplemented: attr BlinkFast"),
                Attr::Reverse => {
                    c.apply_style_modifier(StyleModifier::new().invert(true));
                }
                Attr::Hidden => warn!("Unimplemented: attr Hidden"),
                Attr::Strike => warn!("Unimplemented: attr Strike"),
                Attr::CancelBold => {
                    c.apply_style_modifier(StyleModifier::new().bold(false));
                }
                Attr::CancelBoldDim => {
                    /*??*/
                    c.apply_style_modifier(StyleModifier::new().bold(false));
                }
                Attr::CancelItalic => {
                    c.apply_style_modifier(StyleModifier::new().italic(false));
                }
                Attr::CancelUnderline => {
                    c.apply_style_modifier(StyleModifier::new().underline(false));
                }
                Attr::CancelBlink => warn!("Unimplemented: attr CancelBlink"),
                Attr::CancelReverse => {
                    c.apply_style_modifier(StyleModifier::new().invert(false));
                }
                Attr::CancelHidden => warn!("Unimplemented: attr CancelHidden"),
                Attr::CancelStrike => warn!("Unimplemented: attr CancelStrike"),
                Attr::Foreground(color) => {
                    c.apply_style_modifier(
                        StyleModifier::new().fg_color(ansi_to_unsegen_color(color)),
                    );
                }
                Attr::Background(color) => {
                    c.apply_style_modifier(
                        StyleModifier::new().bg_color(ansi_to_unsegen_color(color)),
                    );
                }
            }
        });
    }

    /// Set mode
    fn set_mode(&mut self, mode: ansi::Mode) {
        match mode {
            ansi::Mode::ShowCursor => {
                self.show_cursor = true;
            }
            ansi::Mode::SwapScreenAndSetRestoreCursor => self.mode = BufferMode::Alternate,
            _ => {
                warn!("Unimplemented: set_mode {:?}", mode);
            }
        }
    }

    /// Unset mode
    fn unset_mode(&mut self, mode: ansi::Mode) {
        match mode {
            ansi::Mode::ShowCursor => {
                self.show_cursor = false;
            }
            ansi::Mode::SwapScreenAndSetRestoreCursor => self.mode = BufferMode::Main,
            _ => {
                warn!("Unimplemented: set_mode {:?}", mode);
            }
        }
    }

    /// DECSTBM - Set the terminal scrolling region
    fn set_scrolling_region(&mut self, _: ::std::ops::Range<index::Line>) {
        //TODO
        warn!("Unimplemented: set_scrolling_region");
    }

    /// DECKPAM - Set keypad to applications mode (ESCape instead of digits)
    fn set_keypad_application_mode(&mut self) {
        //TODO
        warn!("Unimplemented: set_keypad_application_mode");
    }

    /// DECKPNM - Set keypad to numeric mode (digits intead of ESCape seq)
    fn unset_keypad_application_mode(&mut self) {
        //TODO
        warn!("Unimplemented: unset_keypad_application_mode");
    }

    /// Set one of the graphic character sets, G0 to G3, as the active charset.
    ///
    /// 'Invoke' one of G0 to G3 in the GL area. Also refered to as shift in,
    /// shift out and locking shift depending on the set being activated
    fn set_active_charset(&mut self, _: ansi::CharsetIndex) {
        //TODO
        warn!("Unimplemented: set_active_charset");
    }

    /// Assign a graphic character set to G0, G1, G2 or G3
    ///
    /// 'Designate' a graphic character set as one of G0 to G3, so that it can
    /// later be 'invoked' by `set_active_charset`
    fn configure_charset(&mut self, _: ansi::CharsetIndex, _: ansi::StandardCharset) {
        //TODO
        warn!("Unimplemented: configure_charset");
    }

    /// Set an indexed color value
    fn set_color(&mut self, _: usize, _: ansi::Rgb) {
        //TODO: Implement this, once there is support for a per-terminal color table
        warn!("Unimplemented: set_color");
    }

    /// Run the dectest routine
    fn dectest(&mut self) {
        //TODO
        warn!("Unimplemented: dectest");
    }
}

impl TermInfo for DualWindow {
    fn lines(&self) -> index::Line {
        index::Line(self.get_height().raw_value() as usize) //TODO: is this even correct? do we want 'unbounded'?
    }
    fn cols(&self) -> index::Column {
        index::Column(self.get_width().raw_value() as usize) //TODO: see above
    }
}

impl Scrollable for DualWindow {
    fn scroll_forwards(&mut self) -> OperationResult {
        let current = self.current_scrollback_pos();
        let candidate = current + self.scroll_step;
        self.scrollback_position = if candidate < self.buffer.height_as_displayed().from_origin() {
            Some(candidate)
        } else {
            None
        };
        if self.scrollback_position.is_some() {
            Ok(())
        } else {
            Err(())
        }
    }
    fn scroll_backwards(&mut self) -> OperationResult {
        let current = self.current_scrollback_pos();
        if current > self.window_height.from_origin() {
            self.scrollback_position = Some((current - self.scroll_step).positive_or_zero());
            Ok(())
        } else {
            Err(())
        }
    }
    fn scroll_to_beginning(&mut self) -> OperationResult {
        let current = self.current_scrollback_pos();
        if current > self.window_height.from_origin() {
            self.scrollback_position = Some(self.window_height.from_origin());
            Ok(())
        } else {
            Err(())
        }
    }
    fn scroll_to_end(&mut self) -> OperationResult {
        if self.scrollback_position.is_some() {
            self.scrollback_position = None;
            Ok(())
        } else {
            Err(())
        }
    }
}
