#[derive(Debug, PartialEq)]
pub enum Input {
    Key(u8),
    Esc,
    Mouse {
        button: u32,
        col: u32,
        row: u32,
        press: bool,
    },
    FocusLost,
    Other,
}

pub const MOUSE_BUTTONS: u32 = 3;
pub const MOUSE_MOTION: u32 = 32;
pub const MOUSE_WHEEL: u32 = 64;

/// Returns the bytes used; a cut-off sequence or a trailing lone Esc is left over.
pub fn parse(buf: &[u8], out: &mut Vec<Input>) -> usize {
    let mut i = 0;
    while i < buf.len() {
        if buf[i] != 0x1b {
            out.push(Input::Key(buf[i]));
            i += 1;
            continue;
        }
        match buf.get(i + 1) {
            None => break,
            Some(b'[') => {
                let body = i + 2;
                let Some(len) = buf[body..].iter().position(|b| (0x40..=0x7e).contains(b)) else {
                    break;
                };
                let end = body + len;
                out.push(csi(&buf[body..end], buf[end]));
                i = end + 1;
            }
            Some(_) => {
                out.push(Input::Other);
                i += 2;
            }
        }
    }
    i
}

fn csi(params: &[u8], fin: u8) -> Input {
    match (params, fin) {
        ([], b'O') => Input::FocusLost,
        ([b'<', rest @ ..], b'M' | b'm') => mouse(rest, fin == b'M').unwrap_or(Input::Other),
        _ => Input::Other,
    }
}

fn mouse(params: &[u8], press: bool) -> Option<Input> {
    let mut n = std::str::from_utf8(params)
        .ok()?
        .split(';')
        .map(|p| p.parse().ok());
    let (button, col, row) = (n.next()??, n.next()??, n.next()??);
    Some(Input::Mouse {
        button,
        col,
        row,
        press,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all(buf: &[u8]) -> (Vec<Input>, usize) {
        let mut out = Vec::new();
        let used = parse(buf, &mut out);
        (out, used)
    }

    #[test]
    fn keys_mouse_and_focus() {
        let buf = b"p\x1b[<0;12;7M\x1b[<35;3;4m\x1b[Oq";
        let (inputs, used) = all(buf);
        assert_eq!(used, buf.len());
        assert_eq!(
            inputs,
            [
                Input::Key(b'p'),
                Input::Mouse {
                    button: 0,
                    col: 12,
                    row: 7,
                    press: true
                },
                Input::Mouse {
                    button: 35,
                    col: 3,
                    row: 4,
                    press: false
                },
                Input::FocusLost,
                Input::Key(b'q'),
            ]
        );
    }

    #[test]
    fn split_sequences_wait_for_the_rest() {
        assert_eq!(all(b"\x1b"), (vec![], 0));
        assert_eq!(all(b"x\x1b[<0;12"), (vec![Input::Key(b'x')], 1));
        assert_eq!(all(b"\x1b[6;17;8t").0, [Input::Other]);
    }

    #[test]
    fn malformed_mouse_reports_are_ignored() {
        assert_eq!(all(b"\x1b[<0;99999999999;7M").0, [Input::Other]);
        assert_eq!(all(b"\x1b[<0;7M").0, [Input::Other]);
    }
}
