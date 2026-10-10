use webterm_frame::{Cell, CursorShape, Frame, Rgb, flags};

use super::Terminal;

fn term(cols: u16, rows: u16) -> Terminal {
    Terminal::new(cols, rows, 100)
}

fn render(t: &mut Terminal) -> Frame {
    let mut frame = Frame::default();
    t.render(&mut frame);
    frame
}

fn row_text(frame: &Frame, row: u16) -> String {
    let text: String = (0..frame.cols).map(|col| frame.cell(col, row).ch).collect();
    text.trim_end().to_owned()
}

fn cursor_pos(frame: &Frame) -> (u16, u16) {
    let cursor = frame.cursor.expect("cursor visible");
    (cursor.col, cursor.row)
}

#[test]
fn empty_terminal_matches_frame_defaults() {
    let mut t = term(10, 3);
    assert!(t.is_dirty());
    let frame = render(&mut t);
    assert_eq!((frame.cols, frame.rows), (10, 3));
    assert_eq!(frame.cells.len(), 30);
    assert!(frame.cells.iter().all(|cell| *cell == Cell::default()));
    assert_eq!(frame.background, Cell::default().bg);
    assert_eq!(cursor_pos(&frame), (0, 0));
    assert_eq!(frame.cursor.unwrap().color, Cell::default().fg);
}

#[test]
fn plain_text_and_newlines() {
    let mut t = term(10, 3);
    t.feed(b"hello\r\nworld");
    let frame = render(&mut t);
    assert_eq!(row_text(&frame, 0), "hello");
    assert_eq!(row_text(&frame, 1), "world");
    assert_eq!(cursor_pos(&frame), (5, 1));
    assert_eq!(frame.cell(0, 0).fg, Cell::default().fg);
    assert_eq!(frame.cell(0, 0).bg, Cell::default().bg);
}

#[test]
fn long_line_wraps() {
    let mut t = term(5, 3);
    t.feed(b"abcdefgh");
    let frame = render(&mut t);
    assert_eq!(row_text(&frame, 0), "abcde");
    assert_eq!(row_text(&frame, 1), "fgh");
    assert_eq!(cursor_pos(&frame), (3, 1));
    assert!(t.line_wrapped(0));
    assert!(!t.line_wrapped(1));
}

#[test]
fn dirty_flag_tracks_render() {
    let mut t = term(10, 3);
    render(&mut t);
    assert!(!t.is_dirty());
    t.feed(b"");
    assert!(!t.is_dirty());
    t.feed(b"x");
    assert!(t.is_dirty());
    render(&mut t);
    assert!(!t.is_dirty());
    t.scroll_lines(1); // No scrollback yet, nothing moves.
    assert!(!t.is_dirty());
}

#[test]
fn sgr_16_colours() {
    let mut t = term(10, 2);
    t.feed(b"\x1b[31mr\x1b[92mg\x1b[44mb\x1b[0mn");
    let frame = render(&mut t);
    assert_eq!(frame.cell(0, 0).fg, Rgb::new(0xe0, 0x6c, 0x75));
    assert_eq!(frame.cell(1, 0).fg, Rgb::new(0xb1, 0xd9, 0x94));
    assert_eq!(frame.cell(2, 0).bg, Rgb::new(0x61, 0xaf, 0xef));
    assert_eq!(*frame.cell(3, 0), Cell { ch: 'n', ..Cell::default() });
}

#[test]
fn sgr_256_colours() {
    let mut t = term(10, 2);
    t.feed(b"\x1b[38;5;196ma\x1b[48;5;21mb\x1b[38;5;244mc\x1b[38;5;1md");
    let frame = render(&mut t);
    assert_eq!(frame.cell(0, 0).fg, Rgb::new(255, 0, 0));
    assert_eq!(frame.cell(1, 0).bg, Rgb::new(0, 0, 255));
    assert_eq!(frame.cell(2, 0).fg, Rgb::new(128, 128, 128));
    assert_eq!(frame.cell(3, 0).fg, Rgb::new(0xe0, 0x6c, 0x75));
}

