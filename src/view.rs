//! 绘制：搜索面板与设置。每帧整幅重画，同时登记可点击区域。
use crate::{
    app::*,
    config,
    gfx::{Align, Color, Gfx},
    index::Kind,
    textfield::TextField,
};
use std::time::Instant;

/// 没有图标时的占位色块，按名称固定取一个颜色。
const TILES: [u32; 10] =
    [0x1F6FB5, 0x4B5563, 0x2B7A4B, 0x2F7F99, 0xB4500B, 0x5B4B8A, 0x1E8E3E, 0xC62F2F, 0x7A3FB0, 0x1A56C4];
const TOOL_TILE: u32 = 0x56637A;

impl App {
    fn hot(&mut self, x: f32, y: f32, w: f32, h: f32, act: Act, row: Option<usize>) {
        self.hots.push(Hot { x, y, w, h, act, row });
    }

    pub fn draw(&mut self, g: &mut Gfx) {
        let w = self.width();
        let h = self.anims.peek(ID_H).max(BAR);
        let open = self.anims.get(ID_OPEN, 1.0, 120.0);
        let m = self.anims.get(ID_MODE, if self.mode == Mode::Settings { 1.0 } else { 0.0 }, 140.0);
        let fade = self.anims.get(ID_FADE, 1.0, 140.0);
        self.hots.clear();

        g.fill(0.0, 0.0, w, h + 2.0, 0.0, self.theme.tint);

        // 输入栏与设置标题在同一位置交叉淡入淡出
        if m < 1.0 {
            g.layer(open * (1.0 - m), -8.0 * m, 0.0);
            self.search_head(g, w, self.mode == Mode::Search);
        }
        if m > 0.0 {
            g.layer(open * m, 8.0 * (1.0 - m), 0.0);
            self.settings_head(g, w, self.mode == Mode::Settings);
        }

        g.layer(1.0, 0.0, 0.0);
        g.clip(0.0, BAR, w, (h - BAR).max(0.0));
        g.layer(open * fade, 0.0, 4.0 * (1.0 - fade));
        match self.mode {
            Mode::Search => self.search_body(g, w),
            Mode::Settings => self.settings_body(g, w),
        }
        g.layer(1.0, 0.0, 0.0);
        g.unclip();

        // 拖着文件夹经过时，整块面板描一圈强调色
        let ring = self.anims.get(ID_RING, self.dropping as u32 as f32, 120.0);
        if ring > 0.0 {
            g.outline(1.0, 1.0, w - 2.0, h - 2.0, self.radius() - 1.0, 2.0, self.theme.accent.a(ring));
        }
    }

    // ---------- 搜索 ----------

    fn search_head(&mut self, g: &mut Gfx, w: f32, live: bool) {
        let th = &self.theme;
        let (t1, t3, well, accent) = (th.t1, th.t3, th.well, th.accent);
        g.ring(28.75, 26.75, 5.75, 1.5, t3);
        g.path(&[(33.0, 31.0), (37.0, 35.0)], false, 1.5, t3);

        let bx = w - 12.0 - 32.0;
        if self.hover == Act::OpenSettings {
            g.circle(bx + 16.0, 28.0, 16.0, well);
        }
        sliders(g, bx + 7.0, 19.0, self.theme.t2);

        // 索引更新的结果在输入栏右侧短暂出现
        let flash = self.anims.get(ID_FLASH, !self.flash.is_empty() as u32 as f32, 120.0);
        let mut right = bx - 8.0;
        if !self.flash.is_empty() {
            // 刚隐藏了一个程序：旁边提示可以撤销
            if !self.last_hidden.is_empty() {
                let kw = g.measure("Ctrl Z", 12.0) + 12.0;
                g.fill(right - kw, 17.0, kw, 22.0, 5.0, self.theme.kbd.a(flash));
                g.text("Ctrl Z", right - kw, 17.0, kw, 22.0, 12.0, self.theme.t2.a(flash), Align::Center);
                right -= kw + 10.0;
            }
            let fw = g.measure(&self.flash, 12.0);
            g.text(&self.flash, right - fw, 0.0, fw + 1.0, BAR, 12.0, t3.a(flash), Align::Left);
            right -= fw + 12.0;
        }

        let (x, fw) = (54.0, right - 54.0);
        if self.query.is_empty() {
            g.text(&self.s.placeholder, x, 0.0, fw, BAR, 20.0, t3, Align::Left);
        }
        let caret = (live && self.caret_on).then_some(accent);
        let mut q = std::mem::take(&mut self.query);
        g.field(&mut q, x, 0.0, fw, BAR, 20.0, t1, accent.a(0.35), caret);
        self.query = q;
        // 路径模式：光标后淡色显示选中项的补全，Tab 接受
        if live
            && self.query.at_end()
            && self.query.selection().is_none()
            && let Some(rest) = self.path_item().and_then(|p| p.ghost(&self.query.string()))
        {
            let kw = g.measure("Tab", 12.0) + 12.0;
            let gx = self.query.caret_x + 1.0;
            g.text(&rest, gx, 0.0, (x + fw - kw - 12.0 - gx).max(0.0), BAR, 20.0, t3, Align::Left);
            g.fill(x + fw - kw, 17.0, kw, 22.0, 5.0, self.theme.kbd);
            g.text("Tab", x + fw - kw, 17.0, kw, 22.0, 12.0, self.theme.t2, Align::Center);
        }

        if live {
            self.hot(0.0, 0.0, w, BAR, Act::Bar, None);
            self.hot(x, 8.0, fw, BAR - 16.0, Act::QueryField, None);
            self.hot(bx, 12.0, 32.0, 32.0, Act::OpenSettings, None);
        }
    }

