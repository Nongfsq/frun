//! 绘制层：Direct2D + DirectWrite。坐标一律用设计稿里的逻辑像素，缩放在这里统一处理。
use crate::{index, textfield::TextField};
use std::{collections::HashMap, fs};
use windows::{
    Win32::{
        Foundation::*,
        Graphics::{
            Direct2D::{Common::*, *},
            DirectWrite::*,
            Dxgi::Common::*,
        },
        UI::WindowsAndMessaging::GetClientRect,
    },
    core::*,
};
use windows_numerics::{Matrix3x2, Vector2};

/// 内嵌字体：Sarasa UI SC Regular 的子集（拉丁 + GB2312），SIL OFL 1.1。
static FONT: &[u8] = include_bytes!("../assets/frun-ui.woff2");
const FALLBACK_FAMILY: &str = "Microsoft YaHei UI";

#[derive(Clone, Copy, PartialEq)]
pub struct Color(pub f32, pub f32, pub f32, pub f32);

impl Color {
    pub const fn rgb(hex: u32) -> Self {
        Self(((hex >> 16) & 255) as f32 / 255.0, ((hex >> 8) & 255) as f32 / 255.0, (hex & 255) as f32 / 255.0, 1.0)
    }
    pub const fn white(a: f32) -> Self {
        Self(1.0, 1.0, 1.0, a)
    }
    pub const fn black(a: f32) -> Self {
        Self(0.0, 0.0, 0.0, a)
    }
    pub fn a(self, alpha: f32) -> Self {
        Self(self.0, self.1, self.2, self.3 * alpha)
    }
    pub fn mix(self, to: Self, t: f32) -> Self {
        let l = |a: f32, b: f32| a + (b - a) * t;
        Self(l(self.0, to.0), l(self.1, to.1), l(self.2, to.2), l(self.3, to.3))
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum Align {
    Left,
    Center,
    Right,
}

pub struct Gfx {
    d2d: ID2D1Factory,
    dw: IDWriteFactory5,
    coll: Option<IDWriteFontCollection>,
    family: HSTRING,
    /// 保持解包后的字体数据存活
    _font: Option<IDWriteFontFileStream>,
    params: Option<IDWriteRenderingParams3>,
    round: ID2D1StrokeStyle,
    rt: Option<ID2D1HwndRenderTarget>,
    brush: Option<ID2D1SolidColorBrush>,
    mark: Option<ID2D1SolidColorBrush>,
    size: (u32, u32),
    /// 画布当前绑在哪个窗口上（面板或托盘菜单）
    target: isize,
    formats: HashMap<u32, IDWriteTextFormat>,
    icons: HashMap<(u64, u32), Option<ID2D1Bitmap>>,
    /// 逻辑像素到物理像素的比例：显示器缩放 × 用户设置的大小
    pub k: f32,
    alpha: f32,
    offset: (f32, f32),
}

impl Gfx {
    pub fn new() -> Result<Self> {
        unsafe {
            let d2d: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let dw: IDWriteFactory5 = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let round = d2d.CreateStrokeStyle(
                &D2D1_STROKE_STYLE_PROPERTIES {
                    startCap: D2D1_CAP_STYLE_ROUND,
                    endCap: D2D1_CAP_STYLE_ROUND,
                    lineJoin: D2D1_LINE_JOIN_ROUND,
                    ..Default::default()
                },
                None,
            )?;
            // 字体加载失败不致命：回落到系统字体
            let (coll, family, font) = match embedded_font(&dw) {
                Ok((c, f, s)) => (Some(c), HSTRING::from(f), Some(s)),
                Err(_) => (None, HSTRING::from(FALLBACK_FAMILY), None),
            };
            // Windows 默认的文字渲染偏细且对齐到整像素，字形会变形。
            // 这里关掉网格对齐、用对称抗锯齿，并提高灰度对比度，让笔画粗细接近设计稿。
            let params = dw
                .cast::<IDWriteFactory3>()
                .and_then(|f| {
                    f.CreateCustomRenderingParams(
                        1.8,
                        0.5,
                        0.75,
                        0.0,
                        DWRITE_PIXEL_GEOMETRY_FLAT,
                        DWRITE_RENDERING_MODE1_NATURAL_SYMMETRIC,
                        DWRITE_GRID_FIT_MODE_DISABLED,
                    )
                })
                .ok();
            Ok(Self {
                d2d,
                dw,
                coll,
                family,
                _font: font,
                params,
                round,
                rt: None,
                brush: None,
                mark: None,
                size: (0, 0),
                target: 0,
                formats: HashMap::new(),
                icons: HashMap::new(),
                k: 1.0,
                alpha: 1.0,
                offset: (0.0, 0.0),
            })
        }
    }

    /// 开始一帧。窗口大小变了就调整画布，设备丢失后重建。
    pub fn begin(&mut self, hwnd: HWND) -> Result<()> {
        unsafe {
            let mut rc = RECT::default();
            let _ = GetClientRect(hwnd, &mut rc);
            let size = (rc.right.max(1) as u32, rc.bottom.max(1) as u32);
            if self.target != hwnd.0 as isize {
                self.release();
                self.target = hwnd.0 as isize;
            }
            if self.rt.is_none() {
                let rt = self.d2d.CreateHwndRenderTarget(
                    &D2D1_RENDER_TARGET_PROPERTIES {
                        // 软件光栅化：这点内容用不着显卡，而硬件设备会让常驻内存多出近百 MB
                        r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
                        pixelFormat: D2D1_PIXEL_FORMAT {
                            format: DXGI_FORMAT_B8G8R8A8_UNORM,
                            alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                        },
                        dpiX: 96.0,
                        dpiY: 96.0,
                        ..Default::default()
                    },
                    &D2D1_HWND_RENDER_TARGET_PROPERTIES {
                        hwnd,
                        pixelSize: D2D_SIZE_U { width: size.0, height: size.1 },
                        ..Default::default()
                    },
                )?;
                rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
                if let Some(p) = &self.params {
                    rt.SetTextRenderingParams(p);
                }
                let black = D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 1.0 };
                self.brush = Some(rt.CreateSolidColorBrush(&black, None)?);
                self.mark = Some(rt.CreateSolidColorBrush(&black, None)?);
                self.rt = Some(rt);
                self.size = size;
                self.icons.clear();
            } else if size != self.size {
                let rt = self.rt.as_ref().unwrap();
                rt.Resize(&D2D_SIZE_U { width: size.0, height: size.1 })?;
                self.size = size;
            }
            let rt = self.rt.as_ref().unwrap();
            rt.BeginDraw();
            rt.Clear(Some(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }));
            self.alpha = 1.0;
            self.offset = (0.0, 0.0);
            self.apply(None);
            Ok(())
        }
    }