#[test]
fn sgr_truecolour() {
    let mut t = term(10, 2);
    t.feed(b"\x1b[38;2;1;2;3;48;2;4;5;6mx");
    let frame = render(&mut t);
    assert_eq!(frame.cell(0, 0).fg, Rgb::new(1, 2, 3));
    assert_eq!(frame.cell(0, 0).bg, Rgb::new(4, 5, 6));
}

#[test]
fn sgr_inverse_dim_hidden() {
    let mut t = term(10, 2);
    t.feed(b"\x1b[7ma\x1b[31mb\x1b[0;2;38;2;100;200;50mc\x1b[0;8;41md\x1b[0;2;31me");
    let frame = render(&mut t);
    let default = Cell::default();
    assert_eq!((frame.cell(0, 0).fg, frame.cell(0, 0).bg), (default.bg, default.fg));
    assert_eq!(frame.cell(1, 0).bg, Rgb::new(0xe0, 0x6c, 0x75));
    assert_eq!(frame.cell(1, 0).fg, default.bg);
    assert_eq!(frame.cell(2, 0).fg, Rgb::new(66, 132, 33));
    assert_eq!(frame.cell(3, 0).ch, 'd');
    assert_eq!(frame.cell(3, 0).fg, frame.cell(3, 0).bg);
    assert_eq!(frame.cell(3, 0).bg, Rgb::new(0xe0, 0x6c, 0x75));
    assert_eq!(frame.cell(4, 0).fg, Rgb::new(147, 71, 77));
}

#[test]
fn sgr_attribute_flags() {
    let mut t = term(10, 2);
    t.feed(b"\x1b[1mb\x1b[0;3mi\x1b[0;4mu\x1b[0;9ms\x1b[0;4:3mc\x1b[0mn");
    let frame = render(&mut t);
    assert_eq!(frame.cell(0, 0).flags, flags::BOLD);
    assert_eq!(frame.cell(1, 0).flags, flags::ITALIC);
    assert_eq!(frame.cell(2, 0).flags, flags::UNDERLINE);
    assert_eq!(frame.cell(3, 0).flags, flags::STRIKEOUT);
    assert_eq!(frame.cell(4, 0).flags, flags::UNDERLINE);
    assert_eq!(frame.cell(5, 0).flags, 0);
}

#[test]
fn cursor_movement() {
    let mut t = term(10, 5);
    t.feed(b"\x1b[3;4HX");
    let frame = render(&mut t);
    assert_eq!(frame.cell(3, 2).ch, 'X');
    assert_eq!(cursor_pos(&frame), (4, 2));

    t.feed(b"\x1b[A\x1b[2D\x1b[2B\x1b[3C");
    assert_eq!(cursor_pos(&render(&mut t)), (5, 3));

    t.feed(b"\r\x1b[H\x1b[99;99H");
    assert_eq!(cursor_pos(&render(&mut t)), (9, 4));
}

#[test]
fn erase_sequences() {
    let mut t = term(10, 3);
    t.feed(b"0123456789\r\nabcdefghij\r\nklmnopqrst");

    // Erase to end of line, then to start of line.
    t.feed(b"\x1b[1;6H\x1b[K\x1b[2;3H\x1b[1K");
    let frame = render(&mut t);
    assert_eq!(row_text(&frame, 0), "01234");
    assert_eq!(row_text(&frame, 1), "   defghij");
    assert_eq!(row_text(&frame, 2), "klmnopqrst");

    // Erase below the cursor, then the whole screen.
    t.feed(b"\x1b[2;1H\x1b[J");
    let frame = render(&mut t);
    assert_eq!(row_text(&frame, 0), "01234");
    assert_eq!(row_text(&frame, 1), "");
    assert_eq!(row_text(&frame, 2), "");

    t.feed(b"\x1b[2J");
    assert!(render(&mut t).cells.iter().all(|cell| cell.ch == ' '));
}