    fn search_body(&mut self, g: &mut Gfx, w: f32) {
        let (t1, t2, t3, accent, kbd, danger, sel_c) = {
            let th = &self.theme;
            (th.t1, th.t2, th.t3, th.accent, th.kbd, th.danger, th.sel)
        };
        if self.rows.is_empty() {
            let Some(note) = self.note().map(str::to_string) else { return };
            let cy = BAR + ROW / 2.0;
            if self.index.groups.is_empty() && self.scanning {
                // 第一次运行：一个呼吸的圆点、一句话、一个跳动的数字
                let phase = self.pulse.elapsed().as_secs_f32() / 2.4 * std::f32::consts::TAU;
                g.circle(34.0, cy, 4.0, accent.a(if self.anims.off { 1.0 } else { 0.675 - 0.325 * phase.cos() }));
                g.text(&note, 60.0, BAR, w - 160.0, ROW, 15.0, t2, Align::Left);
                if self.progress > 0 {
                    g.text(&self.progress.to_string(), w - 100.0, BAR, 80.0, ROW, 12.0, t3, Align::Right);
                }
            } else if self.can_add_dir() {
                // 搜不到之后的下一步：回车去添加目录
                g.fill(PAD, BAR, w - 2.0 * PAD, ROW, self.radius(), sel_c);
                let mut right = w - PAD - 12.0;
                g.fill(right - 24.0, cy - 11.0, 24.0, 22.0, 5.0, kbd);
                g.text("↵", right - 24.0, cy - 11.0, 24.0, 22.0, 12.0, t2, Align::Center);
                right -= 34.0;
                let label = self.s.add_dir.clone();
                let bw = g.measure(&label, 13.0) + 50.0;
                let hovered = self.hover == Act::AddDirHint;
                g.fill(right - bw, cy - 17.0, bw, 34.0, 10.0, if hovered { self.theme.well2 } else { self.theme.well });
                g.path(&[(right - bw + 17.0, cy), (right - bw + 25.0, cy)], false, 1.5, t1);
                g.path(&[(right - bw + 21.0, cy - 4.0), (right - bw + 21.0, cy + 4.0)], false, 1.5, t1);
                g.text(&label, right - bw + 36.0, cy - 17.0, bw - 50.0 + 1.0, 34.0, 13.0, t1, Align::Left);
                self.hot(right - bw, cy - 17.0, bw, 34.0, Act::AddDirHint, None);
                g.text(&note, PAD + 14.0, BAR, right - bw - 40.0, ROW, 15.0, t2, Align::Left);
            } else {
                g.text(&note, PAD + 14.0, BAR, w - 40.0, ROW, 15.0, t2, Align::Left);
            }
            return;
        }

        // 选中条在行与行之间滑动；启动的一瞬间轻压一下
        let sy = BAR + self.anims.get(ID_SEL, self.sel as f32 * ROW, 110.0);
        let press = self.anims.get(ID_PRESS, self.pressed.is_some() as u32 as f32, 110.0);
        let inset = (w - 2.0 * PAD) * 0.0075 * press;
        g.fill(PAD + inset, sy + inset * 0.3, w - 2.0 * PAD - 2.0 * inset, ROW - inset * 0.6, self.radius(), sel_c.mix(kbd, press));
        g.fill(PAD + inset, sy + 16.0, 3.0, 16.0, 1.5, accent);

        let digits = self.anims.get(ID_DIGITS, self.digits_on() as u32 as f32, 80.0);
        let admin = self.admin_on();
        let px = self.icon_px();
        for i in 0..self.rows.len() {
            let row = &self.rows[i];
            let group = if row.path { &self.paths[row.group] } else { &self.index.groups[row.group] };
            let entry = &group.members[row.member];
            let y = BAR + i as f32 * ROW;
            let dim = if row.variant && !row.current { 0.55 } else { 1.0 };

            if row.path && self.path_items.get(row.group).is_some_and(|p| p.dir) {
                // 路径模式里的文件夹：淡底上画一个文件夹
                g.fill(20.0, y + 10.0, 28.0, 28.0, 7.0, self.theme.well);
                let (x0, y0) = (26.0, y + 17.0);
                let folder = [(x0, y0), (x0 + 5.0, y0), (x0 + 6.6, y0 + 1.6), (x0 + 16.0, y0 + 1.6), (x0 + 16.0, y0 + 14.0), (x0, y0 + 14.0)];
                g.path(&folder, true, 1.3, t2);
            } else if !g.icon(entry.icon_key(), px, 20.0, y + 10.0, 28.0, dim) {
                self.icon_wanted.push(entry.target.clone());
                let first = entry.name.chars().next().unwrap_or(' ');
                let tile = if entry.kind == Kind::Dir { TOOL_TILE } else { TILES[first as usize % TILES.len()] };
                g.fill(20.0, y + 10.0, 28.0, 28.0, 7.0, Color::rgb(tile).a(dim));
                g.text(&first.to_string(), 20.0, y + 10.0, 28.0, 28.0, 13.0, Color::white(dim), Align::Center);
            }

            // 右侧从右往左排：按键提示、来源数、标记、来源说明
            let mut right = w - PAD - 12.0;
            let key = if admin && i == self.sel {
                Some((self.s.admin_enter.clone(), 1.0))
            } else if digits > 0.0 && !admin && i < 8 {
                Some(((i + 1).to_string(), if i == self.sel { 1.0 } else { digits }))
            } else if i == self.sel {
                Some(("↵".to_string(), 1.0))
            } else {
                None
            };
            if let Some((text, alpha)) = key {
                let kw = (g.measure(&text, 12.0) + 12.0).max(24.0);
                g.fill(right - kw, y + 13.0, kw, 22.0, 5.0, kbd.a(alpha));
                g.text(&text, right - kw, y + 13.0, kw, 22.0, 12.0, t2.a(alpha), Align::Center);
                right -= kw + 12.0;
            }
            if row.stack > 1 {
                let n = row.stack.to_string();
                let pw = g.measure(&n, 11.0) + 25.0;
                g.fill(right - pw, y + 14.0, pw, 20.0, 10.0, kbd);
                g.text(&n, right - pw + 8.0, y + 14.0, pw, 20.0, 11.0, t2, Align::Left);
                let cx = right - 14.0;
                g.path(&[(cx + 3.0, y + 21.5), (cx + 5.5, y + 24.0), (cx + 3.0, y + 26.5)], false, 1.2, t2);
                right -= pw + 12.0;
            }
            let tag = if self.fresh.contains(&entry.target) {
                &self.s.tag_new
            } else if row.variant && row.current {
                &self.s.tag_current
            } else {
                ""
            };
            if !tag.is_empty() {
                let tw = g.measure(tag, 12.0);
                g.text(tag, right - tw, y, tw + 1.0, ROW, 12.0, accent, Align::Left);
                right -= tw + 12.0;
            }

            let error = self.error.as_ref().filter(|(t, _)| *t == entry.target).map(|(_, m)| m.as_str());
            let side = match error {
                Some(m) => m,
                None if entry.kind == Kind::Dir || group.members.len() > 1 => source_label(&self.s, entry),
                None => "",
            };
            // 目录里扫出来的文件带上后缀，淡一档紧跟在名称后面。
            // 显示名不是文件名时（取回了程序名的快捷方式）不带，否则拼出一个不存在的文件名
            let file = entry.target.rsplit(['\\', '/']).next().unwrap_or("");
            let ext = match file.rsplit_once('.') {
                Some((stem, ext)) if entry.kind == Kind::Dir && stem.eq_ignore_ascii_case(&entry.name) => ext,
                _ => "",
            };
            let ext = if ext.is_empty() { String::new() } else { format!(".{}", ext.to_lowercase()) };
            let ext_w = if ext.is_empty() { 0.0 } else { g.measure(&ext, 15.0) + 1.0 };
            let name_w = g.measure(&entry.name, 15.0).min(right - 60.0 - ext_w) + 1.0;
            g.text_marked(&entry.name, 60.0, y, name_w, ROW, 15.0, t1, Align::Left, &row.marks, accent);
            g.text(&ext, 60.0 + name_w - 1.0, y, ext_w, ROW, 15.0, t3, Align::Left);
            let side_x = 60.0 + name_w + ext_w + 12.0;
            if error.is_some() {
                g.text(side, side_x, y, right - side_x, ROW, 13.0, danger, Align::Right);
            } else {
                g.text(side, side_x, y, right - side_x, ROW, 12.0, t3, Align::Right);
            }
            self.hot(PAD, y, w - 2.0 * PAD, ROW, Act::Row(i), Some(i));
        }
    }