    pub fn end(&mut self) {
        unsafe {
            let Some(rt) = &self.rt else { return };
            if rt.EndDraw(None, None).is_err() {
                // 设备丢失：下一帧重建
                self.rt = None;
                self.brush = None;
                self.mark = None;
                self.icons.clear();
            }
        }
    }

    /// 面板收起后释放画布和图标位图，常驻时只留字体和索引。
    pub fn release(&mut self) {
        self.rt = None;
        self.brush = None;
        self.mark = None;
        self.icons.clear();
    }

    /// 之后的绘制整体乘以这个不透明度、平移这个距离。
    pub fn layer(&mut self, alpha: f32, dx: f32, dy: f32) {
        self.alpha = alpha.clamp(0.0, 1.0);
        self.offset = (dx, dy);
        self.apply(None);
    }

    fn apply(&self, rotate: Option<(f32, f32, f32)>) {
        let Some(rt) = &self.rt else { return };
        let k = self.k;
        let (ox, oy) = (self.offset.0 * k, self.offset.1 * k);
        let m = match rotate {
            None => Matrix3x2 { M11: k, M12: 0.0, M21: 0.0, M22: k, M31: ox, M32: oy },
            Some((angle, cx, cy)) => {
                let (s, c) = angle.sin_cos();
                Matrix3x2 {
                    M11: c * k,
                    M12: s * k,
                    M21: -s * k,
                    M22: c * k,
                    M31: (cx - cx * c + cy * s) * k + ox,
                    M32: (cy - cx * s - cy * c) * k + oy,
                }
            }
        };
        unsafe { rt.SetTransform(&m) };
    }

    /// 绕 (cx, cy) 旋转后执行 `draw`，用于转圈的刷新图标。
    pub fn rotated(&mut self, angle: f32, cx: f32, cy: f32, draw: impl FnOnce(&mut Self)) {
        self.apply(Some((angle, cx, cy)));
        draw(self);
        self.apply(None);
    }

    fn brush(&self, c: Color) -> Option<&ID2D1SolidColorBrush> {
        let b = self.brush.as_ref()?;
        unsafe { b.SetColor(&D2D1_COLOR_F { r: c.0, g: c.1, b: c.2, a: c.3 * self.alpha }) };
        Some(b)
    }