#[test]
fn erase_uses_current_background() {
    let mut t = term(4, 1);
    t.feed(b"\x1b[48;2;9;8;7m\x1b[2K");
    let frame = render(&mut t);
    assert!(frame.cells.iter().all(|cell| cell.bg == Rgb::new(9, 8, 7)));
    // The padding colour stays the terminal background.
    assert_eq!(frame.background, Cell::default().bg);
}

#[test]
fn korean_double_width() {
    let mut t = term(10, 2);
    t.feed("가a한글".as_bytes());
    let frame = render(&mut t);
    assert_eq!(frame.cell(0, 0).ch, '가');
    assert_eq!(frame.cell(0, 0).flags, flags::WIDE);
    assert_eq!(frame.cell(1, 0).flags, flags::WIDE_SPACER);
    assert_eq!(frame.cell(1, 0).ch, ' ');
    assert_eq!(frame.cell(2, 0).ch, 'a');
    assert_eq!(frame.cell(2, 0).flags, 0);
    assert_eq!(frame.cell(3, 0).ch, '한');
    assert_eq!(frame.cell(4, 0).flags, flags::WIDE_SPACER);
    assert_eq!(frame.cell(5, 0).ch, '글');
    assert_eq!(cursor_pos(&frame), (7, 0));
    assert_eq!(t.lines_text(0, 1), ["가a한글"]);
}

#[test]
fn utf8_split_across_feeds() {
    let mut t = term(10, 1);
    let bytes = "한".as_bytes();
    t.feed(&bytes[..1]);
    t.feed(&bytes[1..]);
    let frame = render(&mut t);
    assert_eq!(frame.cell(0, 0).ch, '한');
    assert_eq!(frame.cell(0, 0).flags, flags::WIDE);
}

#[test]
fn wide_char_wraps_at_last_column() {
    let mut t = term(5, 2);
    t.feed("abcd가".as_bytes());
    let frame = render(&mut t);
    assert_eq!(row_text(&frame, 0), "abcd");
    assert_eq!(frame.cell(4, 0).flags, 0);
    assert_eq!(frame.cell(0, 1).ch, '가');
    assert_eq!(frame.cell(1, 1).flags, flags::WIDE_SPACER);
    assert_eq!(t.lines_text(0, 2), ["abcd", "가"]);
}

#[test]
fn scrollback_and_scroll_lines() {
    let mut t = term(10, 3);
    for i in 0..10 {
        t.feed(format!("line{i}\r\n").as_bytes());
    }
    assert_eq!(t.line_count(), 11);

    let frame = render(&mut t);
    assert_eq!(row_text(&frame, 0), "line8");
    assert_eq!(row_text(&frame, 1), "line9");
    assert_eq!(cursor_pos(&frame), (0, 2));

    t.scroll_lines(2);
    assert!(t.is_dirty());
    assert_eq!(t.display_offset(), 2);
    let frame = render(&mut t);
    assert_eq!(row_text(&frame, 0), "line6");
    assert_eq!(row_text(&frame, 2), "line8");
    assert_eq!(frame.cursor, None, "cursor is scrolled out of view");

    // One line back down brings the cursor row to the bottom edge.
    t.scroll_lines(-1);
    let frame = render(&mut t);
    assert_eq!(row_text(&frame, 0), "line7");
    assert_eq!(frame.cursor, None);

    t.scroll_lines(1000);
    assert_eq!(t.display_offset(), 8);
    assert_eq!(row_text(&render(&mut t), 0), "line0");

    // New output keeps the scrolled view anchored on the same lines.
    t.feed(b"more\r\n");
    assert_eq!(row_text(&render(&mut t), 0), "line0");

    t.scroll_to_bottom();
    assert_eq!(t.display_offset(), 0);
    let frame = render(&mut t);
    assert_eq!(row_text(&frame, 1), "more");
    assert_eq!(cursor_pos(&frame), (0, 2));
}