    // ---------- 设置 ----------

    fn settings_head(&mut self, g: &mut Gfx, w: f32, live: bool) {
        let (t1, t2, well) = (self.theme.t1, self.theme.t2, self.theme.well);
        if self.hover == Act::Back {
            g.circle(28.0, 28.0, 16.0, well);
        }
        g.path(&[(30.0, 23.0), (25.0, 28.0), (30.0, 33.0)], false, 1.5, t2);
        let tabs_w = 288.0f32.min(w - 54.0 - 96.0);
        let tx = w - 12.0 - tabs_w;
        let title = self.s.settings.clone();
        g.text(&title, 54.0, 0.0, tx - 54.0 - 12.0, BAR, 20.0, t1, Align::Left);
        let labels = [self.s.tab_general.clone(), self.s.tab_look.clone(), self.s.tab_dirs.clone()];
        self.seg(g, ID_TABS, tx, 11.0, tabs_w, &labels, self.tab, live.then_some(None), |i| Act::Tab(i));
        if live {
            self.hot(0.0, 0.0, tx, BAR, Act::Bar, None);
            self.hot(12.0, 12.0, 32.0, 32.0, Act::Back, None);
        }
    }

    /// 分段选择：选中的那一段底下有一块会滑动的凸起。
    #[allow(clippy::too_many_arguments)]
    fn seg(
        &mut self,
        g: &mut Gfx,
        id: u32,
        x: f32,
        y: f32,
        w: f32,
        labels: &[String],
        current: usize,
        row: Option<Option<usize>>,
        act: impl Fn(usize) -> Act,
    ) {
        let (t1, t2, well, raise) = (self.theme.t1, self.theme.t2, self.theme.well, self.theme.raise);
        let each = (w - 6.0) / labels.len() as f32;
        let at = self.anims.get(id, current as f32, 180.0);
        g.fill(x, y, w, 34.0, 10.0, well);
        g.fill(x + 3.0 + at * each, y + 3.0, each, 28.0, 7.0, raise);
        for (i, label) in labels.iter().enumerate() {
            let lx = x + 3.0 + i as f32 * each;
            g.text(label, lx + 4.0, y, each - 8.0, 34.0, 13.0, if i == current { t1 } else { t2 }, Align::Center);
            if let Some(row) = row {
                self.hot(lx, y, each, 34.0, act(i), row);
            }
        }
    }

