//! 单行输入框的编辑状态。绘制在 gfx.rs，这里只管文字、光标和选区。
use windows::Win32::{
    Foundation::{HANDLE, HGLOBAL, HWND},
    System::{
        DataExchange::{CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, SetClipboardData},
        Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock},
    },
};

const CF_UNICODETEXT: u32 = 13;

#[derive(Default)]
pub struct TextField {
    /// UTF-16，与 DirectWrite 的文字位置一致
    pub text: Vec<u16>,
    pub caret: usize,
    /// 选区的另一端；等于 caret 表示没有选区
    pub anchor: usize,
    pub scroll: f32,
    /// 上次绘制时文字起点和光标的横坐标、字号，供点击定位和输入法定位
    pub origin: f32,
    pub caret_x: f32,
    pub size: f32,
}

fn is_low(u: u16) -> bool {
    (0xDC00..0xE000).contains(&u)
}

impl TextField {
    pub fn string(&self) -> String {
        String::from_utf16_lossy(&self.text)
    }

    pub fn set(&mut self, s: &str) {
        self.text = s.encode_utf16().collect();
        self.caret = self.text.len();
        self.anchor = self.caret;
    }

    pub fn clear(&mut self) {
        self.set("");
        self.scroll = 0.0;
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn at_end(&self) -> bool {
        self.caret == self.text.len() && self.anchor == self.caret
    }

    pub fn selection(&self) -> Option<(usize, usize)> {
        (self.caret != self.anchor).then(|| (self.caret.min(self.anchor), self.caret.max(self.anchor)))
    }

    fn erase_selection(&mut self) -> bool {
        let Some((a, b)) = self.selection() else { return false };
        self.text.drain(a..b);
        self.caret = a;
        self.anchor = a;
        true
    }

    pub fn insert(&mut self, units: &[u16]) {
        self.erase_selection();
        // 单行：换行和制表符变成空格
        let clean: Vec<u16> = units.iter().map(|&u| if u < 32 { 32 } else { u }).collect();
        self.text.splice(self.caret..self.caret, clean.iter().copied());
        self.caret += clean.len();
        self.anchor = self.caret;
    }

    fn prev(&self, i: usize) -> usize {
        match i {
            0 => 0,
            _ if i >= 2 && is_low(self.text[i - 1]) => i - 2,
            _ => i - 1,
        }
    }

    fn next(&self, i: usize) -> usize {
        let n = self.text.len();
        if i >= n {
            n
        } else if i + 1 < n && is_low(self.text[i + 1]) {
            i + 2
        } else {
            i + 1
        }
    }

    fn word_start(&self, mut i: usize) -> usize {
        let space = |u: u16| u == 32;
        while i > 0 && space(self.text[i - 1]) {
            i -= 1;
        }
        while i > 0 && !space(self.text[i - 1]) {
            i -= 1;
        }
        i
    }

    pub fn move_to(&mut self, i: usize, select: bool) {
        self.caret = i.min(self.text.len());
        if !select {
            self.anchor = self.caret;
        }
    }

    /// 处理编辑键。返回 (是否已处理, 文字是否变化)。
    pub fn key(&mut self, vk: u32, ctrl: bool, shift: bool, hwnd: HWND) -> (bool, bool) {
        let before = self.text.len();
        match vk {
            0x08 => {
                // Backspace
                if !self.erase_selection() && self.caret > 0 {
                    let from = if ctrl { self.word_start(self.caret) } else { self.prev(self.caret) };
                    self.text.drain(from..self.caret);
                    self.move_to(from, false);
                }
            }
            0x2E => {
                // Delete
                if !self.erase_selection() && self.caret < self.text.len() {
                    let to = self.next(self.caret);
                    self.text.drain(self.caret..to);
                }
            }
            0x25 => match self.selection() {
                Some((a, _)) if !shift => self.move_to(a, false),
                _ => self.move_to(if ctrl { self.word_start(self.caret) } else { self.prev(self.caret) }, shift),
            },
            0x27 => match self.selection() {
                Some((_, b)) if !shift => self.move_to(b, false),
                _ => self.move_to(self.next(self.caret), shift),
            },
            0x24 => self.move_to(0, shift),
            0x23 => self.move_to(self.text.len(), shift),
            0x41 if ctrl => {
                self.anchor = 0;
                self.caret = self.text.len();
            }
            0x43 if ctrl => self.copy(hwnd),
            0x58 if ctrl => {
                self.copy(hwnd);
                self.erase_selection();
            }
            0x56 if ctrl => {
                if let Some(text) = clipboard_text(hwnd) {
                    self.insert(&text);
                }
            }
            _ => return (false, false),
        }
        (true, self.text.len() != before || matches!(vk, 0x56 | 0x58 | 0x08 | 0x2E))
    }

    fn copy(&self, hwnd: HWND) {
        let Some((a, b)) = self.selection() else { return };
        let mut units = self.text[a..b].to_vec();
        units.push(0);
        unsafe {
            if OpenClipboard(Some(hwnd)).is_err() {
                return;
            }
            let _ = EmptyClipboard();
            if let Ok(mem) = GlobalAlloc(GMEM_MOVEABLE, units.len() * 2) {
                let p = GlobalLock(mem) as *mut u16;
                if !p.is_null() {
                    std::ptr::copy_nonoverlapping(units.as_ptr(), p, units.len());
                    let _ = GlobalUnlock(mem);
                    let _ = SetClipboardData(CF_UNICODETEXT, Some(HANDLE(mem.0)));
                }
            }
            let _ = CloseClipboard();
        }
    }
}

fn clipboard_text(hwnd: HWND) -> Option<Vec<u16>> {
    unsafe {
        OpenClipboard(Some(hwnd)).ok()?;
        let mut out = None;
        if let Ok(h) = GetClipboardData(CF_UNICODETEXT) {
            let mem = HGLOBAL(h.0);
            let p = GlobalLock(mem) as *const u16;
            if !p.is_null() {
                let mut len = 0;
                while *p.add(len) != 0 {
                    len += 1;
                }
                out = Some(std::slice::from_raw_parts(p, len).to_vec());
                let _ = GlobalUnlock(mem);
            }
        }
        let _ = CloseClipboard();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editing_keeps_caret_and_selection_consistent() {
        let hwnd = HWND::default();
        let mut tf = TextField::default();
        tf.insert(&"ab终端".encode_utf16().collect::<Vec<_>>());
        assert!(tf.at_end());
        tf.key(0x25, false, true, hwnd); // Shift+Left 选中"端"
        assert_eq!(tf.selection(), Some((3, 4)));
        tf.insert(&[b'x' as u16]);
        assert_eq!(tf.string(), "ab终x");
        tf.key(0x24, false, false, hwnd);
        tf.key(0x2E, false, false, hwnd);
        assert_eq!(tf.string(), "b终x");
        tf.set("foo bar");
        tf.key(0x08, true, false, hwnd); // Ctrl+Backspace 删一个词
        assert_eq!(tf.string(), "foo ");
    }
}