#[test]
fn cursor_visible_while_partly_scrolled() {
    let mut t = term(10, 4);
    t.feed(b"a\r\nb\r\nc\r\nd\r\ne\r\nf\x1b[2;1H");
    t.scroll_lines(1);
    assert_eq!(cursor_pos(&render(&mut t)), (0, 2));
}

#[test]
fn scrollback_limit_is_respected() {
    let mut t = Terminal::new(10, 2, 3);
    for i in 0..20 {
        t.feed(format!("{i}\r\n").as_bytes());
    }
    assert_eq!(t.line_count(), 5);
    assert_eq!(t.lines_text(0, 5), ["16", "17", "18", "19", ""]);
}

#[test]
fn lines_text_ranges() {
    let mut t = term(10, 3);
    t.feed(b"one  \r\n\ttab\r\nthree\r\nfour\r\nfive");
    assert_eq!(t.line_count(), 6);
    let all = ["one", "        ta", "b", "three", "four", "five"];
    assert_eq!(t.lines_text(0, 6), all);
    assert_eq!(t.lines_text(1, 3), all[1..3]);
    assert_eq!(t.lines_text(4, 100), all[4..], "end is clamped");
    assert!(t.lines_text(3, 3).is_empty());
    assert!(t.lines_text(9, 20).is_empty());
    assert!(t.line_wrapped(1) && !t.line_wrapped(2) && !t.line_wrapped(99));

    // Scrolling the viewport does not change line numbering.
    t.scroll_lines(2);
    assert_eq!(t.lines_text(0, 6), all);
}

#[test]
fn cursor_shape_and_visibility() {
    let mut t = term(10, 2);
    assert_eq!(render(&mut t).cursor.unwrap().shape, CursorShape::Block);
    t.feed(b"\x1b[4 q");
    assert_eq!(render(&mut t).cursor.unwrap().shape, CursorShape::Underline);
    t.feed(b"\x1b[6 q");
    assert_eq!(render(&mut t).cursor.unwrap().shape, CursorShape::Beam);
    t.feed(b"\x1b[?25l");
    assert_eq!(render(&mut t).cursor, None);
    t.feed(b"\x1b[?25h");
    assert!(render(&mut t).cursor.is_some());
}

#[test]
fn dsr_and_da_replies() {
    let mut t = term(20, 5);
    t.feed(b"\x1b[3;7H\x1b[6n");
    assert_eq!(t.take_output(), b"\x1b[3;7R");
    assert!(t.take_output().is_empty(), "take_output drains");

    t.feed(b"\x1b[5n");
    assert_eq!(t.take_output(), b"\x1b[0n");

    t.feed(b"\x1b[c");
    let reply = t.take_output();
    assert!(reply.starts_with(b"\x1b[?") && reply.ends_with(b"c"), "{reply:?}");
}

#[test]
fn colour_and_size_queries() {
    let mut t = term(20, 5);
    t.feed(b"\x1b]11;?\x07");
    assert_eq!(t.take_output(), b"\x1b]11;rgb:1010/1212/1616\x07");
    t.feed(b"\x1b[18t");
    assert_eq!(t.take_output(), b"\x1b[8;5;20t");
}

#[test]
fn osc_palette_override() {
    let mut t = term(10, 1);
    t.feed(b"\x1b]4;1;rgb:11/22/33\x07\x1b]11;#040506\x07\x1b[31mx");
    let frame = render(&mut t);
    assert_eq!(frame.cell(0, 0).fg, Rgb::new(0x11, 0x22, 0x33));
    assert_eq!(frame.cell(0, 0).bg, Rgb::new(4, 5, 6));
    assert_eq!(frame.background, Rgb::new(4, 5, 6));
}