    fn switch(&mut self, g: &mut Gfx, id: u32, right: f32, cy: f32, on: bool) {
        let th = &self.theme;
        let (well2, accent, t2, on_accent) = (th.well2, th.accent, th.t2, th.on_accent);
        let t = self.anims.get(id, on as u32 as f32, 160.0);
        let x = right - 40.0;
        g.fill(x, cy - 12.0, 40.0, 24.0, 12.0, well2.mix(accent, t));
        g.circle(x + 12.0 + 16.0 * t, cy, 9.0, t2.mix(on_accent, t));
    }

    /// 圆形图标按钮的底；返回左上角，图标由调用方画。
    fn round_button(&mut self, g: &mut Gfx, right: f32, cy: f32, act: Act, row: usize, ghost: bool) -> f32 {
        let x = right - 32.0;
        let hovered = self.hover == act;
        if !ghost || hovered {
            g.circle(x + 16.0, cy, 16.0, if hovered { self.theme.well2 } else { self.theme.well });
        }
        self.hot(x, cy - 16.0, 32.0, 32.0, act, Some(row));
        x
    }

    fn settings_body(&mut self, g: &mut Gfx, w: f32) {
        let (t1, t2, t3, accent, well, well2, danger, on_accent, sel_c) = {
            let th = &self.theme;
            (th.t1, th.t2, th.t3, th.accent, th.well, th.well2, th.danger, th.on_accent, th.sel)
        };
        let rows = self.srows();
        let sel = self.set_sel.min(rows.len() - 1);
        let sy = BAR + self.anims.get(ID_SET_SEL, sel as f32 * ROW, 110.0);
        g.fill(PAD, sy, w - 2.0 * PAD, ROW, self.radius(), sel_c.a(0.8));

        let right = w - PAD - 8.0;
        for (i, &row) in rows.iter().enumerate() {
            let y = BAR + i as f32 * ROW;
            let cy = y + ROW / 2.0;
            let id = ID_ROW + i as u32 + self.tab as u32 * 32;
            self.hot(PAD, y, w - 2.0 * PAD, ROW, Act::None, Some(i));
            // 控件左边界；标签占到这里为止
            let mut left = right;
            let mut label = String::new();
            let mut label_color = t1;
            let mut label_x = PAD + 14.0;

            match row {
                SRow::Lang | SRow::Theme | SRow::Corners | SRow::Screen => {
                    let (name, labels, design_w) = match row {
                        SRow::Lang => (&self.s.language, vec![self.s.lang_zh.clone(), self.s.lang_en.clone()], 176.0),
                        SRow::Theme => (
                            &self.s.theme,
                            vec![self.s.theme_system.clone(), self.s.theme_light.clone(), self.s.theme_dark.clone()],
                            264.0,
                        ),
                        SRow::Corners => (
                            &self.s.corners,
                            vec![self.s.corner_square.clone(), self.s.corner_small.clone(), self.s.corner_round.clone()],
                            264.0,
                        ),
                        _ => (&self.s.screen, vec![self.s.screen_pointer.clone(), self.s.screen_primary.clone()], 304.0f32),
                    };
                    label = name.clone();
                    let sw = design_w.min(w * 0.56);
                    left = right - sw;
                    self.seg(g, id, left, cy - 17.0, sw, &labels, self.seg_value(row), Some(Some(i)), |j| Act::Seg(row, j));
                }
                SRow::Ime | SRow::Autostart | SRow::Admin | SRow::Reduce => {
                    label = match row {
                        SRow::Ime => &self.s.ime,
                        SRow::Autostart => &self.s.autostart,
                        SRow::Admin => &self.s.admin,
                        _ => &self.s.reduce,
                    }
                    .clone();
                    self.switch(g, id, right, cy, self.switch_value(row));
                    left = right - 48.0;
                    self.hot(left, cy - 22.0, 48.0, 44.0, Act::Toggle(row), Some(i));
                }
                SRow::Hotkey | SRow::Position => {
                    let (name, text, color, note, note_color) = match row {
                        SRow::Hotkey if self.recording => (&self.s.hotkey, self.s.recording.clone(), accent, "", t3),
                        SRow::Hotkey => {
                            let note = if self.hotkey_note.is_some() { self.s.hotkey_taken.as_str() } else { "" };
                            (&self.s.hotkey, self.cfg.hotkey.replace('+', " + "), t1, note, danger)
                        }
                        _ => (&self.s.position, self.s.reset.clone(), t1, "", t3),
                    };
                    label = name.clone();
                    let note = if row == SRow::Position {
                        if self.centered() {
                            self.s.centered.clone()
                        } else {
                            format!("{:.0}% · {:.0}%", self.cfg.cx * 100.0, self.cfg.ty * 100.0)
                        }
                    } else {
                        note.to_string()
                    };
                    let bw = g.measure(&text, 13.0) + 28.0;
                    left = right - bw;
                    let act = Act::Button(row);
                    g.fill(left, cy - 17.0, bw, 34.0, 10.0, if self.hover == act { well2 } else { well });
                    g.text(&text, left, cy - 17.0, bw, 34.0, 13.0, color, Align::Center);
                    self.hot(left, cy - 17.0, bw, 34.0, act, Some(i));
                    if !note.is_empty() {
                        let nw = g.measure(&note, 12.0);
                        left -= nw + 10.0;
                        g.text(&note, left, y, nw + 1.0, ROW, 12.0, note_color, Align::Left);
                    }
                }
                SRow::Size => {
                    label = self.s.size.clone();
                    let v = self.scale_draft.unwrap_or(self.cfg.scale);
                    let t = (v - config::SCALE_MIN) as f32 / (config::SCALE_MAX - config::SCALE_MIN) as f32;
                    left = right - 160.0;
                    g.fill(left, cy - 3.0, 160.0, 6.0, 3.0, well2);
                    let grab = self.scale_draft.is_some() as u32 as f32;
                    let r = 9.0 + self.anims.get(id, grab, 100.0);
                    g.circle(left + 9.0 + t * 142.0, cy, r, t1);
                    self.hot(left, cy - 16.0, 160.0, 32.0, Act::Slider, Some(i));
                    let note = format!("{v}%");
                    let nw = g.measure("100%", 12.0);
                    left -= nw + 10.0;
                    g.text(&note, left, y, nw + 1.0, ROW, 12.0, t3, Align::Right);
                }
                SRow::MaxRows => {
                    label = self.s.max_rows.clone();
                    let x = self.round_button(g, right, cy, Act::Step(1), i, false);
                    g.path(&[(x + 12.0, cy), (x + 20.0, cy)], false, 1.5, t1);
                    g.path(&[(x + 16.0, cy - 4.0), (x + 16.0, cy + 4.0)], false, 1.5, t1);
                    let value = format!("{}{}", self.cfg.max_rows, self.s.rows_unit);
                    g.text(&value, x - 62.0, y, 52.0, ROW, 14.0, t1, Align::Center);
                    let x = self.round_button(g, x - 72.0, cy, Act::Step(-1), i, false);
                    g.path(&[(x + 12.0, cy), (x + 20.0, cy)], false, 1.5, t1);
                    left = x;
                }
                SRow::Folder(n) => {
                    label = self.cfg.dirs[n].clone();
                    label_x = 60.0;
                    // 新加的目录淡入并落位
                    let arrive = self.anims.get(ID_ARRIVE + n as u32, 1.0, 160.0);
                    g.layer(arrive, 0.0, -4.0 * (1.0 - arrive));
                    g.fill(20.0, y + 10.0, 28.0, 28.0, 7.0, well);
                    folder(g, 26.0, y + 16.0, t2);
                    // 删除按钮只在选中的那一行出现
                    if i == sel {
                        let x = self.round_button(g, right, cy, Act::Remove(n), i, true);
                        g.path(&[(x + 12.0, cy - 4.0), (x + 20.0, cy + 4.0)], false, 1.5, t2);
                        g.path(&[(x + 20.0, cy - 4.0), (x + 12.0, cy + 4.0)], false, 1.5, t2);
                    }
                    left = right - 42.0;
                    let count = self.index.count_in(&self.cfg.dirs[n]).to_string();
                    let nw = g.measure(&count, 12.0).max(28.0);
                    left -= nw;
                    g.text(&count, left, y, nw + 1.0, ROW, 12.0, t3, Align::Right);
                    let lw = left - 16.0 - label_x;
                    g.text(&label, label_x, y, lw, ROW, 15.0, t1, Align::Left);
                    g.layer(1.0, 0.0, 0.0);
                    label.clear();
                }
                SRow::Add => {
                    // 加号打开系统的文件夹选择框；旁边可以直接粘贴路径
                    let drop = self.anims.get(id, self.dropping as u32 as f32, 120.0);
                    let hovered = self.hover == Act::Plus;
                    g.fill(20.0, y + 10.0, 28.0, 28.0, 7.0, if hovered { well2 } else { well }.mix(accent, drop));
                    let pc = t2.mix(on_accent, drop);
                    g.path(&[(30.0, cy), (38.0, cy)], false, 1.5, pc);
                    g.path(&[(34.0, cy - 4.0), (34.0, cy + 4.0)], false, 1.5, pc);
                    self.hot(20.0, y + 10.0, 28.0, 28.0, Act::Plus, Some(i));
                    let fw = right - 60.0;
                    if self.path.is_empty() {
                        let hint = if self.dropping { &self.s.drop_here } else { &self.s.path_hint };
                        g.text(hint, 60.0, y, fw, ROW, 15.0, t3, Align::Left);
                    }
                    let caret = (i == sel && self.caret_on).then_some(accent);
                    let mut tf = std::mem::take(&mut self.path);
                    g.field(&mut tf, 60.0, y, fw, ROW, 15.0, t1, accent.a(0.35), caret);
                    self.path = tf;
                    // 补全：光标后面淡色显示这一层剩下的部分，Tab 接受
                    if i == sel
                        && let Some((_, rest)) = self.path_ghost()
                    {
                        let gx = self.path.caret_x + 1.0;
                        let kw = g.measure("Tab", 12.0) + 12.0;
                        g.text(&rest, gx, y, (right - kw - 12.0 - gx).max(0.0), ROW, 15.0, t3, Align::Left);
                        g.fill(right - kw, cy - 11.0, kw, 22.0, 5.0, self.theme.kbd);
                        g.text("Tab", right - kw, cy - 11.0, kw, 22.0, 12.0, t2, Align::Center);
                    }
                    self.hot(60.0, y + 8.0, fw, ROW - 16.0, Act::PathField, Some(i));
                }
                SRow::Ext => {
                    label = self.s.file_types.clone();
                    // 输入框在最右，左边依次是已启用的类型和目录里实际存在的候选类型
                    let focused = i == sel;
                    left = right - 72.0;
                    g.fill(left, cy - 14.0, 72.0, 28.0, 8.0, well);
                    if focused {
                        g.outline(left - 1.0, cy - 15.0, 74.0, 30.0, 9.0, 1.5, accent);
                    }
                    if self.ext.is_empty() {
                        g.text(&self.s.add_ext, left + 10.0, cy - 14.0, 52.0, 28.0, 13.0, t3, Align::Left);
                    }
                    let caret = (focused && self.caret_on).then_some(accent);
                    let mut tf: TextField = std::mem::take(&mut self.ext);
                    g.field(&mut tf, left + 10.0, cy - 14.0, 52.0, 28.0, 13.0, t1, accent.a(0.35), caret);
                    self.ext = tf;
                    self.hot(left, cy - 14.0, 72.0, 28.0, Act::ExtField, Some(i));

                    let mut tokens: Vec<(String, usize, bool)> =
                        self.cfg.exts.iter().map(|e| (e.clone(), 0, true)).collect();
                    let mut ghosts: Vec<(String, usize)> = Vec::new();
                    for (_, ext, n) in &self.index.found {
                        if self.cfg.exts.contains(ext) {
                            continue;
                        }
                        match ghosts.iter_mut().find(|g| &g.0 == ext) {
                            Some(g) => g.1 += n,
                            None => ghosts.push((ext.clone(), *n)),
                        }
                    }
                    ghosts.sort_by(|a, b| b.1.cmp(&a.1));
                    tokens.extend(ghosts.into_iter().take(3).map(|(e, n)| (e, n, false)));
                    let floor = PAD + 14.0 + g.measure(&label, 15.0) + 16.0;
                    for (ext, n, active) in tokens.into_iter().rev() {
                        let text = format!(".{ext}");
                        let count = if active { String::new() } else { n.to_string() };
                        let (tw, cw) = (g.measure(&text, 13.0), g.measure(&count, 11.0));
                        let pw = tw + 20.0 + if active { 0.0 } else { cw + 6.0 };
                        if left - 6.0 - pw < floor {
                            continue;
                        }
                        left -= 6.0 + pw;
                        let act = Act::Token(ext, active);
                        let hovered = self.hover == act;
                        if active || hovered {
                            g.fill(left, cy - 14.0, pw, 28.0, 8.0, if active { well2 } else { well });
                        }
                        g.text(&text, left + 10.0, cy - 14.0, tw + 1.0, 28.0, 13.0, if active { t1 } else { t3 }, Align::Left);
                        g.text(&count, left + 16.0 + tw, cy - 14.0, cw + 1.0, 28.0, 11.0, t3, Align::Left);
                        self.hot(left, cy - 14.0, pw, 28.0, act, Some(i));
                    }
                }
                SRow::Hidden => {
                    label = self.s.hidden.clone();
                    label_x = 60.0;
                    label_color = t2;
                    g.fill(20.0, y + 10.0, 28.0, 28.0, 7.0, well);
                    eye_off(g, 26.0, y + 16.0, t2);
                    // 箭头随展开转向
                    let turn = self.anims.get(id, self.hidden_open as u32 as f32, 140.0);
                    let x = self.round_button(g, right, cy, Act::HiddenToggle, i, true);
                    g.rotated(turn * std::f32::consts::FRAC_PI_2, x + 16.0, cy, |g| {
                        g.path(&[(x + 14.0, cy - 4.0), (x + 18.0, cy), (x + 14.0, cy + 4.0)], false, 1.5, t2);
                    });
                    let count = self.store.hidden.len().to_string();
                    let nw = g.measure(&count, 12.0);
                    left = x - 10.0 - nw;
                    g.text(&count, left, y, nw + 1.0, ROW, 12.0, t3, Align::Left);
                }
                SRow::HiddenItem(n) => {
                    let (_, name, side) = self.store.hidden[n].clone();
                    let first = name.chars().next().unwrap_or(' ');
                    g.fill(20.0, y + 10.0, 28.0, 28.0, 7.0, Color::rgb(TOOL_TILE).a(0.55));
                    g.text(&first.to_string(), 20.0, y + 10.0, 28.0, 28.0, 13.0, Color::white(0.55), Align::Center);
                    let x = self.round_button(g, right, cy, Act::Restore(n), i, true);
                    undo(g, x + 8.0, cy - 8.0, t2);
                    let name_w = g.measure(&name, 15.0).min(x - 76.0) + 1.0;
                    g.text(&name, 60.0, y, name_w, ROW, 15.0, t2, Align::Left);
                    let sx = 60.0 + name_w + 12.0;
                    g.text(&side, sx, y, x - 10.0 - sx, ROW, 12.0, t3, Align::Right);
                }
                SRow::Status => {
                    // 呼吸的小圆点表示自动更新一直在工作
                    let phase = self.pulse.elapsed().as_secs_f32() / 2.4 * std::f32::consts::TAU;
                    let breathe = if self.anims.off { 1.0 } else { 0.675 - 0.325 * phase.cos() };
                    g.circle(34.0, cy, 4.0, accent.a(breathe));
                    label_x = 60.0;
                    label_color = t2;
                    label = if self.scanning && self.flash == self.s.updating {
                        self.s.reindexing.clone()
                    } else {
                        format!("{}{}", self.index.groups.len(), self.s.indexed)
                    };
                    let x = self.round_button(g, right, cy, Act::Refresh, i, false);
                    let spin = if self.scanning && !self.anims.off {
                        Instant::now().duration_since(self.pulse).as_secs_f32() / 0.7 * std::f32::consts::TAU
                    } else {
                        0.0
                    };
                    g.rotated(spin, x + 16.0, cy, |g| refresh(g, x + 8.0, cy - 8.0, t1));
                    left = x;
                }
            }
            if !label.is_empty() {
                g.text(&label, label_x, y, left - 16.0 - label_x, ROW, 15.0, label_color, Align::Left);
            }
        }
    }
}