    pub fn fill(&self, x: f32, y: f32, w: f32, h: f32, r: f32, c: Color) {
        let (Some(rt), Some(b)) = (&self.rt, self.brush(c)) else { return };
        let rect = D2D_RECT_F { left: x, top: y, right: x + w, bottom: y + h };
        unsafe {
            if r > 0.0 {
                rt.FillRoundedRectangle(&D2D1_ROUNDED_RECT { rect, radiusX: r, radiusY: r }, b);
            } else {
                rt.FillRectangle(&rect, b);
            }
        }
    }

    pub fn outline(&self, x: f32, y: f32, w: f32, h: f32, r: f32, width: f32, c: Color) {
        let (Some(rt), Some(b)) = (&self.rt, self.brush(c)) else { return };
        let rect = D2D_RECT_F { left: x, top: y, right: x + w, bottom: y + h };
        unsafe {
            rt.DrawRoundedRectangle(&D2D1_ROUNDED_RECT { rect, radiusX: r, radiusY: r }, b, width, &self.round);
        }
    }

    pub fn circle(&self, cx: f32, cy: f32, r: f32, c: Color) {
        let (Some(rt), Some(b)) = (&self.rt, self.brush(c)) else { return };
        let e = D2D1_ELLIPSE { point: Vector2 { X: cx, Y: cy }, radiusX: r, radiusY: r };
        unsafe { rt.FillEllipse(&e, b) };
    }

    pub fn ring(&self, cx: f32, cy: f32, r: f32, width: f32, c: Color) {
        let (Some(rt), Some(b)) = (&self.rt, self.brush(c)) else { return };
        let e = D2D1_ELLIPSE { point: Vector2 { X: cx, Y: cy }, radiusX: r, radiusY: r };
        unsafe { rt.DrawEllipse(&e, b, width, &self.round) };
    }

    /// 圆头折线，图标都用它画。
    pub fn path(&self, pts: &[(f32, f32)], closed: bool, width: f32, c: Color) {
        let (Some(rt), Some(b)) = (&self.rt, self.brush(c)) else { return };
        unsafe {
            if pts.len() == 2 {
                let p = |i: usize| Vector2 { X: pts[i].0, Y: pts[i].1 };
                rt.DrawLine(p(0), p(1), b, width, &self.round);
                return;
            }
            let Ok(geo) = self.d2d.CreatePathGeometry() else { return };
            let Ok(sink) = geo.Open() else { return };
            sink.BeginFigure(Vector2 { X: pts[0].0, Y: pts[0].1 }, D2D1_FIGURE_BEGIN_HOLLOW);
            for p in &pts[1..] {
                sink.AddLine(Vector2 { X: p.0, Y: p.1 });
            }
            sink.EndFigure(if closed { D2D1_FIGURE_END_CLOSED } else { D2D1_FIGURE_END_OPEN });
            let _ = sink.Close();
            rt.DrawGeometry(&geo, b, width, &self.round);
        }
    }

    /// 从 `from` 顺时针画到 `to` 的大圆弧。
    pub fn arc(&self, from: (f32, f32), to: (f32, f32), r: f32, width: f32, c: Color) {
        let (Some(rt), Some(b)) = (&self.rt, self.brush(c)) else { return };
        unsafe {
            let Ok(geo) = self.d2d.CreatePathGeometry() else { return };
            let Ok(sink) = geo.Open() else { return };
            sink.BeginFigure(Vector2 { X: from.0, Y: from.1 }, D2D1_FIGURE_BEGIN_HOLLOW);
            sink.AddArc(&D2D1_ARC_SEGMENT {
                point: Vector2 { X: to.0, Y: to.1 },
                size: D2D_SIZE_F { width: r, height: r },
                rotationAngle: 0.0,
                sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                arcSize: D2D1_ARC_SIZE_LARGE,
            });
            sink.EndFigure(D2D1_FIGURE_END_OPEN);
            let _ = sink.Close();
            rt.DrawGeometry(&geo, b, width, &self.round);
        }
    }

    pub fn clip(&self, x: f32, y: f32, w: f32, h: f32) {
        if let Some(rt) = &self.rt {
            let rect = D2D_RECT_F { left: x, top: y, right: x + w, bottom: y + h };
            unsafe { rt.PushAxisAlignedClip(&rect, D2D1_ANTIALIAS_MODE_ALIASED) };
        }
    }

    pub fn unclip(&self) {
        if let Some(rt) = &self.rt {
            unsafe { rt.PopAxisAlignedClip() };
        }
    }