#[test]
fn resize_reflows_and_resizes_frame() {
    let mut t = term(10, 3);
    t.feed(b"abcdefghij\r\nxyz");
    let mut frame = Frame::default();
    t.render(&mut frame);

    // Narrowing rewraps the long line; its first half moves into scrollback.
    t.resize(5, 2);
    assert!(t.is_dirty());
    assert_eq!((t.cols(), t.rows()), (5, 2));
    t.render(&mut frame);
    assert_eq!((frame.cols, frame.rows), (5, 2));
    assert_eq!(frame.cells.len(), 10);
    assert_eq!(row_text(&frame, 0), "fghij");
    assert_eq!(row_text(&frame, 1), "xyz");
    assert_eq!(cursor_pos(&frame), (3, 1));
    assert_eq!(t.lines_text(0, t.line_count()), ["abcde", "fghij", "xyz"]);
    assert!(t.line_wrapped(0));

    // Widening joins it again.
    t.resize(10, 3);
    t.render(&mut frame);
    assert_eq!((frame.cols, frame.rows), (10, 3));
    assert_eq!(row_text(&frame, 0), "abcdefghij");
    assert_eq!(row_text(&frame, 1), "xyz");
    assert_eq!(cursor_pos(&frame), (3, 1));

    // Same size is a no-op; degenerate sizes are clamped instead of panicking.
    t.resize(10, 3);
    assert!(!t.is_dirty());
    t.resize(0, 0);
    assert_eq!((t.cols(), t.rows()), (2, 1));
    t.render(&mut frame);
    assert_eq!(frame.cells.len(), 2);
}

#[test]
fn shrinking_rows_moves_lines_to_scrollback() {
    let mut t = term(10, 4);
    t.feed(b"a\r\nb\r\nc\r\nd");
    t.resize(10, 2);
    let frame = render(&mut t);
    assert_eq!(row_text(&frame, 0), "c");
    assert_eq!(row_text(&frame, 1), "d");
    assert_eq!(t.lines_text(0, t.line_count()), ["a", "b", "c", "d"]);
}

#[test]
fn input_modes() {
    let mut t = term(10, 2);
    assert!(!t.app_cursor_keys() && !t.bracketed_paste() && !t.app_keypad());
    t.feed(b"\x1b[?1h\x1b[?2004h\x1b=");
    assert!(t.app_cursor_keys() && t.bracketed_paste() && t.app_keypad());
    t.feed(b"\x1b[?1l\x1b[?2004l\x1b>");
    assert!(!t.app_cursor_keys() && !t.bracketed_paste() && !t.app_keypad());
}

#[test]
fn alt_screen_round_trip() {
    let mut t = term(10, 2);
    t.feed(b"main\x1b[?1049h\x1b[HALT");
    assert!(t.alt_screen());
    assert_eq!(row_text(&render(&mut t), 0), "ALT");
    t.feed(b"\x1b[?1049l");
    assert!(!t.alt_screen());
    assert_eq!(row_text(&render(&mut t), 0), "main");
}

#[test]
fn title_and_bell() {
    let mut t = term(10, 2);
    assert_eq!(t.title(), None);
    t.feed(b"\x1b]0;ttyUSB0\x07");
    assert_eq!(t.title(), Some("ttyUSB0"));
    assert!(!t.take_bell(), "an OSC terminator is not a bell");
    t.feed(b"\x07");
    assert!(t.take_bell());
    assert!(!t.take_bell());
}

#[test]
fn synchronized_update_is_not_buffered() {
    let mut t = term(10, 2);
    t.feed(b"\x1b[?2026hshown");
    assert_eq!(row_text(&render(&mut t), 0), "shown");
    t.feed(b"\x1b[?2026l!");
    assert_eq!(row_text(&render(&mut t), 0), "shown!");
}