/// 设置图标：两条滑杆。
fn sliders(g: &Gfx, x: f32, y: f32, c: Color) {
    g.path(&[(x + 3.0, y + 5.5), (x + 9.0, y + 5.5)], false, 1.5, c);
    g.path(&[(x + 13.0, y + 5.5), (x + 15.0, y + 5.5)], false, 1.5, c);
    g.ring(x + 11.0, y + 5.5, 2.0, 1.5, c);
    g.path(&[(x + 3.0, y + 12.5), (x + 5.0, y + 12.5)], false, 1.5, c);
    g.path(&[(x + 9.0, y + 12.5), (x + 15.0, y + 12.5)], false, 1.5, c);
    g.ring(x + 7.0, y + 12.5, 2.0, 1.5, c);
}

fn folder(g: &Gfx, x: f32, y: f32, c: Color) {
    let p = |a: f32, b: f32| (x + a, y + b);
    g.path(
        &[
            p(2.0, 4.2),
            p(3.2, 3.0),
            p(5.9, 3.0),
            p(7.5, 4.6),
            p(12.8, 4.6),
            p(14.0, 5.8),
            p(14.0, 11.8),
            p(12.8, 13.0),
            p(3.2, 13.0),
            p(2.0, 11.8),
        ],
        true,
        1.3,
        c,
    );
}