    fn format(&mut self, size: f32) -> Option<IDWriteTextFormat> {
        let key = (size * 4.0) as u32;
        if let Some(f) = self.formats.get(&key) {
            return Some(f.clone());
        }
        unsafe {
            let f = self
                .dw
                .CreateTextFormat(
                    &self.family,
                    self.coll.as_ref(),
                    DWRITE_FONT_WEIGHT_REGULAR,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    size,
                    w!("zh-CN"),
                )
                .ok()?;
            let _ = f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP);
            let _ = f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
            if let Ok(sign) = self.dw.CreateEllipsisTrimmingSign(&f) {
                let trim = DWRITE_TRIMMING { granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER, ..Default::default() };
                let _ = f.SetTrimming(&trim, &sign);
            }
            self.formats.insert(key, f.clone());
            Some(f)
        }
    }

    fn layout(&mut self, text: &[u16], size: f32, w: f32, h: f32) -> Option<IDWriteTextLayout> {
        let f = self.format(size)?;
        unsafe { self.dw.CreateTextLayout(text, &f, w.max(0.0), h.max(0.0)).ok() }
    }

    pub fn measure(&mut self, s: &str, size: f32) -> f32 {
        let wide: Vec<u16> = s.encode_utf16().collect();
        let Some(layout) = self.layout(&wide, size, 100_000.0, 100.0) else { return 0.0 };
        let mut m = DWRITE_TEXT_METRICS::default();
        unsafe {
            let _ = layout.GetMetrics(&mut m);
        }
        m.widthIncludingTrailingWhitespace
    }

    /// 在给定的框里画一行文字，垂直居中，放不下时以省略号结尾。
    pub fn text(&mut self, s: &str, x: f32, y: f32, w: f32, h: f32, size: f32, c: Color, align: Align) {
        self.text_marked(s, x, y, w, h, size, c, align, &[], c);
    }

    /// 同 `text`，`marks[i]` 为真的字符用 `mark` 色（命中的字符）。
    #[allow(clippy::too_many_arguments)]
    pub fn text_marked(
        &mut self,
        s: &str,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        size: f32,
        c: Color,
        align: Align,
        marks: &[bool],
        mark: Color,
    ) {
        if w <= 0.0 || s.is_empty() {
            return;
        }
        let wide: Vec<u16> = s.encode_utf16().collect();
        let Some(layout) = self.layout(&wide, size, w, h) else { return };
        unsafe {
            let _ = layout.SetTextAlignment(match align {
                Align::Left => DWRITE_TEXT_ALIGNMENT_LEADING,
                Align::Center => DWRITE_TEXT_ALIGNMENT_CENTER,
                Align::Right => DWRITE_TEXT_ALIGNMENT_TRAILING,
            });
            if marks.iter().any(|m| *m)
                && let Some(mb) = &self.mark
            {
                mb.SetColor(&D2D1_COLOR_F { r: mark.0, g: mark.1, b: mark.2, a: mark.3 * self.alpha });
                let mut at = 0u32;
                for (ch, on) in s.chars().zip(marks) {
                    let len = ch.len_utf16() as u32;
                    if *on {
                        let _ = layout.SetDrawingEffect(mb, DWRITE_TEXT_RANGE { startPosition: at, length: len });
                    }
                    at += len;
                }
            }
            let (Some(rt), Some(b)) = (&self.rt, self.brush(c)) else { return };
            rt.DrawTextLayout(Vector2 { X: x, Y: y }, &layout, b, D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT);
        }
    }

    /// 画一个单行输入框：选区、文字、光标。光标滚出可见范围时自动跟随。
    #[allow(clippy::too_many_arguments)]
    pub fn field(
        &mut self,
        tf: &mut TextField,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        size: f32,
        c: Color,
        selection: Color,
        caret: Option<Color>,
    ) {
        let Some(layout) = self.layout(&tf.text, size, 100_000.0, h) else { return };
        let pos = |i: usize| unsafe {
            let (mut px, mut py) = (0.0, 0.0);
            let mut m = DWRITE_HIT_TEST_METRICS::default();
            let _ = layout.HitTestTextPosition(i as u32, false, &mut px, &mut py, &mut m);
            px
        };
        let cx = pos(tf.caret);
        if cx - tf.scroll > w - 2.0 {
            tf.scroll = cx - w + 2.0;
        }
        if cx - tf.scroll < 0.0 {
            tf.scroll = cx;
        }
        if tf.text.is_empty() {
            tf.scroll = 0.0;
        }
        tf.origin = x - tf.scroll;
        tf.size = size;
        self.clip(x, y, w, h);
        if let Some((a, b)) = tf.selection() {
            let (xa, xb) = (pos(a), pos(b));
            self.fill(tf.origin + xa, y + (h - size * 1.4) / 2.0, xb - xa, size * 1.4, 2.0, selection);
        }
        if let (Some(rt), Some(b)) = (&self.rt, self.brush(c)) {
            unsafe {
                rt.DrawTextLayout(Vector2 { X: tf.origin, Y: y }, &layout, b, D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT);
            }
        }
        if let Some(cc) = caret {
            self.fill(tf.origin + cx - 0.75, y + (h - size * 1.3) / 2.0, 1.5, size * 1.3, 0.75, cc);
        }
        self.unclip();
        tf.caret_x = tf.origin + cx;
    }

    /// 输入框里横坐标 `px` 对应第几个字符位置。
    pub fn field_hit(&mut self, tf: &TextField, px: f32) -> usize {
        let Some(layout) = self.layout(&tf.text, tf.size.max(1.0), 100_000.0, 100.0) else { return 0 };
        let (mut trailing, mut inside) = (BOOL(0), BOOL(0));
        let mut m = DWRITE_HIT_TEST_METRICS::default();
        unsafe {
            let _ = layout.HitTestPoint(px - tf.origin, 10.0, &mut trailing, &mut inside, &mut m);
        }
        (m.textPosition + if trailing.as_bool() { m.length } else { 0 }) as usize
    }

    /// 画图标缓存里的图标；缓存里没有时返回 false，由调用方画占位。
    pub fn icon(&mut self, key: u64, px: u32, x: f32, y: f32, size: f32, opacity: f32) -> bool {
        let Some(rt) = &self.rt else { return false };
        let bmp = self.icons.entry((key, px)).or_insert_with(|| unsafe {
            let bits = fs::read(index::icon_dir(px).join(format!("{key:016x}"))).ok()?;
            let n = ((bits.len() / 4) as f64).sqrt() as u32;
            if n == 0 || (n * n * 4) as usize != bits.len() {
                return None;
            }
            rt.CreateBitmap(
                D2D_SIZE_U { width: n, height: n },
                Some(bits.as_ptr() as _),
                n * 4,
                &D2D1_BITMAP_PROPERTIES {
                    pixelFormat: D2D1_PIXEL_FORMAT {
                        format: DXGI_FORMAT_B8G8R8A8_UNORM,
                        alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                    },
                    dpiX: 96.0,
                    dpiY: 96.0,
                },
            )
            .ok()
        });
        let Some(bmp) = bmp else { return false };
        let dest = D2D_RECT_F { left: x, top: y, right: x + size, bottom: y + size };
        unsafe {
            rt.DrawBitmap(&*bmp, Some(&dest), opacity * self.alpha, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, None);
        }
        true
    }

    /// 扫描进程写出新图标后调用：之前没找到的重新找一次。
    pub fn forget_missing_icons(&mut self) {
        self.icons.retain(|_, v| v.is_some());
    }

    pub fn forget_icons(&mut self) {
        self.icons.clear();
    }
}