/// 划掉的眼睛：不再显示。
fn eye_off(g: &Gfx, x: f32, y: f32, c: Color) {
    let p = |a: f32, b: f32| (x + a, y + b);
    g.path(&[p(2.0, 8.0), p(4.5, 5.2), p(8.0, 4.0), p(11.5, 5.2), p(14.0, 8.0), p(11.5, 10.8), p(8.0, 12.0), p(4.5, 10.8)], true, 1.3, c);
    g.path(&[p(3.0, 13.0), p(13.0, 3.0)], false, 1.3, c);
}

/// 回转的箭头：恢复。
fn undo(g: &Gfx, x: f32, y: f32, c: Color) {
    g.arc((x + 8.0, y + 3.5), (x + 3.5, y + 8.0), 4.5, 1.5, c);
    g.path(&[(x + 5.0, y + 3.5), (x + 8.0, y + 3.5)], false, 1.5, c);
    g.path(&[(x + 7.0, y + 1.5), (x + 5.0, y + 3.5), (x + 7.0, y + 5.5)], false, 1.5, c);
}

fn refresh(g: &Gfx, x: f32, y: f32, c: Color) {
    g.arc((x + 13.0, y + 8.0), (x + 11.4, y + 4.3), 5.0, 1.5, c);
    g.path(&[(x + 13.0, y + 2.5), (x + 13.0, y + 5.0), (x + 10.5, y + 5.0)], false, 1.5, c);
}