/// 解开内嵌的 WOFF2，注册成只属于本进程的字体集合。用户不需要安装字体。
unsafe fn embedded_font(dw: &IDWriteFactory5) -> Result<(IDWriteFontCollection, String, IDWriteFontFileStream)> {
    unsafe {
        let stream = dw.UnpackFontFile(DWRITE_CONTAINER_TYPE_WOFF2, FONT.as_ptr() as _, FONT.len() as u32)?;
        let size = stream.GetFileSize()?;
        let (mut data, mut ctx) = (std::ptr::null_mut(), std::ptr::null_mut());
        stream.ReadFileFragment(&mut data, 0, size, &mut ctx)?;
        let loader = dw.CreateInMemoryFontFileLoader()?;
        dw.RegisterFontFileLoader(&loader)?;
        let file = loader.CreateInMemoryFontFileReference(dw, data as _, size as u32, &stream)?;
        let builder = dw.CreateFontSetBuilder()?;
        builder.AddFontFile(&file)?;
        let coll: IDWriteFontCollection = dw.CreateFontCollectionFromFontSet(&builder.CreateFontSet()?)?.cast()?;
        let names = coll.GetFontFamily(0)?.GetFamilyNames()?;
        let mut buf = [0u16; 128];
        names.GetString(0, &mut buf)?;
        let len = buf.iter().position(|&c| c == 0).unwrap_or(0);
        Ok((coll, String::from_utf16_lossy(&buf[..len]), stream))
    }
}
